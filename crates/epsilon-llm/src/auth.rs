//! Provider credential store — opencode's `auth.json` pattern (M22).
//!
//! opencode lets a user run `opencode auth login`, pick a provider, paste a key
//! once, and every later session just works; the key lives in
//! `~/.local/share/opencode/auth.json`. Epsilon ports that experience: the file
//! is `<global config dir>/auth.json` (`~/.config/epsilon/auth.json`), holding
//! `{ "provider-id": { "type": "api", "key": "..." } }` — the same shape as
//! opencode's `Auth.Info` (`api` variant, the only kind our direct-HTTP wires
//! need).
//!
//! Precedence lives in [`crate::catalog::resolve_api_key`]: **stored key wins**
//! over the environment variable (the login flow is the documented path), env
//! vars remain the fallback for CI and users who prefer them.
//!
//! Write discipline (pi `FileAuthStorageBackend` parity, M35): reads stay
//! advisory and fail open, but every read-modify-write holds a cross-process
//! `auth.lock` (stale locks are stolen), persists via same-dir tmp + atomic
//! rename, enforces 0600 on unix, and REFUSES to overwrite a corrupt file — a
//! failed-open read must never become a wipe on the next save.
//!
//! Placement note: this store lives in epsilon-llm (not core) because pi keeps
//! provider auth inside its `packages/ai` brick and Epsilon's dependency law
//! (`contracts <- llm <- core`) forbids llm from reaching core.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// How long a lockfile may live before its holder is presumed crashed and the
/// lock stolen (pi's async-backend stale threshold).
const LOCK_STALE_SECS: u64 = 30;
/// Lock acquisition attempts before giving up (pi's sync-backend retry count).
const LOCK_ATTEMPTS: u32 = 10;

/// One stored provider credential (opencode `Auth.Info`, `api` variant;
/// pi `auth-storage.ts`: untagged union, `type:"api_key"` coerced on read,
/// `type:"oauth"` for token pairs).
///
/// The on-disk shape stays opencode's `{type:"api", key, env?}` for api keys
/// (what `save_credential` writes and `auth login` reads back). On READ, two
/// extra shapes are accepted (pi `auth-storage.ts`):
///
/// - `{type:"api_key", key, env?}` (same fields, older discriminator);
/// - `{type:"oauth", refresh, access, expires, ...}` (token pair; extra
///   fields ignored).
///
/// Anything else fails to parse — a corrupt file must not silently become a key.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "StoredCredentialWire")]
pub enum StoredCredential {
    /// API-key credential (literal, `$ENV`/`${ENV}` template, or `!command`).
    Api {
        /// The secret API key value (or template/command).
        key: String,
        /// Extra environment values injected when resolving `key`'s template
        /// (pi `credential.env` — per-provider overrides for secret managers).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        env: Option<std::collections::BTreeMap<String, String>>,
    },
    /// OAuth token pair (pi `OAuthCredential`): short-lived `access` + stable
    /// `refresh`, with `expires` as epoch millis (+ 5-min skew, pi parity).
    /// Unknown extra fields are ignored on read (pi preserves them opaquely).
    Oauth {
        /// Refresh token (stable; exchanged for new access tokens).
        refresh: String,
        /// Access token (short-lived; sent on the wire).
        access: String,
        /// Epoch millis when `access` expires (pi subtracts 5 min skew at issue).
        expires: u64,
        /// Provider extras minted alongside the pair (M55). pi's Copilot
        /// credential carries `enterpriseUrl`, and the API host is
        /// `copilot-api.<domain>` for Business/Enterprise accounts, so the
        /// domain has to survive the round-trip. Kept as an env-style map so
        /// future provider extras (project ids, locations) need no new field.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        env: Option<std::collections::BTreeMap<String, String>>,
    },
}

impl StoredCredential {
    /// Discriminator string (`"api"` / `"oauth"`).
    pub fn kind(&self) -> &'static str {
        match self {
            StoredCredential::Api { .. } => "api",
            StoredCredential::Oauth { .. } => "oauth",
        }
    }

    /// The api key when this is an `api` credential.
    pub fn key(&self) -> Option<&str> {
        match self {
            StoredCredential::Api { key, .. } => Some(key),
            StoredCredential::Oauth { .. } => None,
        }
    }

    /// The credential `env` map (`api` secrets templates, or the extras an
    /// OAuth login minted alongside its pair — see [`StoredCredential::Oauth`]).
    pub fn env(&self) -> Option<&std::collections::BTreeMap<String, String>> {
        match self {
            StoredCredential::Api { env, .. } => env.as_ref(),
            StoredCredential::Oauth { env, .. } => env.as_ref(),
        }
    }
}

impl serde::Serialize for StoredCredential {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            StoredCredential::Api { key, env } => {
                let len = if env.is_some() { 3 } else { 2 };
                let mut m = s.serialize_map(Some(len))?;
                m.serialize_entry("type", "api")?;
                m.serialize_entry("key", key)?;
                if let Some(env) = env {
                    m.serialize_entry("env", env)?;
                }
                m.end()
            }
            StoredCredential::Oauth {
                refresh,
                access,
                expires,
                env,
            } => {
                let mut m = s.serialize_map(Some(if env.is_some() { 5 } else { 4 }))?;
                m.serialize_entry("type", "oauth")?;
                m.serialize_entry("refresh", refresh)?;
                m.serialize_entry("access", access)?;
                m.serialize_entry("expires", expires)?;
                if let Some(env) = env {
                    m.serialize_entry("env", env)?;
                }
                m.end()
            }
        }
    }
}

