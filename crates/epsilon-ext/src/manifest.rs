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
    /// Tools this extension provides. An extension with zero tools is event-only (later).
    #[serde(default)]
    pub tools: Vec<ExtToolSpec>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_manifest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("extension.json"),
            serde_json::json!({
                "name": "echo-ext",
                "version": "0.1.0",
                "bin": "./ext-echo",
                "tools": [{
                    "name": "ext_echo",
                    "description": "Echo text back.",
                    "schema": {"type": "object", "properties": {"text": {"type": "string"}}}
                }]
            })
            .to_string(),
        )
        .unwrap();

        let m = load(dir.path()).unwrap();
        assert_eq!(m.name, "echo-ext");
        assert_eq!(m.tools.len(), 1);
        assert!(m.resolved_bin(dir.path()).starts_with(dir.path()));
    }

    #[test]
    fn rejects_empty_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("extension.json"),
            r#"{"name":"","version":"0","bin":"x"}"#,
        )
        .unwrap();
        assert!(load(dir.path()).is_err());
    }
}
