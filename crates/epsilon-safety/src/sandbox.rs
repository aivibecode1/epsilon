//! Layer 2 of the two-layer safety model (ADR-005): OS confinement for child processes.
//!
//! Semantics ported from DSH's sandbox provider (HA-DSH §5, `packages/sandbox`): policy
//! rides each call, backends are probed per platform, and **confinement fails closed** —
//! when a backend is demanded but unavailable, the command is refused rather than run
//! unconfined. Denials are distinguishable from command failures by the
//! [`SandboxError`] type (DSH's `denialSignatures`, message-shaped).
//!
//! Backends:
//! - [`JobObjectSandbox`] (Windows): Job Object with memory / active-process / CPU-rate
//!   caps and `KILL_ON_JOB_CLOSE`, so descendants cannot outlive or escape the call.
//! - `BwrapSandbox` (Unix): bubblewrap argv builder + invocation (filesystem/unshare
//!   isolation; resource caps need cgroups and are out of v1 scope — documented).
//! - [`NullSandbox`]: fail-closed placeholder; `available() == false` always.
//!
//! In-band escalation (ADR-005): a confinement refusal reaches the MODEL as an error
//! tool result carrying the `[confinement]` marker, so it can adapt or surface a real
//! approval ask instead of silently retrying unconfined.

use std::sync::Arc;

use async_trait::async_trait;

/// Resource/behavior caps for one confined execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfinementPolicy {
    /// Per-process memory cap in bytes (`JOB_OBJECT_LIMIT_PROCESS_MEMORY`).
    pub process_memory_limit_bytes: Option<u64>,
    /// Whole-job memory cap in bytes (`JOB_OBJECT_LIMIT_JOB_MEMORY`).
    pub job_memory_limit_bytes: Option<u64>,
    /// Maximum live descendant processes (`ActiveProcessLimit`) — the "child cap".
    pub active_process_limit: Option<u32>,
    /// CPU rate hard cap as a whole percent, 1..=100 (`CPU_RATE_CONTROL`, percent*100 on wire).
    pub cpu_rate_percent: Option<u32>,
    /// Wall-clock limit; the whole tree is terminated when it elapses.
    pub wall_timeout_secs: u64,
}

impl Default for ConfinementPolicy {
    fn default() -> Self {
        Self {
            process_memory_limit_bytes: Some(512 * 1024 * 1024),
            job_memory_limit_bytes: Some(1024 * 1024 * 1024),
            active_process_limit: Some(64),
            cpu_rate_percent: None,
            wall_timeout_secs: 120,
        }
    }
}

/// The command to confine.
#[derive(Debug, Clone)]
pub struct ConfinedCommand {
    /// Executable to launch.
    pub program: String,
    /// Arguments passed verbatim.
    pub args: Vec<String>,
    /// Working directory for the child.
    pub cwd: std::path::PathBuf,
}

/// Successful confined execution result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfinedOutcome {
    /// Exit code, if the process exited normally.
    pub exit_code: Option<i32>,
    /// Captured stdout (lossy UTF-8).
    pub stdout: String,
    /// Captured stderr (lossy UTF-8).
    pub stderr: String,
    /// Backend that ran the command (diagnostics/details payload).
    pub backend: &'static str,
}

/// Why a confined run refused or failed BEFORE producing command output.
///
/// Distinct from command failure: consumers (bash tool) render these with a
/// `[confinement]` marker so the model can tell "sandbox blocked it" apart from
/// "the command failed" (DSH denial-signature rule).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxError {
    /// No backend exists for this platform/policy; running unconfined was refused.
    Unavailable(String),
    /// A backend exists but could not establish confinement (job creation, bwrap exec…).
    Setup(String),
    /// Wall-clock timeout hit; the whole job/tree was terminated.
    Timeout {
        /// The wall limit that elapsed, in seconds.
        seconds: u64,
    },
    /// The spawn itself failed under confinement setup.
    Spawn(String),
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxError::Unavailable(d) => write!(f, "no confinement backend available: {d}"),
            SandboxError::Setup(d) => write!(f, "confinement failed to establish: {d}"),
            SandboxError::Timeout { seconds } => {
                write!(f, "terminated after {seconds}s wall limit")
            }
            SandboxError::Spawn(d) => write!(f, "spawn failed under confinement: {d}"),
        }
    }
}

impl std::error::Error for SandboxError {}

/// An OS-confinement backend. Implementations must be fail-closed: if `available()`
/// is true but confinement cannot be established at run time, return
/// [`SandboxError::Setup`] — never fall back to unconfined execution.
#[async_trait]
pub trait Sandbox: Send + Sync {
    /// Backend name for diagnostics.
    fn name(&self) -> &'static str;

    /// Whether this platform can confine at all right now.
    fn available(&self) -> bool;

    /// Run one command confined under `policy`.
    async fn run(
        &self,
        cmd: ConfinedCommand,
        policy: &ConfinementPolicy,
    ) -> Result<ConfinedOutcome, SandboxError>;
}

/// Fail-closed placeholder used when no OS backend exists (and by tests).
pub struct NullSandbox;

#[async_trait]
impl Sandbox for NullSandbox {
    fn name(&self) -> &'static str {
        "null"
    }

    fn available(&self) -> bool {
        false
    }

    async fn run(
        &self,
        _cmd: ConfinedCommand,
        _policy: &ConfinementPolicy,
    ) -> Result<ConfinedOutcome, SandboxError> {
        Err(SandboxError::Unavailable(
            "null sandbox never runs commands; policy demands confinement".into(),
        ))
    }
}