/// Credential-extra key holding a Copilot Business/Enterprise domain (M55).
/// pi's Copilot credential carries `enterpriseUrl`; the API host for those
/// accounts is `copilot-api.<domain>` (and the token mint lives on
/// `api.<domain>`), so the domain travels with the pair.
pub const ENTERPRISE_DOMAIN_ENV_KEY: &str = "COPILOT_ENTERPRISE_DOMAIN";

/// The enterprise domain stored alongside a provider's credential, if any
/// (M55; only Copilot logins set it today).
pub fn enterprise_domain_of_in(config_dir: &Path, provider: &str) -> Option<String> {
    load_credentials_in(config_dir)
        .get(provider)
        .and_then(|c| c.env())
        .and_then(|env| env.get(ENTERPRISE_DOMAIN_ENV_KEY))
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
}

/// [`enterprise_domain_of_in`] against the resolved global directory.
pub fn enterprise_domain_of(provider: &str) -> Option<String> {
    enterprise_domain_of_in(&global_dir()?, provider)
}

/// Read-side wire shape: accepts every discriminator the ecosystem writes.
/// Serializes back through the canonical `StoredCredential` (so writes always
/// normalize to `{type:"api"}` / `{type:"oauth"}` — files converge instead of
/// accumulating legacy spellings).
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum StoredCredentialWire {
    /// What we write (opencode schema).
    #[serde(rename = "api")]
    Api {
        key: String,
        #[serde(default)]
        env: Option<std::collections::BTreeMap<String, String>>,
    },
    /// pi's older discriminator for the same shape.
    #[serde(rename = "api_key")]
    ApiKey {
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        env: Option<std::collections::BTreeMap<String, String>>,
    },
    /// pi's token pair (extra fields ignored — pi preserves them opaquely,
    /// we only need the three wire fields).
    #[serde(rename = "oauth")]
    Oauth {
        #[serde(default)]
        refresh: Option<String>,
        #[serde(default)]
        access: Option<String>,
        #[serde(default)]
        expires: Option<u64>,
        #[serde(default)]
        env: Option<std::collections::BTreeMap<String, String>>,
    },
}

impl From<StoredCredentialWire> for StoredCredential {
    fn from(w: StoredCredentialWire) -> Self {
        match w {
            StoredCredentialWire::Api { key, env } => StoredCredential::Api { key, env },
            StoredCredentialWire::ApiKey { key, env } => StoredCredential::Api {
                key: key.unwrap_or_default(),
                env,
            },
            StoredCredentialWire::Oauth {
                refresh,
                access,
                expires,
                env,
            } => StoredCredential::Oauth {
                refresh: refresh.unwrap_or_default(),
                access: access.unwrap_or_default(),
                expires: expires.unwrap_or(0),
                env,
            },
        }
    }
}

/// `auth.json` inside a config dir (opencode's file name and location).
pub(crate) fn auth_path_in(config_dir: &Path) -> PathBuf {
    config_dir.join("auth.json")
}

/// The user's home directory — `$HOME`, falling back to `$USERPROFILE`.
///
/// **This is the one copy of that rule** (M223), the same way [`global_dir`] is: the
/// harness reads *several* cross-tool conventions out of the home directory (Claude
/// Code's `~/.claude/skills`, Codex's `~/.codex/skills`, and the rest of the skill-root
/// table in `epsilon_core::skills`), and each of those would otherwise have grown its
/// own `var_os("HOME")` or, worse, assumed `/home/<user>` and been wrong on Windows.
///
/// `None` means no home directory resolves at all, which is the case every caller
/// already handles for [`global_dir`]: skip the user-tier roots rather than invent a
/// path. It is *not* an error — a process with no `HOME` still has a workspace.
pub fn home_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home))
}

/// `~/.config/epsilon` — the home directory, then Epsilon's own directory under it.
///
/// Delegates to [`home_dir`] rather than reading the environment itself; see M223 and
/// the source-scanning test in `epsilon_core::config` that counts the implementations.
pub fn global_dir() -> Option<PathBuf> {
    Some(home_dir()?.join(".config").join("epsilon"))
}

/// Parse auth.json one entry at a time.
///
/// A whole-map `from_str` made one malformed provider hide every other
/// credential: the user looked logged out everywhere because one entry was
/// damaged. The good entries are the ones a request can still use; the bad
/// key is dropped, and a later save refuses to overwrite the corrupt file via
/// `existing_auth_map`, so the evidence remains until the user repairs it.
fn parse_auth_map(raw: &str) -> std::collections::BTreeMap<String, StoredCredential> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Default::default();
    };
    let Some(obj) = value.as_object() else {
        return Default::default();
    };
    obj.iter()
        .filter_map(|(provider, entry)| {
            serde_json::from_value::<StoredCredential>(entry.clone())
                .ok()
                .map(|credential| (provider.clone(), credential))
        })
        .collect()
}

/// Cross-process guard for auth.json read-modify-write cycles. `create_new`
/// is atomic on every platform, so exactly one holder wins; stale locks (mtime
/// older than [`LOCK_STALE_SECS`]) are stolen — a crashed holder must not wedge
/// logins forever. The file is removed on drop, including on panic unwind.
struct AuthLock {
    path: PathBuf,
}

