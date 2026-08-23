//! `extension.json` manifest: identity, launch command, and declared tools.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// One tool an extension declares.
#[derive(Debug, Clone, Deserialize)]
pub struct ExtToolSpec {
    /// Tool name as the model sees it.
    pub name: String,
    /// LLM-facing description.
    pub description: String,
    /// JSON Schema (2020-12) for arguments.
    pub schema: serde_json::Value,
}

/// Manifest file (`extension.json`), validated on load.
#[derive(Debug, Clone, Deserialize)]
pub struct ExtManifest {
    /// Extension name (diagnostics + tool namespace).
    pub name: String,
    /// Semver-ish version string (informational).
    pub version: String,
    /// Executable to spawn; relative paths resolve against the extension directory.
    pub bin: PathBuf,
    /// Extra argv for `bin` (e.g. a script entry point flag).
    #[serde(default)]
    pub args: Vec<String>,
    /// Tools this extension provides. An extension with zero tools is event-only.
    #[serde(default)]
    pub tools: Vec<ExtToolSpec>,
    /// Event types the extension wants forwarded. `["*"]` = everything; empty = none.
    /// Names match AgentEvent serde tags (e.g. "tool_execution_end").
    #[serde(default)]
    pub events: Vec<String>,
}

impl ExtManifest {
    /// Should this event type be forwarded to the extension?
    pub fn wants_event(&self, event_type: &str) -> bool {
        self.events.iter().any(|e| e == "*" || e == event_type)
    }
}

/// Load + validate a manifest from `<dir>/extension.json`.
pub fn load(dir: &Path) -> Result<ExtManifest, String> {
    let path = dir.join("extension.json");
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {path:?}: {e}"))?;
    let m: ExtManifest =
        serde_json::from_str(&raw).map_err(|e| format!("invalid manifest {path:?}: {e}"))?;
    if m.name.is_empty() {
        return Err("manifest name must not be empty".into());
    }
    if m.bin.as_os_str().is_empty() {
        return Err("manifest bin must not be empty".into());
    }
    Ok(m)
}

impl ExtManifest {
    /// Resolve `bin` against the extension directory when relative.
    pub fn resolved_bin(&self, dir: &Path) -> PathBuf {
        if self.bin.is_absolute() {
            self.bin.clone()
        } else {
            dir.join(&self.bin)
        }
    }
}

/// Test-support constructor: in-memory manifest without touching disk.
#[doc(hidden)]
pub fn load_or_build_test_manifest(events: Vec<String>) -> ExtManifest {
    ExtManifest {
        name: "test".into(),
        version: "0.0.0".into(),
        bin: PathBuf::new(),
        args: vec![],
        tools: vec![],
        events,
    }
}
