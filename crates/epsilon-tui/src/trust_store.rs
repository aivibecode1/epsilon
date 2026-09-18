//! Remembered per-path project trust (pi `ProjectTrustStore`).
//!
//! M45: trust is two mechanisms — the config hash (per-file integrity, owned by
//! `epsilon-cli::runtime`) and THIS store (per-path memory at
//! `<global_dir>/trust.json`: path → true/false, nearest-ancestor inheritance).
//! The store lives here (not in epsilon-cli) so the TUI can record acceptances
//! without depending on the CLI crate (dependency law: tui <- cli would cycle).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// path → trusted map (pi `TrustFile`: `true`/`false`; `null` removes the key).
type TrustMap = BTreeMap<String, bool>;

/// Remembered per-path trust decision (pi `ProjectTrustDecision`):
/// `Some(true)` trusted, `Some(false)` refused, `None` never asked.
pub type ProjectTrustDecision = Option<bool>;

/// One remembered decision: nearest ancestor with a `true`/`false` value wins
/// (pi `findNearestTrustEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectTrustEntry {
    /// Canonicalized directory path the decision was recorded for.
    pub path: String,
    /// The remembered decision.
    pub decision: bool,
}

/// Persistent per-path trust memory (pi `ProjectTrustStore`).
///
/// Keys sorted on write; [`ProjectTrustStore::clear`] removes the key (pi's
/// `null`). Reads and writes hold a `trust.json.lock` sibling with
/// stale-steal, so concurrent frontends never interleave a torn write.
pub struct ProjectTrustStore {
    path: PathBuf,
}

impl ProjectTrustStore {
    /// Open the store under the global dir (`None` when no home resolves —
    /// callers fall back to hash-only trust).
    pub fn open() -> Option<Self> {
        let dir = epsilon_core::config::global_dir()?;
        Some(Self {
            path: dir.join("trust.json"),
        })
    }

    /// Open at an explicit path (tests).
    pub fn open_in(path: PathBuf) -> Self {
        Self { path }
    }

    /// Remembered decision for `cwd`: the nearest `cwd`-or-ancestor entry,
    /// or `None` when nothing was ever recorded (pi `get`).
    pub fn get(&self, cwd: &Path) -> ProjectTrustDecision {
        self.get_entry(cwd).map(|e| e.decision)
    }

    /// Nearest entry at or above `cwd` (pi `getEntry`).
    pub fn get_entry(&self, cwd: &Path) -> Option<ProjectTrustEntry> {
        let _lock = self.lock()?;
        let data = self.read()?;
        find_nearest(&data, cwd)
    }

    /// Record one decision (`None` clears the key — pi's `null`).
    pub fn set(&self, cwd: &Path, decision: ProjectTrustDecision) -> Result<(), String> {
        self.set_many(std::slice::from_ref(&(cwd.to_path_buf(), decision)))
    }

    /// Record several decisions atomically, sorted keys on write (pi `setMany`).
    pub fn set_many(&self, decisions: &[(PathBuf, ProjectTrustDecision)]) -> Result<(), String> {
        let _lock = self
            .lock()
            .ok_or_else(|| "could not lock the trust store".to_string())?;
        let mut data = self.read().unwrap_or_default();
        for (path, decision) in decisions {
            let key = canonicalize(path);
            match decision {
                None => {
                    data.remove(&key);
                }
                Some(d) => {
                    data.insert(key, *d);
                }
            }
        }
        self.write(&data)
    }

    /// Remove the entry for `cwd` (forget — the next run asks again).
    pub fn clear(&self, cwd: &Path) -> Result<(), String> {
        self.set(cwd, None)
    }

    fn lock(&self) -> Option<LockGuard> {
        LockGuard::acquire(&self.path)
    }

    fn read(&self) -> Option<TrustMap> {
        let raw = std::fs::read_to_string(&self.path).ok()?;
        if raw.trim().is_empty() {
            return Some(TrustMap::new());
        }
        let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
        let obj = value.as_object()?;
        let mut map = TrustMap::new();
        for (k, v) in obj {
            match v {
                serde_json::Value::Bool(b) => {
                    map.insert(k.clone(), *b);
                }
                serde_json::Value::Null => {} // `null` never persists; skip defensively
                _ => return None,             // corrupt value: store unreadable
            }
        }
        Some(map)
    }

