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

use crate::escalation::SandboxMode;
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
    /// How much this particular command may do (port plan item 15). Per-call truth: the
    /// session's mode is the default a caller starts from, and a widening only ever
    /// arrives here after an approval (see `escalation`).
    pub mode: SandboxMode,
}

impl Default for ConfinedCommand {
    /// A command confined to the workspace, which is what every existing caller means.
    fn default() -> Self {
        Self {
            program: String::new(),
            args: Vec::new(),
            cwd: std::path::PathBuf::new(),
            mode: SandboxMode::WorkspaceWrite,
        }
    }
}

/// How completely the boundary held (M77; DSH `packages/sandbox/sandbox/src/index.ts:59`).
///
/// `Sandbox::run` used to return an outcome that said nothing about the *quality* of the
/// confinement, which left the platform caveat (Windows job objects bound resources but not the
/// filesystem) as a paragraph in a report instead of a fact a caller can check. A caller that
/// needs an absolute boundary can now reject anything but [`Enforcement::Full`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Enforcement {
    /// Every dimension the caller must assume was confined. On this platform, for this policy.
    Full,
    /// Something the caller would reasonably assume is **not** confined, named explicitly.
    Partial {
        /// Dimension names, chosen so a report can read them: `"filesystem"`, `"network"`,
        /// `"memory"`, `"process tree"`, `"wall clock"`.
        missing: Vec<&'static str>,
    },
}

impl Enforcement {
    /// Build from a list of dimensions that could not be confined.
    ///
    /// An empty list is [`Enforcement::Full`] by construction, so a backend cannot accidentally
    /// report `Partial { missing: [] }` — an unanswerable state.
    pub fn from_missing(missing: Vec<&'static str>) -> Self {
        if missing.is_empty() {
            Enforcement::Full
        } else {
            Enforcement::Partial { missing }
        }
    }

    /// True when nothing is missing.
    pub fn is_full(&self) -> bool {
        matches!(self, Enforcement::Full)
    }

    /// The dimensions that are not confined, empty when full.
    pub fn missing(&self) -> &[&'static str] {
        match self {
            Enforcement::Full => &[],
            Enforcement::Partial { missing } => missing,
        }
    }

    /// One line for `details`, a notice or `epsilon doctor`.
    pub fn describe(&self) -> String {
        match self {
            Enforcement::Full => "full".to_string(),
            Enforcement::Partial { missing } => {
                format!("partial — not confined: {}", missing.join(", "))
            }
        }
    }
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
    /// How completely the boundary the policy asked for was applied (M77).
    pub enforcement: Enforcement,
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

    /// What this backend can confine at all, independent of any policy (M77).
    ///
    /// The answer is a property of the *platform*: Windows job objects bound resources but can
    /// not restrict what a process reads or writes, and saying so here is what turns the caveat
    /// into something a caller (or `epsilon doctor`) can act on. A backend must not claim a
    /// dimension it does not enforce — an over-claim is worse than no field at all.
    fn capabilities(&self) -> Enforcement;

    /// Run one command confined under `policy`.
    ///
    /// `on_output`, when present, receives incremental stdout/stderr chunks as
    /// they arrive so a long build is visible while it runs instead of arriving
    /// as one silent wall at exit (pi streams the same way). Backends must treat
    /// the sink as best-effort: a slow or panicking consumer may never affect the
    /// command's own exit status, and the returned [`ConfinedOutcome`] always
    /// carries the COMPLETE output regardless of what was streamed.
    async fn run(
        &self,
        cmd: ConfinedCommand,
        policy: &ConfinementPolicy,
        on_output: Option<OutputSink>,
    ) -> Result<ConfinedOutcome, SandboxError>;
}

/// Incremental output callback for a running confined command (see
/// [`Sandbox::run`]). Chunks are lossy-UTF-8 slices in arrival order; callers
/// that render them should keep only a bounded tail.
pub type OutputSink = Arc<dyn Fn(&str) + Send + Sync>;

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

    fn capabilities(&self) -> Enforcement {
        // It refuses every command, so it confines nothing — and must never be read as a
        // boundary that held.
        Enforcement::from_missing(vec!["filesystem", "network", "memory", "process tree"])
    }

    async fn run(
        &self,
        _cmd: ConfinedCommand,
        _policy: &ConfinementPolicy,
        _on_output: Option<OutputSink>,
    ) -> Result<ConfinedOutcome, SandboxError> {
        Err(SandboxError::Unavailable(
            "null sandbox never runs commands; policy demands confinement".into(),
        ))
    }
}