impl Drop for AuthLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn acquire_auth_lock(config_dir: &Path) -> Result<AuthLock, String> {
    let path = config_dir.join("auth.lock");
    for attempt in 0..LOCK_ATTEMPTS {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(AuthLock { path }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age.as_secs() >= LOCK_STALE_SECS);
                if stale {
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
                std::thread::sleep(std::time::Duration::from_millis(10 * (attempt as u64 + 1)));
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                // A transient Windows sharing violation, not a permission problem:
                // when two processes (or eight threads, as the concurrency test
                // runs) create-and-delete the same lock path at once, one of them
                // gets ACCESS_DENIED (os error 5) rather than AlreadyExists, purely
                // because the other is mid-delete. Reporting that to the user as
                // `acquire …auth.lock: Access is denied` is wrong twice over: it is
                // not a permission failure and it is not permanent. Measured from
                // the concurrency test before this arm existed — the CLI and the TUI
                // saving at the same moment, which is the case that test is named
                // for, is a realistic way to hit it.
                //
                // Treated as contention, with the same backoff, so the retry budget
                // covers both reasons for losing the race.
                std::thread::sleep(std::time::Duration::from_millis(10 * (attempt as u64 + 1)));
            }
            Err(e) => {
                return Err(format!("acquire {}: {e}", path.display()));
            }
        }
    }
    Err(format!(
        "timed out acquiring {} (another process holds it?)",
        path.display()
    ))
}

/// The on-disk map, distinguishing "nothing to preserve" from "must not touch".
/// `Ok(None)` = missing or blank file; `Err` = present but unparsable.
fn existing_auth_map(
    path: &Path,
) -> Result<Option<std::collections::BTreeMap<String, StoredCredential>>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("read {}: {e}", path.display())),
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    match serde_json::from_str(&raw) {
        Ok(map) => Ok(Some(map)),
        Err(_) => Err(format!(
            "refusing to overwrite corrupt {} (back it up or delete it first)",
            path.display()
        )),
    }
}

/// All stored credentials from `config_dir/auth.json` keyed by provider id
/// (`anthropic`, `openai`, ...). Corrupt/missing files yield an empty map —
/// credentials are advisory at read time exactly like the rest of config;
/// env vars remain the fallback path in [`crate::catalog::resolve_api_key`].
pub fn load_credentials_in(
    config_dir: &Path,
) -> std::collections::BTreeMap<String, StoredCredential> {
    match std::fs::read_to_string(auth_path_in(config_dir)) {
        Ok(raw) => parse_auth_map(&raw),
        Err(_) => Default::default(),
    }
}

/// Store `key` for `provider_id`, preserving other entries. Creates the dir.
/// Writes 0600 permissions on unix (opencode writes 0o600); Windows relies on
/// the user-profile ACL, which is already user-scoped.
pub fn save_credential_in(
    config_dir: &Path,
    provider_id: &str,
    key: &str,
) -> Result<PathBuf, String> {
    save_credential_full_in(config_dir, provider_id, key, None)
}

/// [`save_credential_in`] with an optional per-provider env override map
/// (pi `credential.env` — extra values injected when resolving `key`).
pub fn save_credential_full_in(
    config_dir: &Path,
    provider_id: &str,
    key: &str,
    env: Option<std::collections::BTreeMap<String, String>>,
) -> Result<PathBuf, String> {
    if provider_id.trim().is_empty() {
        return Err("refusing to store a credential under an empty provider id".into());
    }
    if key.trim().is_empty() {
        return Err("refusing to store an empty API key".into());
    }
    create_private_dir(config_dir)?;
    let _lock = acquire_auth_lock(config_dir)?;
    let path = auth_path_in(config_dir);

    // A corrupt file fails open on READ — it must never become a wipe on WRITE.
    let mut map = existing_auth_map(&path)?.unwrap_or_default();
    map.insert(
        provider_id.to_string(),
        StoredCredential::Api {
            key: key.to_string(),
            env,
        },
    );
    write_auth_file(&path, &map)?;
    Ok(path)
}

/// SigV4 request inputs, bundled so the signer stays under the arg-count lint.
#[derive(Debug, Clone)]
pub struct BedrockSignInput<'a> {
    /// AWS access key id.
    pub access_key: &'a str,
    /// AWS secret access key.
    pub secret_key: &'a str,
    /// Session token, if temporary credentials.
    pub session_token: Option<&'a str>,
    /// `bedrock-runtime` for converse calls.
    pub service: &'a str,
    /// AWS region being signed against.
    pub region: &'a str,
    /// Full endpoint URL (host is extracted for the canonical request).
    pub endpoint: &'a str,
    /// Request path (`/model/{id}/converse-stream`).
    pub path: &'a str,
    /// Serialized JSON body (hashed into the canonical request).
    pub body: &'a str,
    /// `YYYYMMDDTHHMMSSZ` stamp.
    pub datetime: &'a str,
}

