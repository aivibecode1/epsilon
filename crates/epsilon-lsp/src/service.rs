//! The lifecycle that keeps diagnostics from becoming a process leak.
//!
//! opencode keys its clients by file extension and nearest project root, dedupes in-flight spawns,
//! and keeps a `broken` set so a server that fails is not retried forever
//! (`packages/opencode/src/lsp/lsp.ts:208-297`). All three behaviours are here, and the third is
//! the one that matters in a long session: a server that crashes on startup would otherwise be
//! respawned on every single edit.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::client::{Client, ClientConfig, ClientState};
use crate::report::render_report;
use crate::server::{SERVERS, ServerDef, project_root, select_server, which};
use epsilon_contracts::lsp_queries::{LspLocation, LspQueryFailure, LspQueryProvider, LspSymbol};

/// How the service finds and runs servers.
#[derive(Clone)]
pub struct ServiceConfig {
    /// Supplied instead of `PATH` lookup, per server id (tests, and a user override later).
    pub commands: HashMap<&'static str, PathBuf>,
    /// Environment for every spawned server (after scrubbing).
    pub env: Vec<(String, String)>,
    /// Replace the built-in definition for a file extension (tests: a fixture server; later: a
    /// user's own server for a language this table does not know).
    pub def_override: Option<&'static ServerDef>,
    /// How long to wait for a diagnostic publish after a document change.
    pub publish_timeout: Duration,
    /// How long to wait for the `initialize` handshake.
    pub handshake_timeout: Duration,
    /// Environment scrubber applied to every spawned server.
    ///
    /// The crate applies a conservative name-based floor by default; the harness passes
    /// `epsilon-safety`'s rule in, so the subprocess policy lives in one place.
    pub scrub_env: Option<crate::client::ScrubEnv>,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            commands: HashMap::new(),
            env: Vec::new(),
            def_override: None,
            publish_timeout: Duration::from_secs(4),
            handshake_timeout: Duration::from_secs(10),
            scrub_env: None,
        }
    }
}

impl std::fmt::Debug for ServiceConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceConfig")
            .field("commands", &self.commands.len())
            .field("def_override", &self.def_override.map(|d| d.id))
            .field("env", &self.env.len())
            .field("publish_timeout", &self.publish_timeout)
            .field("handshake_timeout", &self.handshake_timeout)
            .field("scrub_env", &self.scrub_env.is_some())
            .finish()
    }
}

/// Diagnostics for a file an edit just touched.
///
/// Every method is total: a missing server, a broken server, a server that never answers — all of
/// them are `None`, because the edit has already succeeded and the harness has no business failing
/// it for want of a linter.
pub struct DiagnosticsService {
    cfg: ServiceConfig,
    /// Keyed by `server-id:root`; the map's lock is held across a spawn, which is how the in-flight
    /// dedupe is guaranteed rather than hoped for.
    clients: Mutex<HashMap<String, Arc<Mutex<Client>>>>,
    /// Keys that must not be tried again (spawn failed, or the client went broken).
    broken: Mutex<HashSet<String>>,
}

impl Default for DiagnosticsService {
    fn default() -> Self {
        Self::new(ServiceConfig::default())
    }
}

impl DiagnosticsService {
    /// Build a service with the given configuration.
    pub fn new(cfg: ServiceConfig) -> Self {
        Self {
            cfg,
            clients: Mutex::new(HashMap::new()),
            broken: Mutex::new(HashSet::new()),
        }
    }

    /// Servers this machine can actually run, for the doctor report: `(name, found)`.
    ///
    /// Discovery only — nothing is spawned, so this is safe to call from a diagnostic that runs
    /// before the runtime exists.
    pub fn discover(&self) -> Vec<(&'static ServerDef, PathBuf)> {
        crate::server::SERVERS
            .iter()
            .filter_map(|def| self.command_path(def).map(|path| (def, path)))
            .collect()
    }