/// Drain one child pipe into a buffer, forwarding each chunk to `sink` as it
/// arrives. Shared by both backends so streaming behaves identically on Windows
/// (Job Objects) and Unix (bubblewrap); the returned bytes are always complete
/// even when the sink is dropped or never set.
pub async fn drain_pipe(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    sink: Option<OutputSink>,
) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut out: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if let Some(s) = &sink {
                    s(&String::from_utf8_lossy(&buf[..n]));
                }
            }
            // A read error ends the drain; whatever arrived is still the truth.
            Err(_) => break,
        }
    }
    out
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
    mode: SandboxMode,
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
    ];
    // The workspace mount is what the mode buys: read-only mounts it read-only, so a command
    // that tries to write fails inside the sandbox rather than being policed afterwards.
    argv.push(
        match mode {
            SandboxMode::ReadOnly => "--ro-bind",
            _ => "--bind",
        }
        .into(),
    );
    argv.push(workspace.display().to_string());
    argv.push(workspace.display().to_string());
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

    /// Namespaces cover the filesystem and the network; the resource limits do not.
    ///
    /// bwrap enforces nothing about memory, CPU or process count — those need cgroups — and this
    /// backend does not apply the policy's wall-clock limit either (the job-object backend does).
    /// Declaring all three is the difference between a caveat in a comment and a fact a caller
    /// can refuse to proceed on.
    fn capabilities(&self) -> Enforcement {
        Enforcement::from_missing(vec!["memory", "process tree", "wall clock"])
    }

    async fn run(
        &self,
        cmd: ConfinedCommand,
        policy: &ConfinementPolicy,
        on_output: Option<OutputSink>,
    ) -> Result<ConfinedOutcome, SandboxError> {
        let mut target = vec![cmd.program.clone()];
        target.extend(cmd.args.iter().cloned());
        if cmd.mode == SandboxMode::DangerFullAccess {
            // No confinement is not a confinement: a caller that wants it must not call the
            // sandbox at all. Failing closed keeps a caller bug loud.
            return Err(SandboxError::Setup(
                "danger-full-access is not a confined mode; run the command directly \
                 instead of through a sandbox"
                    .into(),
            ));
        }
        let argv = bwrap_argv(&self.bwrap_path, &cmd.cwd, cmd.mode, &target);

        let mut command = tokio::process::Command::new(&argv[0]);
        command.args(&argv[1..]).current_dir(&cmd.cwd);
        // Bwrap does not clear the environment it hands the sandboxed process.
        crate::env::scrub_secret_env(&mut command);
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());

        let mut child = command
            .spawn()
            .map_err(|e| SandboxError::Setup(format!("bwrap exec: {e}")))?;
        // Cancellation drops this future, and a dropped future must not leave a sandboxed
        // tree running unsupervised. The unconfined path already sets this
        // (`shell.rs`); a confined run is the one place it matters more, because the tree it
        // leaks is isolated, long-lived, and invisible to the user who cancelled.
        child.kill_on_drop(true);
        let stdout_pipe = child
            .stdout
            .take()
            .ok_or_else(|| SandboxError::Setup("stdout pipe unavailable".into()))?;
        let stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| SandboxError::Setup("stderr pipe unavailable".into()))?;

        // Drain both pipes concurrently: a child that fills a pipe buffer while
        // we wait on the other stream would deadlock.
        let (out_sink, err_sink) = (on_output.clone(), on_output);
        let out_task = tokio::spawn(async move { drain_pipe(stdout_pipe, out_sink).await });
        let err_task = tokio::spawn(async move { drain_pipe(stderr_pipe, err_sink).await });

        // F11: one deadline covers the *whole* run — waiting for the child AND collecting the
        // pipes. Bounding only the wait would still hang on a descendant that outlives the
        // direct child while holding stdout open, which is the case that turns a promised
        // timeout into an indefinite wait. `--die-with-parent` is already in the argv, so the
        // namespace goes with the process and a held pipe cannot outlive the kill.
        let seconds = policy.wall_timeout_secs.max(1);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(seconds);
        let whole = async {
            let status = child
                .wait()
                .await
                .map_err(|e| SandboxError::Spawn(format!("bwrap wait: {e}")))?;
            // Both readers finish on EOF once the tree is gone; awaiting them here is what
            // makes the collected output complete rather than truncated at the deadline.
            let stdout = out_task.await.unwrap_or_default();
            let stderr = err_task.await.unwrap_or_default();
            Ok::<_, SandboxError>((status, stdout, stderr))
        };

        match tokio::time::timeout_at(deadline, whole).await {
            Ok(Ok((status, stdout, stderr))) => Ok(ConfinedOutcome {
                exit_code: status.code(),
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
                backend: self.name(),
                // The backend's envelope, not a per-field audit of this policy: what it cannot
                // confine at all is exactly what a caller has to know (M77).
                enforcement: self.capabilities(),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                // Elapsed. Kill the tree, and deliberately do NOT await the drains: a pipe
                // still held is precisely why the clock ran out, so waiting on it here would
                // move the hang we are fixing to the line after it. The tasks are detached and
                // end when the namespace goes with the killed process.
                let _ = child.start_kill();
                Err(SandboxError::Timeout { seconds })
            }
        }
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
                ..Default::default()
            },
            &ConfinementPolicy::default(),
            None,
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
            SandboxMode::WorkspaceWrite,
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
            SandboxMode::WorkspaceWrite,
            &["sh".into()],
        );
        assert_eq!(argv[0], "/usr/bin/bwrap");
        assert_eq!(*argv.last().unwrap(), "sh");
    }

    /// The read-only rung of the mode ladder, as the argv it produces. Pure, so it is
    /// checked on Windows too — the flag is the whole mechanism on Unix.
    #[test]
    fn read_only_mounts_the_workspace_read_only() {
        let sh = ["sh".into()];
        let ro = bwrap_argv(
            std::path::Path::new("/usr/bin/bwrap"),
            std::path::Path::new("/work"),
            SandboxMode::ReadOnly,
            &sh,
        );
        let rw = bwrap_argv(
            std::path::Path::new("/usr/bin/bwrap"),
            std::path::Path::new("/work"),
            SandboxMode::WorkspaceWrite,
            &sh,
        );
        // Exactly one workspace mount, and its shape is the mode.
        assert_eq!(rw.windows(3).filter(|w| w[0] == "--bind").count(), 1);
        assert_eq!(ro.windows(3).filter(|w| w[0] == "--bind").count(), 0);
        let ro_mounts: Vec<_> = ro.windows(3).filter(|w| w[0] == "--ro-bind").collect();
        // Two read-only binds: the root filesystem, and the workspace.
        assert_eq!(ro_mounts.len(), 2);
        assert!(
            ro_mounts.iter().any(|w| w[1] == "/work" && w[2] == "/work"),
            "the workspace is the read-only bind that matters: {ro_mounts:?}"
        );
        assert!(!ro.contains(&"--bind".to_string()));
    }
    /// "No confinement" is not a confinement: a backend handed that mode must refuse, not
    /// run the command with its resource caps and call it full access.
    #[tokio::test]
    async fn a_backend_refuses_to_pretend_that_danger_full_access_is_a_boundary() {
        let cmd = ConfinedCommand {
            program: "sh".into(),
            mode: SandboxMode::DangerFullAccess,
            ..Default::default()
        };
        let err = NullSandbox
            .run(cmd, &ConfinementPolicy::default(), None)
            .await
            .expect_err("no boundary means no run");
        // The null backend's own refusal is `Unavailable`; what matters here is that nothing
        // ran and the caller can tell why.
        assert!(
            matches!(err, SandboxError::Unavailable(_) | SandboxError::Setup(_)),
            "{err:?}"
        );
    }

    #[test]
    fn the_null_backend_confines_nothing_and_says_so() {
        // It never runs a command, so it must never be read as a boundary that held.
        let caps = NullSandbox.capabilities();
        assert!(!caps.is_full());
        for dimension in ["filesystem", "network", "process tree"] {
            assert!(
                caps.missing().contains(&dimension),
                "a backend that refuses everything cannot claim {dimension}: {caps:?}"
            );
        }
        assert!(
            caps.describe().starts_with("partial"),
            "{}",
            caps.describe()
        );
    }

    /// The Windows caveat as a checkable fact: job objects bound resources, not content.
    #[cfg(windows)]
    #[test]
    fn the_job_object_backend_does_not_claim_to_confine_content() {
        let sandbox = JobObjectSandbox::new();
        let caps = sandbox.capabilities();
        assert!(
            !caps.is_full(),
            "this platform cannot confine the filesystem"
        );
        assert!(caps.missing().contains(&"filesystem"), "{caps:?}");
        assert!(caps.missing().contains(&"network"), "{caps:?}");
        // What it *does* enforce must not be listed as missing.
        let missing = caps.missing();
        for dimension in ["memory", "process tree", "wall clock"] {
            assert!(
                !missing.contains(&dimension),
                "{dimension} IS enforced by a job object: {missing:?}"
            );
        }
    }

    /// Every backend's answer must be internally consistent, whoever it is.
    #[test]
    fn whatever_backend_this_platform_picks_states_a_coherent_envelope() {
        let sandbox = best_available();
        let caps = sandbox.capabilities();
        if sandbox.available() {
            // An available backend that claims nothing is missing must be able to confine; one
            // that cannot confine content must say so rather than staying silent.
            assert!(caps.is_full() || !caps.missing().is_empty());
        } else {
            assert!(
                !caps.is_full(),
                "an unavailable backend must not describe a full boundary"
            );
        }
    }
}