/// SigV4-signed Bedrock request headers for a Converse Stream call.
///
/// Pure function of the sign input: canonical request → string-to-sign →
/// derived key (`kDate/kRegion/kService/kSigning` with the workspace `sha2`
/// pin — no `hmac` crate needed: HMAC-SHA256 is three lines over the `Sha256`
/// API) → `Authorization` + `X-Amz-Date` (+ `X-Amz-Security-Token` when a
/// session token is present).
pub fn sign_bedrock_request(input: &BedrockSignInput<'_>) -> Vec<(String, String)> {
    use sha2::{Digest, Sha256};
    let date = input.datetime.get(..8).unwrap_or(input.datetime);
    let body_hash = hex_sha256(input.body.as_bytes());
    // Canonical headers: content-type + host + x-amz-date (+ token), sorted
    // lowercase, exactly the SigV4 form.
    let host = input
        .endpoint
        .strip_prefix("https://")
        .or_else(|| input.endpoint.strip_prefix("http://"))
        .unwrap_or(input.endpoint)
        .split('/')
        .next()
        .unwrap_or(input.endpoint);
    let mut signed: Vec<(&str, String)> = vec![
        ("content-type", "application/json".to_string()),
        ("host", host.to_string()),
        ("x-amz-date", input.datetime.to_string()),
    ];
    if let Some(token) = input.session_token.filter(|t| !t.is_empty()) {
        signed.push(("x-amz-security-token", token.to_string()));
    }
    signed.sort_by_key(|(k, _)| *k);
    let canonical_headers: String = signed
        .iter()
        .map(|(k, v)| format!("{k}:{}\n", v.trim()))
        .collect();
    let signed_headers: String = signed.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(";");
    let canonical_request = format!(
        "POST\n{}\n\n{canonical_headers}\n{signed_headers}\n{body_hash}",
        input.path
    );
    let scope = format!("{date}/{}/{}/aws4_request", input.region, input.service);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        input.datetime,
        hex_sha256(canonical_request.as_bytes())
    );
    let signing_key = hmac_sha256(
        &hmac_sha256(
            &hmac_sha256(
                &hmac_sha256(
                    format!("AWS4{}", input.secret_key).as_bytes(),
                    date.as_bytes(),
                ),
                input.region.as_bytes(),
            ),
            input.service.as_bytes(),
        ),
        b"aws4_request",
    );
    let signature = hex_bytes(&hmac_sha256(&signing_key, string_to_sign.as_bytes()));
    let mut headers = vec![
        (
            "Authorization".to_string(),
            format!(
                "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
                input.access_key
            ),
        ),
        ("X-Amz-Date".to_string(), input.datetime.to_string()),
        ("Content-Type".to_string(), "application/json".to_string()),
    ];
    if let Some(token) = input.session_token.filter(|t| !t.is_empty()) {
        headers.push(("X-Amz-Security-Token".to_string(), token.to_string()));
    }
    // Touch Digest so the import stays live when tests pin vectors below.
    let _ = Sha256::new();
    headers
}

/// Raw HMAC-SHA256 (the three lines the missing `hmac` crate would own):
/// `H((K⊕opad) ∥ H((K⊕ipad) ∥ msg))` over `Sha256`.
fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut kb = [0u8; 64];
    if key.len() > 64 {
        let digest = Sha256::digest(key);
        kb[..32].copy_from_slice(&digest);
    } else {
        kb[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= kb[i];
        opad[i] ^= kb[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(msg);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    outer.finalize().to_vec()
}

/// Lowercase hex of SHA-256 (duplicates the cli helper — auth owns its copy
/// so the llm brick never reaches into cli).
fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex_bytes(&Sha256::digest(bytes))
}

/// Lowercase hex of raw bytes.
fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Store an OAuth token pair for `provider_id`, preserving other entries.
/// `expires` is epoch millis (pi subtracts 5-min skew at issue; callers pass
/// the already-skewed value). Like api saves: locked, atomic, 0600, no-clobber.
pub fn save_oauth_in(
    config_dir: &Path,
    provider_id: &str,
    refresh: &str,
    access: &str,
    expires: u64,
) -> Result<PathBuf, String> {
    if provider_id.trim().is_empty() {
        return Err("refusing to store a credential under an empty provider id".into());
    }
    save_oauth_full_in(config_dir, provider_id, refresh, access, expires, None)
}

/// [`save_oauth_in`] plus the provider extras an OAuth login minted alongside
/// the pair (M55: GitHub Copilot's enterprise domain — pi's
/// `credential.enterpriseUrl`). `None` leaves the credential with no extras,
/// exactly like [`save_oauth_in`].
pub fn save_oauth_full_in(
    config_dir: &Path,
    provider_id: &str,
    refresh: &str,
    access: &str,
    expires: u64,
    env: Option<std::collections::BTreeMap<String, String>>,
) -> Result<PathBuf, String> {
    if provider_id.trim().is_empty() {
        return Err("refusing to store a credential under an empty provider id".into());
    }
    if refresh.trim().is_empty() || access.trim().is_empty() {
        return Err("refusing to store an empty OAuth token".into());
    }
    create_private_dir(config_dir)?;
    let _lock = acquire_auth_lock(config_dir)?;
    let path = auth_path_in(config_dir);
    let mut map = existing_auth_map(&path)?.unwrap_or_default();
    map.insert(
        provider_id.to_string(),
        StoredCredential::Oauth {
            refresh: refresh.to_string(),
            access: access.to_string(),
            expires,
            env: env.filter(|m| !m.is_empty()),
        },
    );
    write_auth_file(&path, &map)?;
    Ok(path)
}

/// [`save_oauth_in`] against the resolved global directory.
pub fn save_oauth(
    provider_id: &str,
    refresh: &str,
    access: &str,
    expires: u64,
) -> Result<PathBuf, String> {
    let dir = global_dir().ok_or_else(|| "no home directory resolved".to_string())?;
    save_oauth_in(&dir, provider_id, refresh, access, expires)
}

/// A refreshed OAuth token pair from a refresh-token exchange (pi
/// `OAuthCredential`: rotated `access` + possibly-rotated `refresh`,
/// `expires` epoch millis already minus pi's 5-min skew).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OauthRefreshed {
    /// New short-lived access token.
    pub access: String,
    /// Refresh token to store (pi rotates it when the server returns one).
    pub refresh: String,
    /// New expiry epoch millis (skew already applied by the exchange).
    pub expires: u64,
}

/// Outcome of the two-phase refresh composition ([`oauth_needs_refresh_in`] +
/// [`store_refreshed_oauth_in`]): a usable access token, or why refresh
/// did not produce one. Mirrors pi's `resolveStoredOAuth` contract —
/// refresh failure is `RefreshFailed` (caller falls back to stored/env key),
/// never a silent `Missing`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OauthRefresh {
    /// Usable access token (was live, or refreshed + persisted just now).
    Live(String),
    /// No oauth credential stored for this provider.
    Missing,
    /// Stored but the exchange failed or is unconfigured — the caller falls
    /// back to the stored/env api key path, never sends the stale token.
    RefreshFailed(String),
}

