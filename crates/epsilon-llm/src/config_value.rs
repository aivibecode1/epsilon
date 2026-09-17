//! Config-value resolution: literals, `$ENV`/`${ENV}` templates, and `!shell`
//! commands (pi `resolve-config-value.ts` parity at the value layer).
//!
//! A stored credential's `key` (and any future header value) may be:
//! - a literal key (`sk-ant-…`) — returned as-is;
//! - a template naming environment variables (`$VAULT_KEY`, `${PREFIX}_KEY`)
//!   with `$$` escaping a literal `$` and `$!` a literal `!` — resolved
//!   against the credential's `env` map first, then the process environment
//!   (pi's `env`-param-then-`process.env` order);
//! - a shell command (`!pass show api/openai`, `!op read …`) — run once,
//!   stdout trimmed, result cached for the process lifetime (pi's
//!   `commandResultCache`).
//!
//! Command execution is OFF the hot path: only values starting with `!` spawn
//! a process, with a 10s timeout and stderr discarded. Failures return `None`
//! (the caller falls through to the next key source) rather than panicking —
//! a broken secret helper must degrade, not crash the loop.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Cache for shell-command results, process lifetime (pi's commandResultCache).
fn command_cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Clear the command cache (exported for tests — pi's `clearConfigValueCache`).
pub fn clear_command_cache() {
    if let Ok(mut cache) = command_cache().lock() {
        cache.clear();
    }
}

fn is_env_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_env_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Whether a stored key value is a shell-command reference (leading `!`).
/// Command values resolve only on key read, never on list/display paths —
/// pi's `isCommandConfigValue` guard against executing secrets helpers from
/// UI code.
pub fn is_command_value(config: &str) -> bool {
    config.starts_with('!')
}

/// Resolve one reference against the merged env view. `None` = unresolvable
/// (missing variable, failed/empty command).
pub fn resolve_config_value(
    config: &str,
    extra_env: Option<&std::collections::BTreeMap<String, String>>,
) -> Option<String> {
    if let Some(command) = config.strip_prefix('!') {
        return execute_command(command);
    }
    resolve_template(config, extra_env)
}

/// Resolve every name referenced by a template value (for "missing variable"
/// diagnostics — pi's `getMissingConfigValueEnvVarNames`).
pub fn missing_template_vars(
    config: &str,
    extra_env: Option<&std::collections::BTreeMap<String, String>>,
) -> Vec<String> {
    let mut names = Vec::new();
    for name in template_var_names(config) {
        if lookup_var(&name, extra_env).is_none() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

fn template_var_names(config: &str) -> Vec<String> {
    let bytes = config.as_bytes();
    let mut names = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        let Some(&next) = bytes.get(i + 1) else {
            break;
        };
        match next {
            b'$' | b'!' => {
                i += 2;
            }
            b'{' => {
                if let Some(end) = config[i + 2..].find('}') {
                    let name = &config[i + 2..i + 2 + end];
                    if is_valid_env_name(name) {
                        names.push(name.to_string());
                    }
                    i += 2 + end + 1;
                } else {
                    i += 2;
                }
            }
            _ if is_env_start(next) => {
                let mut j = i + 1;
                while j < bytes.len() && is_env_char(bytes[j]) {
                    j += 1;
                }
                names.push(config[i + 1..j].to_string());
                i = j;
            }
            _ => {
                i += 2;
            }
        }
    }
    names
}

fn is_valid_env_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().next().is_some_and(is_env_start)
        && name.bytes().all(is_env_char)
}

fn lookup_var(
    name: &str,
    extra_env: Option<&std::collections::BTreeMap<String, String>>,
) -> Option<String> {
    if let Some(v) = extra_env.and_then(|m| m.get(name)) {
        return Some(v.clone());
    }
    std::env::var(name).ok()
}

fn resolve_template(
    config: &str,
    extra_env: Option<&std::collections::BTreeMap<String, String>>,
) -> Option<String> {
    let bytes = config.as_bytes();
    let mut out = String::with_capacity(config.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            // Push one UTF-8 char (byte copy would split multi-byte sequences).
            let ch = config[i..].chars().next()?;
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let Some(&next) = bytes.get(i + 1) else {
            out.push('$');
            i += 1;
            continue;
        };
        match next {
            // `$$` → `$`, `$!` → `!` (pi's escape rules, escaping only).
            b'$' | b'!' => {
                out.push(next as char);
                i += 2;
            }
            b'{' => {
                if let Some(end) = config[i + 2..].find('}') {
                    let name = &config[i + 2..i + 2 + end];
                    if is_valid_env_name(name) {
                        out.push_str(&lookup_var(name, extra_env)?);
                    } else {
                        // Not a valid name: literal, pi parity.
                        out.push_str(&config[i..i + 2 + end + 1]);
                    }
                    i += 2 + end + 1;
                } else {
                    // Unterminated `${`: literal `$`, pi parity.
                    out.push('$');
                    i += 1;
                }
            }
            _ if is_env_start(next) => {
                let mut j = i + 1;
                while j < bytes.len() && is_env_char(bytes[j]) {
                    j += 1;
                }
                out.push_str(&lookup_var(&config[i + 1..j], extra_env)?);
                i = j;
            }
            _ => {
                // `$` followed by anything else: literal `$`, pi parity.
                out.push('$');
                i += 1;
            }
        }
    }
    Some(out)
}

fn execute_command(command: &str) -> Option<String> {
    if let Ok(cache) = command_cache().lock()
        && let Some(cached) = cache.get(command).cloned()
    {
        return cached;
    }
    let result = execute_command_uncached(command);
    if let Ok(mut cache) = command_cache().lock() {
        cache.insert(command.to_string(), result.clone());
    }
    result
}

fn execute_command_uncached(command: &str) -> Option<String> {
    #[cfg(windows)]
    {
        execute_via_powershell(command)
    }
    #[cfg(not(windows))]
    {
        execute_via_sh(command)
    }
}

#[cfg(not(windows))]
fn execute_via_sh(command: &str) -> Option<String> {
    run_command("sh", &["-c".to_string(), command.to_string()])
}

#[cfg(windows)]
fn execute_via_powershell(command: &str) -> Option<String> {
    // Same shell choice as the bash tool: PowerShell is always present.
    if let Some(v) = run_command(
        "powershell",
        &[
            "-NoLogo".to_string(),
            "-NonInteractive".to_string(),
            "-Command".to_string(),
            command.to_string(),
        ],
    ) {
        return Some(v);
    }
    // Fall back to cmd for trivial commands when PowerShell is unavailable.
    run_command("cmd", &["/C".to_string(), command.to_string()])
}

fn run_command(program: &str, args: &[String]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_passes_through() {
        assert_eq!(
            resolve_config_value("sk-ant-123", None).as_deref(),
            Some("sk-ant-123")
        );
    }

    #[test]
    fn template_resolves_from_process_env() {
        // `resolve_api_key_in`-style injection is impossible here (env comes
        // from the process), so these cases go through the `extra_env` map —
        // the same code path the credential store uses, without touching the
        // process environment (`std::env::set_var` is `unsafe` in edition
        // 2024, and the workspace forbids `unsafe_code` outright).
        let mut extra = std::collections::BTreeMap::new();
        extra.insert("EPSILON_TEST_VAULT_KEY".to_string(), "secret-1".to_string());
        assert_eq!(
            resolve_config_value("prefix-$EPSILON_TEST_VAULT_KEY", Some(&extra)).as_deref(),
            Some("prefix-secret-1")
        );
        assert_eq!(
            resolve_config_value("${EPSILON_TEST_VAULT_KEY}/suffix", Some(&extra)).as_deref(),
            Some("secret-1/suffix")
        );
        // And the process env still works when nothing overrides it: use a
        // variable Cargo itself guarantees (PATH exists on every platform).
        assert!(
            resolve_config_value("$PATH", None).is_some_and(|v| !v.is_empty()),
            "process env must be consulted"
        );
    }

    #[test]
    fn template_prefers_extra_env_over_process() {
        // PATH is always set; the credential map must shadow it.
        let mut extra = std::collections::BTreeMap::new();
        extra.insert("PATH".to_string(), "from-credential".to_string());
        assert_eq!(
            resolve_config_value("$PATH", Some(&extra)).as_deref(),
            Some("from-credential")
        );
        // Without the override the real PATH (non-empty, platform-set) wins.
        assert_ne!(
            resolve_config_value("$PATH", None).as_deref(),
            Some("from-credential")
        );
    }

    #[test]
    fn missing_variable_is_unresolvable_and_reported() {
        assert_eq!(
            resolve_config_value("$EPSILON_TEST_DEFINITELY_MISSING_XYZ", None),
            None
        );
        assert_eq!(
            missing_template_vars("a-$EPSILON_TEST_DEFINITELY_MISSING_XYZ-b", None),
            vec!["EPSILON_TEST_DEFINITELY_MISSING_XYZ".to_string()]
        );
    }

    #[test]
    fn escapes_and_invalid_names_stay_literal() {
        assert_eq!(
            resolve_config_value("$$HOME", None).as_deref(),
            Some("$HOME")
        );
        assert_eq!(resolve_config_value("$!cmd", None).as_deref(), Some("!cmd"));
        assert_eq!(
            resolve_config_value("cost $5", None).as_deref(),
            Some("cost $5")
        );
        assert_eq!(
            resolve_config_value("${not-a-name}", None).as_deref(),
            Some("${not-a-name}")
        );
        assert_eq!(
            resolve_config_value("trailing$", None).as_deref(),
            Some("trailing$")
        );
    }

    #[test]
    fn command_output_is_used_trimmed_and_cached() {
        clear_command_cache();
        // Both shells echo with a trailing newline; resolution trims it.
        // (`echo` exists on PowerShell and sh alike, so no per-OS branch.)
        assert_eq!(
            resolve_config_value("!echo m36-command-key", None).as_deref(),
            Some("m36-command-key")
        );
        // Cached: a second call returns the same without re-executing.
        assert_eq!(
            resolve_config_value("!echo m36-command-key", None).as_deref(),
            Some("m36-command-key")
        );
        clear_command_cache();
    }

    #[test]
    fn failing_command_is_unresolvable() {
        clear_command_cache();
        // `exit 3` fails on PowerShell and sh alike — no per-OS branch.
        assert_eq!(resolve_config_value("!exit 3", None), None);
    }
}
