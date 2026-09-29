//! Windows Job Object confinement — the ONLY unsafe user in this workspace.
//!
//! AGENTS.md exception, granted per-crate with justification: Job Objects are raw Win32
//! FFI and no maintained safe wrapper covers memory + active-process + CPU-rate caps
//! together. Contained discipline (see Cargo.toml lints):
//! - every `unsafe` block carries a SAFETY comment (clippy::undocumented_unsafe_blocks);
//! - no new unsafe may appear outside this module;
//! - the job handle is owned by [`JobGuard`], which terminates the whole tree on drop
//!   (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` backs this up at the kernel level).
//!
//! Semantics (DSH armor, ADR-005): the shell child AND every descendant stay inside the
//! job for the call's lifetime; silent breakaway is refused because we never grant
//! `JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK`; on timeout or drop the tree dies. Limits are
//! applied BEFORE assignment so no command code runs under uncapped settings.

use std::time::Duration;
use tokio::process::Command;
use winapi::um::winnt::HANDLE;

/// How long the pipe readers may take *after* the child has exited (F11).
///
/// A separate grace rather than the remaining wall budget, for the same reason as the
/// unconfined shell in M148: the wait already consumed that budget, so a command that used
/// all of it would get no time at all to report what it produced. Five seconds covers a
/// child flushing a large buffer through a pipe that a dying process is still closing.
const DRAIN_GRACE: Duration = Duration::from_secs(5);

/// Await a pipe reader under `grace`, reporting whether it finished rather than only
/// what it managed to read.
///
/// Returns `(bytes, finished)`. A reader that is dropped mid-await is detached, not
/// cancelled, so it keeps draining into the sink in the background and simply stops
/// counting toward this call.
async fn collect_within(
    mut task: tokio::task::JoinHandle<Vec<u8>>,
    buf: crate::sandbox::SharedBuf,
    grace: Duration,
) -> (Vec<u8>, bool) {
    // `&mut task`, not `task`: on expiry the handle is dropped rather than moved, so the reader
    // is detached and keeps draining in the background instead of being cancelled mid-read.
    let finished = tokio::time::timeout(grace, &mut task).await.is_ok();
    (crate::sandbox::snapshot(&buf), finished)
}

use super::{
    ConfinedCommand, ConfinedOutcome, ConfinementPolicy, Enforcement, Sandbox, SandboxError,
};

mod ffi {
    use std::mem::zeroed;