    fn write(&self, data: &TrustMap) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
        let mut obj = serde_json::Map::with_capacity(data.len());
        for (k, v) in data {
            obj.insert(k.clone(), serde_json::Value::Bool(*v));
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::Value::Object(obj).to_string())
            .map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("could not publish {}: {e}", self.path.display()))?;
        Ok(())
    }
}

/// Canonicalize a workspace path for store keys: absolute + lexically
/// normalized (pi `normalizeCwd`: `canonicalizePath(resolvePath(cwd))`).
fn canonicalize(path: &Path) -> String {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut out = PathBuf::new();
    for comp in abs.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out.to_string_lossy().into_owned()
}

/// Nearest `cwd`-or-ancestor key with a decision (pi `findNearestTrustEntry`).
fn find_nearest(data: &TrustMap, cwd: &Path) -> Option<ProjectTrustEntry> {
    let mut dir = canonicalize(cwd);
    loop {
        if let Some(d) = data.get(&dir) {
            return Some(ProjectTrustEntry {
                path: dir,
                decision: *d,
            });
        }
        let parent = Path::new(&dir).parent()?.to_string_lossy().into_owned();
        if parent == dir {
            return None;
        }
        dir = parent;
    }
}

/// Cross-process lock for the trust store (same stale-steal discipline as the
/// auth store: a `trust.json.lock` sibling; holders older than 30s are
/// presumed dead). Returns `None` when the lock is unusable — callers treat
/// that as "store unavailable", never as trusted.
struct LockGuard {
    path: PathBuf,
}

impl LockGuard {
    fn acquire(store_path: &Path) -> Option<Self> {
        let dir = store_path.parent()?;
        std::fs::create_dir_all(dir).ok()?;
        let path = store_path.with_extension("json.lock");
        // Best-effort create-exclusive; stale holders (>30s) are stolen.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                use std::io::Write;
                let _ = writeln!(f, "{}", std::process::id());
                Some(Self { path })
            }
            Err(_) => {
                let stale = std::fs::metadata(&path)
                    .ok()?
                    .modified()
                    .ok()?
                    .elapsed()
                    .map(|e| e.as_secs() > 30)
                    .unwrap_or(false);
                if stale {
                    let _ = std::fs::remove_file(&path);
                    Self::acquire(store_path)
                } else {
                    None
                }
            }
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_in(dir: &Path) -> ProjectTrustStore {
        ProjectTrustStore::open_in(dir.join("trust.json"))
    }

    #[test]
    fn empty_store_asks_set_get_clear_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        let ws = tmp.path().join("ws");
        assert_eq!(s.get(&ws), None);
        s.set(&ws, Some(true)).unwrap();
        assert_eq!(s.get(&ws), Some(true));
        s.set(&ws, Some(false)).unwrap();
        assert_eq!(s.get(&ws), Some(false));
        s.clear(&ws).unwrap();
        assert_eq!(s.get(&ws), None);
    }

    #[test]
    fn nearest_ancestor_wins_and_child_overrides() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        let parent = tmp.path().join("repo");
        let child = parent.join("sub");
        s.set(&parent, Some(true)).unwrap();
        assert_eq!(s.get(&child), Some(true));
        let entry = s.get_entry(&child).unwrap();
        assert!(entry.path.ends_with("repo"));
        s.set(&child, Some(false)).unwrap();
        assert_eq!(s.get(&child), Some(false));
        // Clearing the child falls back to the parent.
        s.clear(&child).unwrap();
        assert_eq!(s.get(&child), Some(true));
    }

    #[test]
    fn set_many_is_atomic_and_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        let a = tmp.path().join("b-proj");
        let b = tmp.path().join("a-proj");
        s.set_many(&[(a.clone(), Some(true)), (b.clone(), Some(false))])
            .unwrap();
        assert_eq!(s.get(&a), Some(true));
        assert_eq!(s.get(&b), Some(false));
        let raw = std::fs::read_to_string(tmp.path().join("trust.json")).unwrap();
        let apos = raw.find("b-proj").unwrap();
        let bpos = raw.find("a-proj").unwrap();
        assert!(bpos < apos, "keys sorted on write: {raw}");
    }

    #[test]
    fn corrupt_store_reads_as_no_memory() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        std::fs::write(tmp.path().join("trust.json"), "{bad json").unwrap();
        assert_eq!(s.get(&tmp.path().join("ws")), None);
        // ...but a write heals it (atomic tmp+rename replaces the garbage).
        s.set(&tmp.path().join("ws"), Some(true)).unwrap();
        assert_eq!(s.get(&tmp.path().join("ws")), Some(true));
    }
}
