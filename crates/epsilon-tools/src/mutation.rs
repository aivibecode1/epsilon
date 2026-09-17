//! Per-path file-mutation queue (pi `file-mutation-queue.ts` parity).
//!
//! `with_file_mutation_queue` serializes mutations targeting the same canonical
//! path while letting mutations to different paths run concurrently. The
//! ponytail ladder here is a global `HashMap<PathBuf, Arc<Mutex<()>>>` behind a
//! `OnceLock` — no per-env WeakMap, no per-file canonicalization via realpath
//! at v1 (lexically normalized absolute path is the key).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

/// Global map: canonical path → gate mutex. Held only briefly to look up/insert.
fn mutex_for(path: &Path) -> Arc<Mutex<()>> {
    static MAP_SYNC: OnceLock<std::sync::Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let key: PathBuf = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let sync_map = MAP_SYNC.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = sync_map.lock().expect("mutation map poisoned");
    Arc::clone(guard.entry(key).or_insert_with(|| Arc::new(Mutex::new(()))))
}

/// Serialize `f` with respect to `path` — concurrent callers for the same path
/// run one-at-a-time, callers for different paths run in parallel.
pub async fn with_file_mutation_queue<T, F, Fut>(path: &Path, f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let gate = mutex_for(path);
    let _held = gate.lock().await;
    f().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn serializes_same_path() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("same.txt");
        let start = Instant::now();
        let (r1, r2) = tokio::join!(
            with_file_mutation_queue(&p, || async {
                tokio::time::sleep(Duration::from_millis(60)).await;
                1u8
            }),
            with_file_mutation_queue(&p, || async { 2u8 })
        );
        assert_eq!((r1, r2), (1, 2));
        assert!(
            start.elapsed() >= Duration::from_millis(55),
            "same-path must serialize (second waits for first)"
        );
    }

    #[tokio::test]
    async fn different_paths_run_concurrently() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        let start = Instant::now();
        let (r1, r2) = tokio::join!(
            with_file_mutation_queue(&a, || async {
                tokio::time::sleep(Duration::from_millis(60)).await;
                1u8
            }),
            with_file_mutation_queue(&b, || async {
                tokio::time::sleep(Duration::from_millis(60)).await;
                2u8
            })
        );
        assert_eq!((r1, r2), (1, 2));
        assert!(
            start.elapsed() < Duration::from_millis(110),
            "different paths must not serialize"
        );
    }
}