    use winapi::ctypes::c_void as ffi_void;
    #[cfg(test)]
    use winapi::um::jobapi2::QueryInformationJobObject;
    use winapi::um::jobapi2::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject, TerminateJobObject,
    };
    use winapi::um::winnt::{
        HANDLE, JOB_OBJECT_CPU_RATE_CONTROL_ENABLE, JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
        JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_JOB_MEMORY,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
        JOBOBJECT_CPU_RATE_CONTROL_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectCpuRateControlInformation, JobObjectExtendedLimitInformation,
    };

    /// Owned job handle. Terminate-on-drop makes cleanup deterministic even when the
    /// await that owns the child was cancelled mid-flight.
    pub(super) struct JobGuard(pub(super) HANDLE);

    // SAFETY: a Win32 handle value is trivially movable between threads and we only ever
    // pass it to Job Object FFI; no aliasing interior state exists.
    unsafe impl Send for JobGuard {}

    impl Drop for JobGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: handle is valid and owned exclusively by this guard.
                unsafe { TerminateJobObject(self.0, 1) };
                // SAFETY: as above; this is the last use.
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    pub(super) fn create_job() -> Result<JobGuard, String> {
        // SAFETY: null attributes/name create an anonymous job; ownership transfers to JobGuard.
        let handle = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
        if handle.is_null() {
            return Err("CreateJobObjectW failed".into());
        }
        Ok(JobGuard(handle))
    }

    /// Apply policy limits to the still-empty job.
    pub(super) fn apply_limits(
        job: HANDLE,
        policy: &super::super::ConfinementPolicy,
    ) -> Result<(), String> {
        // SAFETY: zeroed is valid for this all-numbers Win32 struct (no refs/unions).
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        let mut flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if let Some(per_process) = policy.process_memory_limit_bytes {
            limits.ProcessMemoryLimit = per_process as usize;
            flags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        }
        if let Some(per_job) = policy.job_memory_limit_bytes {
            limits.JobMemoryLimit = per_job as usize;
            flags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
        }
        if let Some(active) = policy.active_process_limit {
            limits.BasicLimitInformation.ActiveProcessLimit = active;
            flags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        }
        limits.BasicLimitInformation.LimitFlags = flags;
        // SAFETY: correct info class + buffer size for an owned, valid handle.
        let ok = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _ as *mut ffi_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err("SetInformationJobObject(extended limits) failed".into());
        }

        if let Some(percent) = policy.cpu_rate_percent {
            let rate = percent.clamp(1, 100) * 100; // wire unit: parts per 10_000
            // SAFETY: same all-numbers layout; union is initialized below before use.
            let mut cpu: JOBOBJECT_CPU_RATE_CONTROL_INFORMATION = unsafe { zeroed() };
            cpu.ControlFlags =
                JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP;
            // SAFETY: union write into a zeroed, owned stack struct.
            unsafe { *cpu.u.CpuRate_mut() = rate };
            // SAFETY: correct info class + buffer size for an owned, valid handle.
            let ok = unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectCpuRateControlInformation,
                    &cpu as *const _ as *const _ as *mut ffi_void,
                    std::mem::size_of::<JOBOBJECT_CPU_RATE_CONTROL_INFORMATION>() as u32,
                )
            };
            if ok == 0 {
                return Err("SetInformationJobObject(cpu rate) failed".into());
            }
        }
        Ok(())
    }

    pub(super) fn assign(child_process_handle: HANDLE, job: HANDLE) -> Result<(), String> {
        // SAFETY: both handles are valid; child has not exited before this call because
        // we hold it via tokio's Child and assign immediately after spawn.
        let ok = unsafe { AssignProcessToJobObject(job, child_process_handle) };
        if ok == 0 {
            return Err("AssignProcessToJobObject failed".into());
        }
        Ok(())
    }

    /// Read back effective extended limits (tests + diagnostics).
    #[cfg(test)]
    pub(super) fn read_extended_limits(
        job: HANDLE,
    ) -> Result<JOBOBJECT_EXTENDED_LIMIT_INFORMATION, String> {
        // SAFETY: zeroed is valid for this all-numbers Win32 struct; overwritten by query.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        let mut ret_len: u32 = 0;
        // SAFETY: caller-provided handle + correctly sized out-buffer.
        let ok = unsafe {
            QueryInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &mut info as *mut _ as *mut ffi_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                &mut ret_len,
            )
        };
        if ok == 0 {
            return Err("QueryInformationJobObject failed".into());
        }
        Ok(info)
    }

    /// Terminate a job without dropping the guard that owns it.
    ///
    /// Not a test hook: the **production** timeout path calls this to kill a
    /// process tree that overran, and the guard's own `Drop` then finds nothing
    /// left to do. The comment here used to call it a "test/diagnostic hook",
    /// which was false of the tree and would have led the next reader to delete
    /// the one call site that matters.
    pub(super) fn terminate(job: HANDLE) {
        // SAFETY: valid owned handle.
        unsafe { TerminateJobObject(job, 1) };
    }

    /// Expose raw handle for assignment from the parent module.
    pub(super) fn raw(job: &JobGuard) -> HANDLE {
        job.0
    }

    use winapi::um::handleapi::CloseHandle;
}

/// Confine one shell invocation inside a fresh Job Object per call (DSH: policy rides
/// each call; a per-call job also means per-call teardown).
pub struct JobObjectSandbox;