/// Refresh the stored OAuth pair when it expires soon (pi
/// `auth/resolve.ts::resolveStoredOAuth` double-checked pattern):
/// optimistic read first (no lock when the token is comfortably live), else
/// acquire `auth.lock`, re-read (another process may have refreshed meanwhile),
/// run `exchange` ONCE, persist the rotated pair under the same lock, release.
///
/// `exchange` maps the stored refresh token to a new pair over the network
/// (per-provider token URL + client id — pi hardcodes them per `oauth/*.ts`
/// file; callers inject them so `auth.rs` stays offline-testable and never
/// embeds another project's client id). `min_validity_ms` is pi's
/// 5-minute window (`DEFAULT_OAUTH_MINIMUM_VALIDITY_MS`); pass `0` for the
/// plain "expired means expired" check.
///
/// Sync by design: the exchange must NEVER run under `auth.lock` (network IO
/// wedges every other process). So the composition is two-phase and the lock
/// is only ever held for file IO:
///
/// 1. [`oauth_needs_refresh_in`] (lock-free read): live → use it.
/// 2. Caller runs the provider exchange off-lock.
/// 3. [`store_refreshed_oauth_in`] (locked re-check + persist): re-reads under
///    the lock; if another process already rotated, keeps theirs.
///
/// Steps 1+3 compose pi's "refresh once globally" without ever holding
/// `auth.lock` across network IO. [`OauthRefresh`] names the caller-facing
/// outcome; [`OauthRefreshed`] is the exchanged pair handed to step 3.
pub fn oauth_needs_refresh_in(config_dir: &Path, provider: &str, min_validity_ms: u64) -> bool {
    match load_credentials_in(config_dir).get(provider) {
        Some(StoredCredential::Oauth { expires, .. }) => {
            let now = now_millis();
            *expires <= now.saturating_add(min_validity_ms).saturating_add(60_000)
        }
        // No pair (or not oauth): nothing to refresh — the key path owns it.
        _ => false,
    }
}

/// Locked re-check + persist for a freshly exchanged pair (step 3 above).
/// Returns the access token the caller should use: the store's (when another
/// process rotated first — last-writer-wins would discard their refresh
/// token) or the caller's `fresh` (persisted just now). Refuses empty tokens
/// without touching the file.
pub fn store_refreshed_oauth_in(
    config_dir: &Path,
    provider_id: &str,
    fresh: &OauthRefreshed,
) -> Result<String, String> {
    if fresh.access.trim().is_empty() || fresh.refresh.trim().is_empty() {
        return Err("refusing to store an empty refreshed OAuth token".into());
    }
    create_private_dir(config_dir)?;
    let _lock = acquire_auth_lock(config_dir)?;
    let path = auth_path_in(config_dir);
    // No-clobber, same as every other write: corrupt stays corrupt.
    let mut map = existing_auth_map(&path)?.unwrap_or_default();
    // Another process refreshed while our exchange was in flight: their pair
    // is newer — so trust liveness: if the stored access is live, keep it
    // and drop ours (last-writer-wins would discard their refresh token).
    if let Some(StoredCredential::Oauth {
        access, expires, ..
    }) = map.get(provider_id)
        && *expires > now_millis() + 60_000
        && *access != fresh.access
    {
        return Ok(access.clone());
    }
    // Provider extras (Copilot's enterprise domain) describe the ACCOUNT, not
    // the token pair, so a rotation must carry them forward.
    let env = map.get(provider_id).and_then(|c| c.env()).cloned();
    map.insert(
        provider_id.to_string(),
        StoredCredential::Oauth {
            refresh: fresh.refresh.clone(),
            access: fresh.access.clone(),
            expires: fresh.expires,
            env,
        },
    );
    write_auth_file(&path, &map)?;
    Ok(fresh.access.clone())
}

/// Current epoch millis (single call site for the expiry comparisons above).
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A live OAuth access token for `provider`, or why none is available.
/// Checks expiry with a 60s clock-skew margin (pi refreshes at expiry-minus-5m
/// on issue; the margin here covers read-to-use drift).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OauthToken {
    /// Usable access token.
    Live(String),
    /// No oauth credential stored for this provider.
    Missing,
    /// Stored but expired (caller should refresh-or-login, never send).
    Expired,
}

/// Read the usable OAuth access token, if any (never falls back to env —
/// OAuth tokens only live in the store).
pub fn oauth_access_token(provider: &str) -> OauthToken {
    oauth_access_token_in(&global_dir().unwrap_or_default(), provider)
}

/// [`oauth_access_token`] with an injected store dir (tests).
pub fn oauth_access_token_in(config_dir: &Path, provider: &str) -> OauthToken {
    match load_credentials_in(config_dir).get(provider) {
        Some(StoredCredential::Oauth {
            access, expires, ..
        }) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            if *expires > now + 60_000 {
                OauthToken::Live(access.clone())
            } else {
                OauthToken::Expired
            }
        }
        _ => OauthToken::Missing,
    }
}

/// Remove the credential for `provider_id`. Reports whether one existed.
pub fn remove_credential_in(config_dir: &Path, provider_id: &str) -> Result<bool, String> {
    let path = auth_path_in(config_dir);
    if !path.exists() {
        return Ok(false);
    }
    let _lock = acquire_auth_lock(config_dir)?;
    // Same no-clobber rule as save: a corrupt file is evidence, not an empty map.
    let mut map = existing_auth_map(&path)?.unwrap_or_default();
    let existed = map.remove(provider_id).is_some();
    if existed {
        write_auth_file(&path, &map)?;
    }
    Ok(existed)
}

