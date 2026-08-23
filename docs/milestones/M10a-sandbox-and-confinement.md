# M10a — Sandbox seam, Windows Job Objects, event-correlation fix

Date: 2026-08-23 · Status: Layer 2 (OS confinement) shipped; ACL runner deferred with
rationale (ADR-005 amendment). M10b (release packaging) follows.

## What was built

- **`epsilon_safety::Sandbox` trait** (ADR-005's specified seam): `name/available/run`
  with policy riding each call (`ConfinementPolicy`: per-process + whole-job memory,
  active-process child cap, CPU-rate hard cap, wall limit). Fail-closed semantics ported
  from DSH (`packages/sandbox`): a demanded-but-unavailable backend REFUSES to run.
- **`JobObjectSandbox`** (Windows): fresh Job Object per call — limits set before the
  child is assigned, `KILL_ON_JOB_CLOSE` + explicit terminate-on-drop kill ladder,
  breakaway refused (never grant SILENT_BREAKAWAY_OK), piped stdout/stderr drained
  concurrently so chatty children cannot deadlock on full buffers. Caps verified via
  `QueryInformationJobObject`; timeout and grandchild-survival behavior verified live.
- **`BwrapSandbox`** (Unix) + platform-independent argv builder (unit-tested on Windows
  too). Documented limitation: bwrap isolates filesystems/namespaces; memory/CPU caps
  need cgroups — out of v1 scope, recorded here and in ADR-005.
- **`Confinement` choice** in epsilon-tools (`Auto | Off | Custom`): `Auto` dogfoods the
  job path everywhere — every existing bash test now runs inside Job Objects. `Off` is an
  explicit opt-out that records `"confinement": "off"` in tool details for auditability.
  CLI flag: `--sandbox auto|off` (off prints a banner warning).
- **Denial signatures**: sandbox refusals surface as `[confinement]`-marked error tool
  results so the model distinguishes "blocked by confinement" from "command failed"
  (DSH rule), instead of failing the run.
- **Permission-event correlation fix** (M6-era gap): exec.rs emitted empty
  `request_id`s. Interactive asks now emit `PermissionRequested{req}` +
  `PermissionDecided{request_id}` as a correlated pair; doom-loop decided events carry
  their ids too. Caveat documented: the pair lands after resolution because the gate owns
  ask timing; live notification stays the approval channel's job.

## The AGENTS.md unsafe exception

`unsafe_code = "forbid"` is un-overridable in Rust, so epsilon-safety's local lints
declare `unsafe_code = "allow"` plus `undocumented_unsafe_blocks = "warn"` — every unsafe
block carries a SAFETY comment, enforced by clippy. ALL FFI lives in
`epsilon-safety/src/sandbox/jobobject.rs`; no other crate or module may use unsafe. The
only non-block unsafe item is `unsafe impl Send for JobGuard` with its own comment.

## Test-caught notes

- winapi's `HANDLE` is `*mut ctypes::c_void` (its own flavor) while tokio hands out
  `*mut std::ffi::c_void` — pointer `.cast()` bridges them; `SetInformationJobObject`
  takes `LPVOID` (mutable), so `as *const _ as *const c_void` needed one more hop.
- `JOBOBJECT_*_INFORMATION` unions are zero-init'd then field-written via generated
  `_mut()` accessors — avoids guessing at winapi's Default coverage for unions.
- PowerShell BLOCK-buffers stdout when redirected: the grandchild-PID relay initially
  never delivered through a pipe within the poll window. Switched to a temp-file relay;
  test went green immediately. (Same lesson as M4's handle discipline: pick the channel
  that matches the writer's buffering model.)
- `JobGuard` holds a raw pointer → `!Send`; the future must be Send for the async-trait
  seam, so the guard carries an explicit `unsafe impl Send` (handle values are trivially
  movable; only ever passed back into Job Object FFI).

## Verification

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # 104 passed (was 98; +6 sandbox)
cargo run -q -p epsilon-cli             # demo unchanged
cargo run -q -p epsilon-cli -- -p "confinement smoke"   # runs bash INSIDE a Job Object, exit 0
```

New tests: null-sandbox fail-closed; bwrap argv shape (both platforms); confined run with
caps queried from the kernel; timeout tree-kill; grandchild-dies-with-job.

## Known gaps remaining

- Linux landlock/bwrap runtime verification needs a Unix host (code ships, tests are
  cfg-gated); bwrap has no resource caps without cgroups.
- Windows ACL restricted-token runner: deferred with rationale in the ADR-005 amendment.
- rpc(stdio) mode + OpenAPI emission: deferred (editor-integration milestone).