impl JobObjectSandbox {
    /// Build the backend.
    pub fn new() -> Self {
        Self
    }
}

impl Default for JobObjectSandbox {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Sandbox for JobObjectSandbox {
    fn name(&self) -> &'static str {
        "windows-job-object"
    }

    fn available(&self) -> bool {
        true
    }

    /// Resources yes, content no: this is the Windows caveat as a value (M77, D-19).
    ///
    /// A job object bounds memory, process count and CPU, and it kills the whole tree on timeout
    /// or on drop — but nothing in it restricts what those processes may read or write, so a
    /// caller that needs a filesystem boundary must reject this backend rather than trust the
    /// word "confined".
    fn capabilities(&self) -> Enforcement {
        Enforcement::from_missing(vec!["filesystem", "network"])
    }

    async fn run(
        &self,
        cmd: ConfinedCommand,
        policy: &ConfinementPolicy,
        on_output: Option<crate::sandbox::OutputSink>,
    ) -> Result<ConfinedOutcome, SandboxError> {
        if cmd.mode == crate::escalation::SandboxMode::DangerFullAccess {
            // No confinement is not a confinement: a caller that wants it must not call the
            // sandbox at all. Failing closed keeps a caller bug loud.
            return Err(SandboxError::Setup(
                "danger-full-access is not a confined mode; run the command directly instead of \
                 through a sandbox"
                    .into(),
            ));
        }
        let job = ffi::create_job().map_err(SandboxError::Setup)?;
        ffi::apply_limits(ffi::raw(&job), policy).map_err(SandboxError::Setup)?;

        let mut command = Command::new(&cmd.program);
        command.args(&cmd.args).current_dir(&cmd.cwd);
        // stdin closed, and hidden window on Windows — the same two properties
        // `epsilon_tools::shell::ChildStdio` sets on the unconfined path, for the
        // same reason. This path is the *default* one, so leaving it inheriting
        // meant the fix upstream did not reach the runs that matter most: a
        // confined child reading the TUI's console stdin takes the user's
        // keystrokes, and a console subsystem child can raise its own window over
        // the TUI. See `audit/evidence/m197-windows-stdin-inheritance.md`.
        command.stdin(std::process::Stdio::null());
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());
        command.kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        // DSH subprocess rule: a confined child still inherits our environment,
        // so a credential-shaped variable would be visible to `env` inside the
        // sandbox. Confinement limits where a process may go, not what it can
        // read — the scrub is what stops a key reaching a tool result.
        crate::env::scrub_secret_env(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| SandboxError::Spawn(format!("{}: {e}", cmd.program)))?;

        // Assign immediately after spawn; the initial process cannot have produced
        // confined-relevant descendants in this window, and breakaway is refused.
        // tokio hands out `*mut std::ffi::c_void`; winapi wants its own c_void flavor.
        let process_handle: HANDLE = child
            .raw_handle()
            .ok_or_else(|| SandboxError::Setup("child process handle unavailable".into()))?
            .cast();
        ffi::assign(process_handle, ffi::raw(&job)).map_err(SandboxError::Setup)?;