/// Best confinement this platform offers, probed once.
///
/// Windows → Job Objects (always available). Unix → bubblewrap when installed on PATH,
/// else [`NullSandbox`]. Callers that demand confinement must treat
/// `!sandbox.available()` as refuse-to-run (fail closed).
pub fn best_available() -> Arc<dyn Sandbox> {
    #[cfg(windows)]
    {
        Arc::new(JobObjectSandbox::new())
    }
    #[cfg(not(windows))]
    {
        match which_bwrap() {
            Some(path) => Arc::new(BwrapSandbox::new(path)),
            None => Arc::new(NullSandbox),
        }
    }
}

/// Locate `bwrap` on PATH (Unix). Returns its path so the backend never depends on cwd.
#[cfg(not(windows))]
fn which_bwrap() -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join("bwrap"))
        .find(|p| p.is_file())
}

/// Build the bubblewrap argv for one confined command (pure; unit-tested everywhere).
///
/// Isolation set: all namespaces unshared, die-with-parent + new session so the tree
/// dies with us, `/` read-only, private dev/proc/tmpfs, and the workspace re-exposed
/// writable — DSH's 'workspace-write' mode, argv-shaped.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn bwrap_argv(
    bwrap_path: &std::path::Path,
    workspace: &std::path::Path,
    target: &[String],
) -> Vec<String> {
    let mut argv: Vec<String> = vec![
        bwrap_path.display().to_string(),
        "--unshare-all".into(),
        "--die-with-parent".into(),
        "--new-session".into(),
        "--ro-bind".into(),
        "/".into(),
        "/".into(),
        "--dev".into(),
        "/dev".into(),
        "--proc".into(),
        "/proc".into(),
        "--tmpfs".into(),
        "/tmp".into(),
        "--bind".into(),
        workspace.display().to_string(),
        workspace.display().to_string(),
    ];
    argv.extend(target.iter().cloned());
    argv
}

/// bubblewrap-backed sandbox (Unix only). Filesystem/namespace isolation; note that
/// bwrap does not enforce memory/CPU caps — those require cgroups (v1 limitation,
/// recorded in the M10 dev log and ADR-005 amendment).
#[cfg(unix)]
pub struct BwrapSandbox {
    bwrap_path: std::path::PathBuf,
}

#[cfg(unix)]
impl BwrapSandbox {
    /// Build around a located bwrap binary.
    pub fn new(path: std::path::PathBuf) -> Self {
        Self { bwrap_path: path }
    }
}

#[cfg(unix)]
#[async_trait::async_trait]
impl Sandbox for BwrapSandbox {
    fn name(&self) -> &'static str {
        "bwrap"
    }

    fn available(&self) -> bool {
        self.bwrap_path.is_file()
    }

    async fn run(
        &self,
        cmd: ConfinedCommand,
        _policy: &ConfinementPolicy,
    ) -> Result<ConfinedOutcome, SandboxError> {
        let mut target = vec![cmd.program.clone()];
        target.extend(cmd.args.iter().cloned());
        let argv = bwrap_argv(&self.bwrap_path, &cmd.cwd, &target);

        let mut command = tokio::process::Command::new(&argv[0]);
        command.args(&argv[1..]).current_dir(&cmd.cwd);
        let output = command
            .output()
            .await
            .map_err(|e| SandboxError::Setup(format!("bwrap exec: {e}")))?;
        Ok(ConfinedOutcome {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            backend: self.name(),
        })
    }
}

#[cfg(windows)]
pub mod jobobject;

#[cfg(windows)]
pub use jobobject::JobObjectSandbox;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_sandbox_is_never_available_and_fails_closed() {
        assert!(!NullSandbox.available());
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let outcome = rt.block_on(NullSandbox.run(
            ConfinedCommand {
                program: "echo".into(),
                args: vec!["hi".into()],
                cwd: std::env::temp_dir(),
            },
            &ConfinementPolicy::default(),
        ));
        assert!(matches!(outcome, Err(SandboxError::Unavailable(_))));
    }

    #[test]
    fn default_policy_caps_are_sane() {
        let p = ConfinementPolicy::default();
        assert_eq!(p.process_memory_limit_bytes, Some(512 * 1024 * 1024));
        assert!(p.active_process_limit.unwrap_or(0) > 0);
    }

    #[cfg(unix)]
    #[test]
    fn bwrap_argv_binds_workspace_writable_and_unshares_all() {
        let argv = bwrap_argv(
            std::path::Path::new("/usr/bin/bwrap"),
            std::path::Path::new("/work"),
            &["sh".into(), "-c".into(), ":".into()],
        );
        assert_eq!(argv[0], "/usr/bin/bwrap");
        assert_eq!(argv.last().map(String::as_str), Some(":"));
        assert!(argv.contains(&"--unshare-all".to_string()));
        // Exactly one writable --bind pair, pointing at the workspace.
        let binds: Vec<_> = argv.windows(3).filter(|w| w[0] == "--bind").collect();
        assert_eq!(binds.len(), 1);
        assert_eq!(binds[0][1], "/work");
    }

    #[cfg(windows)]
    #[test]
    fn bwrap_argv_builder_is_platform_independent() {
        // The argv builder is pure and compiled on Windows too so its shape stays
        // regression-tested without a Unix host.
        let argv = bwrap_argv(
            std::path::Path::new("/usr/bin/bwrap"),
            std::path::Path::new("/work"),
            &["sh".into()],
        );
        assert_eq!(argv[0], "/usr/bin/bwrap");
        assert_eq!(*argv.last().unwrap(), "sh");
    }
}