    /// The definition that answers for `path`: the caller's override when it claims the extension,
    /// otherwise the built-in table.
    fn select(&self, path: &Path) -> Option<&'static ServerDef> {
        if let Some(def) = self.cfg.def_override {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase());
            if let Some(ext) = ext
                && def.extensions.iter().any(|e| *e == ext)
            {
                return Some(def);
            }
        }
        select_server(path)
    }

    fn command_path(&self, def: &'static ServerDef) -> Option<PathBuf> {
        if let Some(path) = self.cfg.commands.get(def.id) {
            return path.is_file().then(|| path.clone());
        }
        which(def.command)
    }

    /// Report diagnostics for `path` after its content became `text`.
    ///
    /// `None` means "nothing to say": no server is configured for this kind of file, no server
    /// binary is installed, the server is broken, or it reported no errors.
    pub async fn after_change(&self, path: &Path, text: &str) -> Option<String> {
        let def = self.select(path)?;
        let command_path = self.command_path(def)?;
        let root = project_root(path, def.root_markers);
        let key = format!("{}:{}", def.id, root.display());

        if self.broken.lock().await.contains(&key) {
            return None;
        }

        let client = { self.client_for(&key, def, root, command_path).await? };
        let mut guard = client.lock().await;
        let diagnostics = match guard.diagnose(path, text).await {
            Ok(diagnostics) => diagnostics,
            Err(_) => {
                // A client that cannot answer will not answer next time either.
                guard.state();
                self.mark_broken(&key).await;
                return None;
            }
        };
        if guard.state() == crate::client::ClientState::Broken {
            self.mark_broken(&key).await;
            return None;
        }
        drop(guard);
        render_report(path, &diagnostics)
    }

    /// Get the client for `key`, spawning and handshaking it if this is the first ask.
    async fn client_for(
        &self,
        key: &str,
        def: &'static ServerDef,
        root: PathBuf,
        command_path: PathBuf,
    ) -> Option<Arc<Mutex<Client>>> {
        // The map lock is held across the spawn: two edits landing together must not start two
        // servers, and a language server's index build is expensive enough that a duplicate is a
        // real cost, not a cosmetic one.
        let mut clients = self.clients.lock().await;
        if let Some(existing) = clients.get(key) {
            return Some(Arc::clone(existing));
        }
        let mut cfg = ClientConfig::new(def, root, command_path);
        cfg.handshake_timeout = self.cfg.handshake_timeout;
        cfg.publish_timeout = self.cfg.publish_timeout;
        cfg.env = self.cfg.env.clone();
        let client = match Client::start_with(cfg, self.cfg.scrub_env.clone()).await {
            Ok(client) => client,
            Err(_) => {
                self.broken.lock().await.insert(key.to_string());
                return None;
            }
        };
        let client = Arc::new(Mutex::new(client));
        clients.insert(key.to_string(), Arc::clone(&client));
        Some(client)
    }

    async fn mark_broken(&self, key: &str) {
        self.broken.lock().await.insert(key.to_string());
        self.clients.lock().await.remove(key);
    }

    /// Every extension the server table knows, sorted — the unknown-extension failure lists
    /// these, so the message names what would have worked.
    fn known_extensions(&self) -> Vec<String> {
        let mut known: Vec<String> = SERVERS
            .iter()
            .flat_map(|def| def.extensions.iter().map(|ext| (*ext).to_string()))
            .collect();
        if let Some(extra) = self.cfg.def_override {
            for ext in extra.extensions {
                if !known.iter().any(|known| known == ext) {
                    known.push((*ext).to_string());
                }
            }
        }
        known.sort();
        known
    }

    /// The running client for `path`'s language, spawning and handshaking it if needed.
    ///
    /// The same multiplex the diagnostics path uses — the same `server-id:root` keying, the
    /// same broken set, the same in-flight dedupe — so a query never spawns a server an edit
    /// already runs, and a dead server is not respawned per query.
    async fn query_client(
        &self,
        path: &Path,
    ) -> Result<(Arc<Mutex<Client>>, String, &'static ServerDef), LspQueryFailure> {
        let def = self
            .select(path)
            .ok_or_else(|| LspQueryFailure::UnknownExtension {
                extension: path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase())
                    .unwrap_or_default(),
                known: self.known_extensions(),
            })?;
        let command_path =
            self.command_path(def)
                .ok_or_else(|| LspQueryFailure::MissingBinary {
                    name: def.name.to_string(),
                    binary: def.command.to_string(),
                })?;
        let root = project_root(path, def.root_markers);
        let key = format!("{}:{}", def.id, root.display());

        if self.broken.lock().await.contains(&key) {
            return Err(LspQueryFailure::Broken {
                detail: format!(
                    "{}: a previous failure marked this server broken for this session",
                    def.id
                ),
            });
        }

        let client = self
            .client_for(&key, def, root, command_path)
            .await
            .ok_or_else(|| LspQueryFailure::Broken {
                detail: format!("{}: the server could not be started", def.id),
            })?;
        Ok((client, key, def))
    }

    /// Settle one query: a late or failed answer marks the server broken, because a request —
    /// unlike a fire-and-forget publish — must be answered for the conversation to stay in
    /// sync. Waits up to the configured publish timeout (the same "how long is an answer
    /// allowed to take" the diagnostics path already configures).
    async fn finish_query<T>(
        &self,
        key: &str,
        def: &ServerDef,
        broken: bool,
        answer: Result<Result<T, String>, tokio::time::error::Elapsed>,
        op: &str,
    ) -> Result<T, LspQueryFailure> {
        match answer {
            Ok(Ok(hits)) if !broken => Ok(hits),
            Ok(Ok(_)) => {
                self.mark_broken(key).await;
                Err(LspQueryFailure::Broken {
                    detail: format!("{}: the server died while answering {op}", def.id),
                })
            }
            Ok(Err(e)) => {
                self.mark_broken(key).await;
                Err(LspQueryFailure::Broken {
                    detail: format!("{}: {e}", def.id),
                })
            }
            Err(_) => {
                self.mark_broken(key).await;
                Err(LspQueryFailure::Broken {
                    detail: format!(
                        "{}: no answer to {op} within {:?}",
                        def.id, self.cfg.publish_timeout
                    ),
                })
            }
        }
    }

    /// Where `path`'s symbol at 0-based (`line`, `character`) is defined (1-based out).
    pub async fn definition(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<LspLocation>, LspQueryFailure> {
        let (client, key, def) = self.query_client(path).await?;
        // Best effort: the server answers from its own open documents when the file cannot
        // be read here, and an empty sync is still a sync.
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let mut guard = client.lock().await;
        let wait = self.cfg.publish_timeout;
        let answer =
            tokio::time::timeout(wait, guard.go_to_definition(path, &text, line, character)).await;
        let broken = guard.state() == ClientState::Broken;
        self.finish_query(&key, def, broken, answer, "definition")
            .await
    }

    /// Where the symbol at 0-based (`line`, `character`) of `path` is used (1-based out).
    pub async fn find_references(
        &self,
        path: &Path,
        line: u32,
        character: u32,
        include_declaration: bool,
    ) -> Result<Vec<LspLocation>, LspQueryFailure> {
        let (client, key, def) = self.query_client(path).await?;
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let mut guard = client.lock().await;
        let wait = self.cfg.publish_timeout;
        let answer = tokio::time::timeout(
            wait,
            guard.find_references(path, &text, line, character, include_declaration),
        )
        .await;
        let broken = guard.state() == ClientState::Broken;
        self.finish_query(&key, def, broken, answer, "references")
            .await
    }

    /// Where the type at 0-based (`line`, `character`) of `path` is implemented (1-based out).
    pub async fn implementation(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<LspLocation>, LspQueryFailure> {
        let (client, key, def) = self.query_client(path).await?;
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let mut guard = client.lock().await;
        let wait = self.cfg.publish_timeout;
        let answer = tokio::time::timeout(
            wait,
            guard.go_to_implementation(path, &text, line, character),
        )
        .await;
        let broken = guard.state() == ClientState::Broken;
        self.finish_query(&key, def, broken, answer, "implementation")
            .await
    }

    /// Every named symbol `path` defines (1-based out).
    pub async fn document_symbol(&self, path: &Path) -> Result<Vec<LspSymbol>, LspQueryFailure> {
        let (client, key, def) = self.query_client(path).await?;
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let mut guard = client.lock().await;
        let wait = self.cfg.publish_timeout;
        let answer = tokio::time::timeout(wait, guard.document_symbol(path, &text)).await;
        let broken = guard.state() == ClientState::Broken;
        self.finish_query(&key, def, broken, answer, "documentSymbol")
            .await
    }

    /// Stop every server this service started.
    pub async fn shutdown(&self) {
        let clients: Vec<Arc<Mutex<Client>>> = {
            let mut map = self.clients.lock().await;
            map.drain().map(|(_, c)| c).collect()
        };
        for client in clients {
            if let Ok(client) = Arc::try_unwrap(client) {
                client.into_inner().shutdown().await;
            }
        }
    }
}

#[async_trait::async_trait]
impl epsilon_contracts::diagnostics::DiagnosticsProvider for DiagnosticsService {
    async fn after_change(&self, path: &Path, text: &str) -> Option<String> {
        DiagnosticsService::after_change(self, path, text).await
    }
}

#[async_trait::async_trait]
impl LspQueryProvider for DiagnosticsService {
    async fn go_to_definition(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<LspLocation>, LspQueryFailure> {
        DiagnosticsService::definition(self, path, line, character).await
    }

    async fn find_references(
        &self,
        path: &Path,
        line: u32,
        character: u32,
        include_declaration: bool,
    ) -> Result<Vec<LspLocation>, LspQueryFailure> {
        DiagnosticsService::find_references(self, path, line, character, include_declaration).await
    }

    async fn go_to_implementation(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<LspLocation>, LspQueryFailure> {
        DiagnosticsService::implementation(self, path, line, character).await
    }

    async fn document_symbol(&self, path: &Path) -> Result<Vec<LspSymbol>, LspQueryFailure> {
        DiagnosticsService::document_symbol(self, path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_file_no_server_claims_is_silent_and_spawns_nothing() {
        let service = DiagnosticsService::default();
        assert_eq!(
            service.after_change(Path::new("notes.txt"), "hello").await,
            None
        );
        assert_eq!(
            service
                .after_change(Path::new("no-extension"), "hello")
                .await,
            None
        );
    }

    #[tokio::test]
    async fn a_server_that_is_not_installed_is_silent_not_an_error() {
        // The whole point of silent degradation: no rust-analyzer, no diagnostics, no failure.
        let mut cfg = ServiceConfig::default();
        cfg.commands
            .insert("rust-analyzer", PathBuf::from("definitely-not-here-xyz"));
        let service = DiagnosticsService::new(cfg);
        assert_eq!(
            service
                .after_change(Path::new("src/lib.rs"), "fn main() {}")
                .await,
            None
        );
    }
}

#[cfg(test)]
mod query_tests {
    use super::*;
    use std::time::Duration;

    use epsilon_contracts::lsp_queries::LspQueryFailure;

    /// A config whose "rust-analyzer" is the fixture, with the start-log wired when asked.
    fn fixture_config(log: Option<&Path>) -> ServiceConfig {
        let mut cfg = ServiceConfig::default();
        cfg.commands.insert("rust-analyzer", fixture_exe());
        cfg.publish_timeout = Duration::from_secs(10);
        cfg.handshake_timeout = Duration::from_secs(10);
        if let Some(path) = log {
            cfg.env.push((
                "EPSILON_LSP_FIXTURE_LOG".to_string(),
                path.display().to_string(),
            ));
        }
        cfg
    }

    /// A definition whose fixture variant refuses to handshake.
    fn refusing_def() -> &'static crate::server::ServerDef {
        Box::leak(Box::new(crate::server::ServerDef {
            id: "rust-analyzer",
            name: "fixture (refuses)",
            extensions: &["rs"],
            command: "lsp_fixture_server",
            args: &["--refuse"],
            root_markers: &["Cargo.toml"],
        }))
    }

    /// A definition whose fixture variant answers documentSymbol with nothing.
    fn no_symbols_def() -> &'static crate::server::ServerDef {
        Box::leak(Box::new(crate::server::ServerDef {
            id: "rust-analyzer",
            name: "fixture (no symbols)",
            extensions: &["rs"],
            command: "lsp_fixture_server",
            args: &["--no-symbols"],
            root_markers: &["Cargo.toml"],
        }))
    }

    /// The fixture executable beside this test binary; see the client tests for why the
    /// path is resolved at run time rather than through `CARGO_BIN_EXE_…`.
    fn fixture_exe() -> PathBuf {
        if let Ok(path) = std::env::var("CARGO_BIN_EXE_lsp_fixture_server") {
            return PathBuf::from(path);
        }
        let mut dir = std::env::current_exe().expect("test executable path");
        dir.pop();
        if dir.file_name().is_some_and(|n| n == "deps") {
            dir.pop();
        }
        dir.join(format!(
            "lsp_fixture_server{}",
            std::env::consts::EXE_SUFFIX
        ))
    }

    /// A workspace with a Cargo.toml marker and one `.rs` file.
    fn workspace(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        let file = src.join("main.rs");
        std::fs::write(&file, text).unwrap();
        (dir, file)
    }

    /// Definition answers rebased to 1-based, and a second op reuses the running server.
    #[tokio::test]
    async fn definition_is_rebased_and_the_server_is_reused() {
        let (_dir, file) = workspace("fn main() {}\n");
        let log = tempfile::NamedTempFile::new().unwrap();
        let service = DiagnosticsService::new(fixture_config(Some(log.path())));
        let hits = service
            .definition(&file, 4, 2)
            .await
            .expect("the fixture answers");
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].line, hits[0].character), (5, 3));
        assert_eq!(hits[0].path, file);
        let impls = service
            .implementation(&file, 0, 0)
            .await
            .expect("implementation answers");
        assert_eq!(impls.len(), 1);
        assert_eq!((impls[0].line, impls[0].character), (1, 1));
        let starts = std::fs::read_to_string(log.path()).unwrap_or_default();
        assert_eq!(
            starts.lines().count(),
            1,
            "one server for both queries, not two: {starts}"
        );
        service.shutdown().await;
    }

    /// References honour `includeDeclaration`: 25 canned hits with it, 24 without.
    #[tokio::test]
    async fn references_honour_include_declaration() {
        let (_dir, file) = workspace("fn main() {}\n");
        let service = DiagnosticsService::new(fixture_config(None));
        let with = service
            .find_references(&file, 4, 2, true)
            .await
            .expect("references answer");
        assert_eq!(with.len(), 25);
        let without = service
            .find_references(&file, 4, 2, false)
            .await
            .expect("references answer");
        assert_eq!(without.len(), 24);
        service.shutdown().await;
    }

    /// No server claims `.txt`: the failure names the extension and lists what is known.
    #[tokio::test]
    async fn an_unknown_extension_lists_what_is_known() {
        let service = DiagnosticsService::default();
        let err = service
            .definition(Path::new("notes.txt"), 0, 0)
            .await
            .expect_err("no server claims .txt");
        let LspQueryFailure::UnknownExtension { extension, known } = err else {
            panic!("wrong failure: {err:?}");
        };
        assert_eq!(extension, "txt");
        assert!(
            known.contains(&"rs".to_string()),
            "lists known extensions: {known:?}"
        );
    }

    /// The server for `.rs` is not installed: the failure names the missing binary.
    #[tokio::test]
    async fn a_missing_binary_names_the_executable() {
        let mut cfg = ServiceConfig::default();
        cfg.commands
            .insert("rust-analyzer", PathBuf::from("definitely-not-here-xyz"));
        let service = DiagnosticsService::new(cfg);
        let err = service
            .definition(Path::new("src/lib.rs"), 0, 0)
            .await
            .expect_err("no binary, no query");
        assert!(
            matches!(
                err,
                LspQueryFailure::MissingBinary { ref binary, .. } if binary == "rust-analyzer"
            ),
            "names the binary: {err:?}"
        );
    }

    /// A server that refuses to handshake is broken once and never retried.
    #[tokio::test]
    async fn a_refusing_server_is_broken_once_and_never_retried() {
        let (_dir, file) = workspace("fn main() {}\n");
        let log = tempfile::NamedTempFile::new().unwrap();
        let mut cfg = fixture_config(Some(log.path()));
        cfg.def_override = Some(refusing_def());
        let service = DiagnosticsService::new(cfg);
        assert!(matches!(
            service.definition(&file, 0, 0).await,
            Err(LspQueryFailure::Broken { .. })
        ));
        assert!(matches!(
            service.definition(&file, 0, 0).await,
            Err(LspQueryFailure::Broken { .. })
        ));
        let starts = std::fs::read_to_string(log.path()).unwrap_or_default();
        assert_eq!(
            starts.lines().count(),
            1,
            "a broken server is not respawned per query: {starts}"
        );
        service.shutdown().await;
    }

    /// A query that ran and found nothing is `Ok`, not a failure.
    #[tokio::test]
    async fn zero_hits_are_ok_not_a_failure() {
        let (_dir, file) = workspace("fn main() {}\n");
        let mut cfg = fixture_config(None);
        cfg.def_override = Some(no_symbols_def());
        let service = DiagnosticsService::new(cfg);
        let symbols = service
            .document_symbol(&file)
            .await
            .expect("an empty answer is still an answer");
        assert!(symbols.is_empty());
        service.shutdown().await;
    }
}