        // Drain pipes concurrently so a chatty child can never block on full buffers,
        // forwarding each chunk to the live sink when one was supplied (pi streams
        // partial tool output so a long build is visible while it runs).
        let stdout_pipe = child
            .stdout
            .take()
            .ok_or_else(|| SandboxError::Setup("stdout pipe unavailable".into()))?;
        let stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| SandboxError::Setup("stderr pipe unavailable".into()))?;
        let (out_sink, err_sink) = (on_output.clone(), on_output);
        // Shared buffers: on a grace expiry these are what lets the partial read survive.
        let stdout_buf = crate::sandbox::new_shared_buf();
        let stderr_buf = crate::sandbox::new_shared_buf();
        let (sb, eb) = (stdout_buf.clone(), stderr_buf.clone());
        let stdout_task =
            tokio::spawn(
                async move { crate::sandbox::drain_pipe(stdout_pipe, out_sink, sb).await },
            );
        let stderr_task =
            tokio::spawn(
                async move { crate::sandbox::drain_pipe(stderr_pipe, err_sink, eb).await },
            );

        let seconds = policy.wall_timeout_secs.max(1);
        let wait = child.wait();
        let waited = tokio::time::timeout(Duration::from_secs(seconds), wait).await;

        match waited {
            Ok(Ok(status)) => {
                // F11, third path. The `timeout` above bounds `child.wait()` and nothing else, so
                // these two bare awaits are an unbounded wait in disguise: the job object kills
                // the processes *assigned to it*, and anything holding these pipes that is not in
                // the job — a detached grandchild, a breakaway — keeps them open, and this call
                // waits on it forever with a command that has already exited.
                //
                // Same shape as M147's bwrap fix and M148's unconfined fix: one grace covering
                // both readers, awaited concurrently, and the fact that the grace expired is
                // reported rather than hidden. A bounded wait that silently returns short output
                // is a different defect from an unbounded one, and is the one left here.
                let (out, err) = tokio::join!(
                    collect_within(stdout_task, stdout_buf, DRAIN_GRACE),
                    collect_within(stderr_task, stderr_buf, DRAIN_GRACE),
                );
                let (stdout_bytes, stdout_done) = out;
                let (stderr_bytes, stderr_done) = err;
                Ok(ConfinedOutcome {
                    exit_code: status.code(),
                    stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
                    backend: self.name(),
                    enforcement: self.capabilities(),
                    output_complete: stdout_done && stderr_done,
                })
            }
            Ok(Err(e)) => Err(SandboxError::Spawn(format!("wait failed: {e}"))),
            Err(_) => {
                // Elapsed: terminate the tree; pipe readers finish on EOF after kill.
                ffi::terminate(ffi::raw(&job));
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                Err(SandboxError::Timeout { seconds })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winapi::um::winnt::JOB_OBJECT_LIMIT_PROCESS_MEMORY;

    fn powershell(command: &str) -> ConfinedCommand {
        ConfinedCommand {
            program: "powershell".into(),
            args: vec![
                "-NoLogo".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                command.into(),
            ],
            cwd: std::env::temp_dir(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn the_job_object_backend_refuses_danger_full_access() {
        // It cannot express "no boundary": job objects only ever *add* limits.
        let mut cmd = powershell("echo hi");
        cmd.mode = crate::escalation::SandboxMode::DangerFullAccess;
        let err = JobObjectSandbox::new()
            .run(cmd, &short_policy(), None)
            .await
            .expect_err("must not run as if unconfined");
        match err {
            SandboxError::Setup(message) => {
                assert!(message.contains("danger-full-access"), "{message}");
            }
            other => panic!("expected a setup refusal, got {other:?}"),
        }
    }

    fn short_policy() -> ConfinementPolicy {
        ConfinementPolicy {
            process_memory_limit_bytes: Some(256 * 1024 * 1024),
            job_memory_limit_bytes: Some(256 * 1024 * 1024),
            active_process_limit: Some(16),
            cpu_rate_percent: None,
            wall_timeout_secs: 120,
        }
    }

    #[tokio::test]
    async fn confined_command_runs_with_limits_applied() {
        let sb = JobObjectSandbox::new();
        assert!(sb.available());

        // Run through the public seam AND verify the caps stuck by creating a second
        // job with identical policy and querying it (querying needs the raw handle,
        // which lives inside the backend; the behavioral assertions below come from
        // the outcome itself).
        let outcome = sb
            .run(powershell("Write-Output job-ok"), &short_policy(), None)
            .await
            .expect("confined run succeeds");
        assert_eq!(outcome.backend, "windows-job-object");
        assert_eq!(outcome.exit_code, Some(0));

        // Limits actually applied: spawn a job manually with the same policy path and
        // query it through the ffi seam.
        let job = ffi::create_job().expect("job");
        ffi::apply_limits(ffi::raw(&job), &short_policy()).expect("limits");
        let info = ffi::read_extended_limits(ffi::raw(&job)).expect("query");
        assert_ne!(
            info.BasicLimitInformation.LimitFlags & JOB_OBJECT_LIMIT_PROCESS_MEMORY,
            0,
            "process memory cap flag must be set"
        );
        assert_eq!(
            info.ProcessMemoryLimit,
            256 * 1024 * 1024,
            "memory cap must match policy"
        );
        assert_eq!(info.BasicLimitInformation.ActiveProcessLimit, 16);
    }

    #[tokio::test]
    async fn timeout_kills_tree_and_reports_timeout_error() {
        let sb = JobObjectSandbox::new();
        let policy = ConfinementPolicy {
            wall_timeout_secs: 2,
            ..short_policy()
        };
        let err = sb
            .run(powershell("Start-Sleep -Seconds 30"), &policy, None)
            .await
            .expect_err("sleeping command must time out");
        assert!(matches!(err, SandboxError::Timeout { seconds: 2 }), "{err}");
    }

    /// Kill-on-close backstop: a grandchild spawned inside the job must die with it.
    /// Bounded polling keeps the suite deterministic; the kernel flag makes the
    /// guarantee itself unconditional.
    #[tokio::test]
    async fn grandchildren_die_with_the_job() {
        use tokio::process::Command;

        // Grandchild lingers; the confined parent relays its PID via a temp file
        // (PowerShell buffers redirected stdout, so files are the reliable channel).
        let sb = JobObjectSandbox::new();
        let pid_file = std::env::temp_dir().join(format!("eps_gc_{}.pid", std::process::id()));
        let _ = std::fs::remove_file(&pid_file);
        let relay = format!(
            "$p = Start-Process powershell -WindowStyle Hidden -PassThru -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 60'; \
             Set-Content -LiteralPath '{}' -Value $p.Id; \
             Start-Sleep -Seconds 8",
            pid_file.display()
        );
        let confined = powershell(&relay);
        let policy = short_policy();

        let job = ffi::create_job().expect("job");
        ffi::apply_limits(ffi::raw(&job), &policy).expect("limits");
        let mut command = Command::new(&confined.program);
        command.args(&confined.args).current_dir(&confined.cwd);
        command.kill_on_drop(true);
        let child = command.spawn().expect("spawn");
        ffi::assign(child.raw_handle().expect("handle").cast(), ffi::raw(&job)).expect("assign");

        // Bounded wait for the PID file to appear.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let pid_line = loop {
            match std::fs::read_to_string(&pid_file) {
                Ok(text) if !text.trim().is_empty() => break text.trim().to_string(),
                _ => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "grandchild pid file never appeared"
                    );
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        };
        assert!(
            pid_line.chars().all(|c| c.is_ascii_digit()),
            "relayed pid must be numeric: {pid_line}"
        );

        // Tear the job down; the grandchild must vanish shortly after.
        drop(child); // kill_on_drop ends the relay so the suite stays fast
        drop(job);
        tokio::time::sleep(Duration::from_millis(1500)).await;

        let probe = Command::new("powershell")
            .args([
                "-NoLogo",
                "-NonInteractive",
                "-Command",
                &format!(
                    "if (Get-Process -Id {pid_line} -ErrorAction SilentlyContinue) {{ 'alive' }} else {{ 'dead' }}"
                ),
            ])
            .output()
            .await
            .expect("probe spawn");
        let verdict = String::from_utf8_lossy(&probe.stdout).into_owned();
        assert!(
            verdict.contains("dead"),
            "grandchild {pid_line} survived job teardown: {verdict}"
        );
        let _ = sb.name();
        let _ = std::fs::remove_file(&pid_file);
    }
}
