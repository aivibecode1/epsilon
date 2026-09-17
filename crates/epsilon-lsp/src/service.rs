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

use crate::client::{Client, ClientConfig};
use crate::report::render_report;
use crate::server::{ServerDef, project_root, select_server, which};

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