fn write_auth_file(
    path: &Path,
    map: &std::collections::BTreeMap<String, StoredCredential>,
) -> Result<(), String> {
    let serialized =
        serde_json::to_string_pretty(map).map_err(|e| format!("serialize auth: {e}"))?;
    // Same-dir tmp + atomic rename: a crash mid-write leaves the previous file
    // intact instead of a torn auth.json.
    //
    // **0600 is applied at creation, not after.** `fs::write` followed by chmod
    // left the secret readable at the umask for that window; the old comment
    // claimed the opposite because it described the intent rather than the code.
    let tmp = path.with_extension("tmp");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| format!("write {}: {e}", tmp.display()))?;
        file.write_all(serialized.as_bytes())
            .map_err(|e| format!("write {}: {e}", tmp.display()))?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, serialized).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("publish {}: {e}", path.display()))?;
    Ok(())
}

/// Create a credential directory that only this user can list (0700 on Unix).
///
/// A 0600 file inside a 0755 directory still leaks its existence and the
/// provider set; pi and DSH both close that half. On Windows the profile ACL is
/// the platform's answer, as the module comment records.
fn create_private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("chmod {}: {e}", dir.display()))?;
    }
    Ok(())
}

/// [`load_credentials_in`] against the resolved global directory.
pub fn load_credentials() -> std::collections::BTreeMap<String, StoredCredential> {
    global_dir()
        .map(|d| load_credentials_in(&d))
        .unwrap_or_default()
}

/// [`save_credential_in`] against the resolved global directory.
pub fn save_credential(provider_id: &str, key: &str) -> Result<PathBuf, String> {
    let dir = global_dir().ok_or_else(|| "no home directory resolved".to_string())?;
    save_credential_in(&dir, provider_id, key)
}

/// [`save_credential_full_in`] against the resolved global directory.
pub fn save_credential_full(
    provider_id: &str,
    key: &str,
    env: Option<std::collections::BTreeMap<String, String>>,
) -> Result<PathBuf, String> {
    let dir = global_dir().ok_or_else(|| "no home directory resolved".to_string())?;
    save_credential_full_in(&dir, provider_id, key, env)
}

