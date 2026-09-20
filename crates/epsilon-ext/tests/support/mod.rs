//! Shared harness for the extension-host protocol tests.
//!
//! Two jobs: name the fixture binary so every test file refers to one definition, and
//! make "this call never came back" a test failure with a printed deadline instead of a
//! suite that hangs. Not every `#[path]`-included file uses every item, hence the allow.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The fixture child, built by cargo for the integration tests.
pub const FIXTURE_EXE: &str = env!("CARGO_BIN_EXE_ext-fixture");

/// A line the `--garbage` fixture writes. Tests assert it never appears in host state.
pub const GARBAGE_MARKER: &str = "not json at all";

/// The message `--tool-error` reports. The host must pass it through unchanged.
pub const TOOL_ERROR: &str = "the fixture tool failed on purpose";

/// Cap on how long a *spawn* may take before the test calls it a hang. The handshake
/// timeout is `INVOKE_TIMEOUT_SECS`, so this leaves room for process start-up on top.
pub const MANIFEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Cap on any single call that can legitimately hit the ten-second invoke timeout. A call
/// that outlives this is not slow, it is unbounded.
pub const INVOKE_CAP: Duration = Duration::from_secs(25);

/// Path to the fixture binary.
pub fn fixture_exe() -> PathBuf {
    Path::new(FIXTURE_EXE).to_path_buf()
}

/// Await `fut`, failing the test if it outlives `cap`.
///
/// The failure message carries the label the caller supplied and the elapsed time, so a
/// timed-out run says which child misbehaved rather than only that something did.
pub async fn awake<F: std::future::Future>(fut: F, cap: Duration, what: &str) -> F::Output {
    match tokio::time::timeout(cap, fut).await {
        Ok(v) => v,
        Err(_) => panic!(
            "{what} did not return within {cap:?} (this is a hang, not a slow test); note that \
             every line the child sends can restart a per-read timer"
        ),
    }
}