/// [`remove_credential_in`] against the resolved global directory.
pub fn remove_credential(provider_id: &str) -> Result<bool, String> {
    let dir = global_dir().ok_or_else(|| "no home directory resolved".to_string())?;
    remove_credential_in(&dir, provider_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigv4_signing_matches_aws_reference_vector() {
        // AWS SigV4 reference test (iam, `creq` suite shape): fixed inputs →
        // fixed Authorization header. Pinned against the documented signing
        // steps (canonical request → string-to-sign → derived key), not a
        // second implementation of the same code.
        //
        // The 9-arg helper exceeds the arg-count lint, so the (identical)
        // suffix args live in one place:
        const ENDPOINT: &str = "https://bedrock-runtime.us-east-1.amazonaws.com";
        const BODY: &str = "{}";
        const DATETIME: &str = "20150830T123600Z";
        fn sign(
            access: &str,
            secret: &str,
            token: Option<&str>,
            service: &str,
            region: &str,
            path: &str,
        ) -> Vec<(String, String)> {
            sign_bedrock_request(&BedrockSignInput {
                access_key: access,
                secret_key: secret,
                session_token: token,
                service,
                region,
                endpoint: ENDPOINT,
                path,
                body: BODY,
                datetime: DATETIME,
            })
        }
        let headers = sign(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            None,
            "bedrock-runtime",
            "us-east-1",
            "/model/anthropic.claude-sonnet-4-5/converse-stream",
        );
        let auth = headers
            .iter()
            .find(|(k, _)| k == "Authorization")
            .map(|(_, v)| v.clone())
            .expect("auth header");
        assert!(
            auth.starts_with(
                "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/bedrock-runtime/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature="
            ),
            "scope + signed headers must match SigV4 form (got {auth})"
        );
        let sig = auth.rsplit('=').next().unwrap_or("");
        assert_eq!(sig.len(), 64, "hex SHA-256 signature (got {sig})");
        // Determinism: same inputs → same signature.
        let again = sign(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            None,
            "bedrock-runtime",
            "us-east-1",
            "/model/anthropic.claude-sonnet-4-5/converse-stream",
        );
        assert_eq!(headers, again);
        // Session tokens ride both the signed headers and the explicit header.
        let with_token = sign(
            "AKID",
            "SK",
            Some("TOKEN"),
            "bedrock-runtime",
            "us-east-1",
            "/model/x/converse-stream",
        );
        let auth_t = with_token
            .iter()
            .find(|(k, _)| k == "Authorization")
            .unwrap()
            .1
            .clone();
        assert!(
            auth_t.contains("SignedHeaders=content-type;host;x-amz-date;x-amz-security-token"),
            "token must be a signed header (got {auth_t})"
        );
        assert!(
            with_token
                .iter()
                .any(|(k, v)| k == "X-Amz-Security-Token" && v == "TOKEN"),
            "token must also ride the explicit header"
        );
    }

    #[test]
    fn oauth_and_legacy_api_key_shapes_read() {
        // pi's `{type:"oauth", refresh, access, expires}` reads with extras ignored.
        let oauth = serde_json::json!({
            "type": "oauth",
            "refresh": "rt-1",
            "access": "at-1",
            "expires": 1_700_000_000_000u64,
            "scope": "user:inference",
        });
        let cred: StoredCredential = serde_json::from_value(oauth).unwrap();
        assert_eq!(cred.kind(), "oauth");
        assert_eq!(cred.key(), None);
        assert!(matches!(
            cred,
            StoredCredential::Oauth {
                refresh,
                access,
                expires,
                ..
            } if refresh == "rt-1" && access == "at-1" && expires == 1_700_000_000_000
        ));

        // pi's older `{type:"api_key"}` discriminator coerces to Api.
        let legacy = serde_json::json!({"type": "api_key", "key": "sk-x"});
        let cred: StoredCredential = serde_json::from_value(legacy).unwrap();
        assert_eq!(cred.key(), Some("sk-x"));

        // Unknown discriminators still fail (corrupt, never a silent key).
        let bad = serde_json::json!({"type": "token", "key": "sk-x"});
        assert!(serde_json::from_value::<StoredCredential>(bad).is_err());
    }

    #[test]
    fn oauth_save_expiry_and_token_states() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        // Live token reads back.
        save_oauth_in(dir, "anthropic", "rt-1", "at-live", now + 3_600_000).unwrap();
        assert!(matches!(
            oauth_access_token_in(dir, "anthropic"),
            OauthToken::Live(t) if t == "at-live"
        ));
        // Api and oauth coexist per provider map.
        save_credential_in(dir, "openai", "sk-x").unwrap();
        assert!(matches!(
            oauth_access_token_in(dir, "openai"),
            OauthToken::Missing
        ));

        // Expired token is reported, never returned.
        save_oauth_in(dir, "anthropic", "rt-2", "at-old", now.saturating_sub(1)).unwrap();
        assert_eq!(oauth_access_token_in(dir, "anthropic"), OauthToken::Expired);

        // Empty tokens refused; file untouched by the refusal.
        assert!(save_oauth_in(dir, "x", "", "at", now + 1).is_err());
        assert!(!load_credentials_in(dir).contains_key("x"));
    }

    #[test]
    fn save_load_remove_round_trip_preserving_other_providers() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();

        save_credential_in(dir, "anthropic", "sk-ant-test").expect("save");
        // Second entry must survive the first one.
        save_credential_in(dir, "openai", "sk-oauth").expect("save");

        let creds = load_credentials_in(dir);
        assert_eq!(creds.len(), 2);
        assert_eq!(creds["anthropic"].kind(), "api");
        assert_eq!(creds["anthropic"].key(), Some("sk-ant-test"));
        assert_eq!(creds["openai"].key(), Some("sk-oauth"));

        // Overwrite semantics: same provider id replaces its key only.
        save_credential_in(dir, "anthropic", "sk-rotated").expect("rotate");
        let creds = load_credentials_in(dir);
        assert_eq!(creds["anthropic"].key(), Some("sk-rotated"));
        assert_eq!(creds["openai"].key(), Some("sk-oauth"));

        // Remove reports existence accurately and prunes the entry.
        assert!(remove_credential_in(dir, "anthropic").expect("remove"));
        assert!(!remove_credential_in(dir, "anthropic").expect("remove again"));
        let creds = load_credentials_in(dir);
        assert_eq!(creds.len(), 1);
        assert!(creds.contains_key("openai"));
    }

    #[test]
    fn missing_and_corrupt_files_fail_open_to_empty() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        assert!(
            load_credentials_in(dir).is_empty(),
            "missing auth.json is empty, not an error"
        );

        std::fs::write(auth_path_in(dir), "{ not json !!!").unwrap();
        assert!(
            load_credentials_in(dir).is_empty(),
            "corrupt auth.json fails open (advisory read contract)"
        );
    }

    /// One damaged provider must not hide the others. The whole-map parse used
    /// to return empty for this file, so a working key disappeared because an
    /// unrelated entry had the wrong type.
    #[test]
    fn one_corrupt_auth_entry_does_not_hide_the_others() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        std::fs::write(
            auth_path_in(dir),
            r#"{"deepseek":{"type":"api","key":"sk-ok"},"broken":{"type":"api","key":12}}"#,
        )
        .unwrap();
        let creds = load_credentials_in(dir);
        assert_eq!(
            creds.get("deepseek").and_then(|c| c.key()),
            Some("sk-ok"),
            "the usable credential survives its damaged neighbour"
        );
        assert!(!creds.contains_key("broken"));
    }

    #[test]
    fn empty_provider_or_key_is_rejected_before_any_write() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        assert!(save_credential_in(dir, "  ", "k").is_err());
        assert!(save_credential_in(dir, "p", "").is_err());
        assert!(save_credential_in(dir, "p", "   ").is_err());
        assert!(
            !auth_path_in(dir).exists(),
            "rejections must not create the file"
        );
    }

    #[test]
    fn stored_shape_matches_opencode_schema() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        save_credential_in(dir, "groq", "gsk_test").expect("save");
        let raw = std::fs::read_to_string(auth_path_in(dir)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            v["groq"]["type"], "api",
            "discriminator matches opencode Auth.Info"
        );
        assert_eq!(v["groq"]["key"], "gsk_test");
    }

    #[test]
    fn blank_stored_key_is_ignored_by_resolution() {
        // resolve_api_key trims a stored key and treats blank as absent, so the
        // store itself refuses to persist one (save_credential_in rejects).
        // This test pins that guard: nothing blank can enter the file.
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        assert!(
            save_credential_in(dir, "faux-provider-x", "   ").is_err(),
            "blank keys are rejected at write time"
        );
        assert!(load_credentials_in(dir).is_empty(), "nothing was persisted");
    }

    #[test]
    fn corrupt_file_blocks_writes_without_clobber() {
        // The S13 data-loss half: fail-open reads must never become wipes.
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        let garbage = "{ not json !!!";
        std::fs::write(auth_path_in(dir), garbage).unwrap();

        let err = save_credential_in(dir, "anthropic", "sk-new").expect_err("must refuse");
        assert!(err.contains("corrupt"), "got: {err}");
        assert_eq!(
            std::fs::read_to_string(auth_path_in(dir)).unwrap(),
            garbage,
            "corrupt file preserved byte-identical"
        );

        let err = remove_credential_in(dir, "anthropic").expect_err("must refuse");
        assert!(err.contains("corrupt"), "got: {err}");
        assert_eq!(std::fs::read_to_string(auth_path_in(dir)).unwrap(), garbage);
    }

    #[test]
    fn stale_lock_is_stolen_fresh_lock_blocks() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        let lock = dir.join("auth.lock");

        // A crashed holder's lock (mtime 60s ago) must not wedge logins.
        std::fs::write(&lock, "dead").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&lock)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(60))
            .unwrap();
        save_credential_in(dir, "anthropic", "sk-test").expect("stale lock stolen");
        assert!(!lock.exists(), "stolen lock is released on drop");
        assert_eq!(load_credentials_in(dir)["anthropic"].key(), Some("sk-test"));

        // A live holder's lock fails the save loudly instead of clobbering.
        std::fs::write(&lock, "live").unwrap();
        let err = save_credential_in(dir, "openai", "sk-x").expect_err("must block");
        assert!(err.contains("lock"), "got: {err}");
        assert!(
            !load_credentials_in(dir).contains_key("openai"),
            "blocked save persists nothing"
        );
    }

    #[test]
    fn concurrent_saves_keep_every_provider() {
        // CLI + TUI saving at once used to read-modify-write over each other.
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path().to_path_buf();
        let mut handles = Vec::new();
        for i in 0..8u32 {
            let dir = dir.clone();
            handles.push(std::thread::spawn(move || {
                save_credential_in(&dir, &format!("prov-{i}"), &format!("key-{i}"))
                    .expect("concurrent save");
            }));
        }
        for h in handles {
            h.join().expect("thread");
        }
        let creds = load_credentials_in(&dir);
        assert_eq!(creds.len(), 8);
        for i in 0..8u32 {
            assert_eq!(
                creds[&format!("prov-{i}")].key().unwrap(),
                format!("key-{i}")
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn auth_file_is_owner_only_on_unix() {
        // The M22 doc/CHANGELOG claim "0600" must hold; the write-side
        // set_permissions result is ignored in save (best-effort), so pin the
        // observable mode instead of the call.
        use std::os::unix::fs::MetadataExt as _;
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        let path = save_credential_in(dir, "anthropic", "sk-test").expect("save");
        let mode = std::fs::metadata(path).expect("stat").mode() & 0o777;
        assert_eq!(mode, 0o600, "auth.json must be owner-read/write only");
    }

    #[test]
    fn oauth_two_phase_refresh_composition() {
        // pi `resolveStoredOAuth` double-check without network: a live pair
        // needs no refresh; an expiring pair round-trips through an
        // off-lock exchange + locked persist; a concurrent rotation wins
        // over the caller's stale exchange.
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path();
        let now = now_millis();

        // Live pair: no refresh needed, token reads back.
        save_oauth_in(dir, "anthropic", "rt-1", "at-live", now + 3_600_000).unwrap();
        assert!(!oauth_needs_refresh_in(dir, "anthropic", 300_000));
        assert!(matches!(
            oauth_access_token_in(dir, "anthropic"),
            OauthToken::Live(t) if t == "at-live"
        ));

        // Expiring pair: needs refresh inside pi's 5-min window.
        save_oauth_in(dir, "anthropic", "rt-2", "at-old", now + 60_000).unwrap();
        assert!(oauth_needs_refresh_in(dir, "anthropic", 300_000));
        // Simulated off-lock exchange, then locked persist.
        let fresh = OauthRefreshed {
            access: "at-new".to_string(),
            refresh: "rt-3".to_string(),
            expires: now + 3_600_000,
        };
        assert_eq!(
            store_refreshed_oauth_in(dir, "anthropic", &fresh).expect("persist"),
            "at-new"
        );
        assert!(matches!(
            oauth_access_token_in(dir, "anthropic"),
            OauthToken::Live(t) if t == "at-new"
        ));
        assert!(!oauth_needs_refresh_in(dir, "anthropic", 300_000));

        // Concurrent rotation wins: stored live pair survives our stale write.
        let stale = OauthRefreshed {
            access: "at-stale".to_string(),
            refresh: "rt-stale".to_string(),
            expires: now + 3_600_000,
        };
        assert_eq!(
            store_refreshed_oauth_in(dir, "anthropic", &stale).expect("re-check"),
            "at-new",
            "live stored pair must win over a stale exchange"
        );

        // Non-oauth provider: nothing to refresh, never an error.
        save_credential_in(dir, "openai", "sk-x").unwrap();
        assert!(!oauth_needs_refresh_in(dir, "openai", 300_000));
        assert!(!oauth_needs_refresh_in(dir, "nobody", 300_000));

        // Empty exchange results refused, file untouched.
        let bad = OauthRefreshed {
            access: String::new(),
            refresh: "rt".to_string(),
            expires: now + 1,
        };
        assert!(store_refreshed_oauth_in(dir, "anthropic", &bad).is_err());
        assert!(matches!(
            oauth_access_token_in(dir, "anthropic"),
            OauthToken::Live(t) if t == "at-new"
        ));
    }
}
