# Working notes: this machine, and this repo's open queue

**This is procedure and open threads, not status.** The open-item map of record is
[`../../audit/evidence/open-items-at-head-2026-09-25.md`](../../audit/evidence/open-items-at-head-2026-09-25.md)
— re-derived from the code, with a verified tier and an unverified tier kept apart. The live
assessment is [`../../audit/11-current-assessment.md`](../../audit/11-current-assessment.md) and
`git log` (what landed). When a number here disagrees with a battery log, the log wins.

## Verification on this machine

- **One build owner at a time.** A battery, a delegated build and a `cargo install` must
  never share the machine. `CARGO_BUILD_JOBS=1` on every long run; an external
  `CARGO_TARGET_DIR` avoids the in-repo `target/`, but a warm in-repo target is faster and
  fine when the disk has room.
- **Run `pwsh -File scripts/gate.ps1 <label>`.** `bash` here is WSL: different `$HOME`,
  no Windows `cargo`, exit 127 in zero seconds with a log that still says `GATE FAIL`.
- **Every attempt gets its own label** — the log is truncated on start, so reusing a label
  destroys the failed attempt's evidence.
- **Evidence must cover the shipped bytes.** Freeze the candidate, run the gates against
  it, and stop an in-flight battery when a fix changes the code.
- **When a pre-fix RED is impossible, run a negative control**: revert one fix in a
  disposable copy (or in place with a hash-verified restore) and show the named test fails
  for the named reason.
- **The gate log is the authority, never a script's `BATTERY_EXIT=` sentinel.** A killed
  battery can still print 0 while the log stops mid-gate; require `===== GATE PASS =====`.
- **Stopping a battery**: take the MSYS wrapper's WINPID from `ps -W` and
  `MSYS_NO_PATHCONV=1 taskkill /F /T /PID <WINPID>`; the process tool's kill can time out
  and leave cargo running.
- **Deleting tests must be reconciled out loud** in the milestone: state the per-crate
  delta and the arithmetic behind it.

## What just landed (2026-09-25)

- **M135** (`813555d`) — `record_edit` splits edit from write in the read-coverage guard; battery
  `m135b`, 1185 tests, negative control captured. The independent review's `capped` finding is kept
  as a named test rather than fixed: an over-cap read shows the model *no lines* and the guard still
  licenses a whole-file overwrite.
- **M136** — the session lease waits out the stale window instead of refusing on first sight, so
  reopening a session you just closed no longer fails for 30 s. Battery `m136a`, 1188 tests, GATE
  PASS. Rejected a pid-liveness probe on purpose: it needs `unsafe` here to learn less than the
  heartbeat already reports. **Its remaining gap is the silence, not the refusal** — the TUI shows
  no progress during the wait, and that needs a callback the session crate cannot own.

## Traps that have cost real time

- **A background gate battery gets killed on this harness, mid-leg.** `m135a` reached 13 of 49 test
  binaries and died without printing `GATE PASS`; per the repo's own rule that log proves nothing.
  A background battery on `CARGO_BUILD_JOBS=1` is long enough that a same-session edit is likely to
  land underneath it *and* long enough to be killed. **Run the battery in the foreground**; the
  420 s tool cap is the lesser evil, and a warm `CARGO_TARGET_DIR` makes the retry cheap. Foreground
  `cargo clippy --workspace` on a *cold* external target does exceed 420 s, so give it a warm
  target or a background run and poll.
- **`cargo test` can fail with `Access is denied (os error 5)` in a dozen unrelated tests, and the
  cause is not your change** (2026-09-25, M135). Avira had pinned the *path*
  `target/debug/deps/<crate>-<hash>.exe`: every test that creates a file failed, every test that
  does not passed, and a **byte-identical copy of the same binary run from `%LOCALAPPDATA%\Temp`
  passes**. That copy is the cheapest disambiguation there is — it separates "my code" from "this
  path" in five seconds, and it is also a way to run a focused test when the machine is busy. The
  fix for a real battery is `CARGO_TARGET_DIR` outside the repo (cold: budget 30–45 min).
  `CARGO_PROFILE_TEST_DEBUG=1` also changes the artifact name, but it **rebuilds the C
  dependencies** (`aws-lc-sys`) and MSVC failed that build with
  `cl : Command line error D8050 ... c1.dll` — do not reach for it first.
  Note also that cargo names test binaries by *metadata*, not by content: editing a source file
  does **not** produce a new name, so "touch the file and retry" does not dodge a pinned path.
- **A test that fails only in a batch, on a file nobody else writes, is worth a copy-and-run before
  a theory.** The same trap, one layer down: `Access is denied` reads like a permission bug in the
  product and is not one.
- **`edit` refuses a source over its own byte cap** (`edit_refuses_capped_source_without_losing_tail`,
  `edit_source_one_byte_over_limit_is_refused`), which means **a file too large to read in one page
  is too large to edit**. So the realistic route into a read-coverage guard is a *narrow* read
  (`{"limit": 20}`) of a small file, not a huge one — a test built on a 5,500-line file fails at the
  edit step and looks like a guard bug. Found while writing M135's behavioural test.
- **`Move-Item`/`mv` restores a file with the BACKUP's mtime**, so cargo reuses the stale
  test binary and a green/red result can be exactly backwards. Touch the file after
  restoring. (Cost 10 minutes during M108's negative control.)
- **PowerShell process sweeps match themselves** when the pattern is in their own command
  line — split the literal or filter on `Name`.
- **MSYS `sha256sum` prefixes `\` to backslash paths**, so comparing hashes across
  `$LOCALAPPDATA` and repo paths gives a false MISMATCH; hash through forward-slash paths
  and strip the escape.
- **MSYS `sed` rejects `s/\\/\//g`** ("unterminated `s' command") — which silently kills an
  `&&` chain mid-way and leaves a restore half-done. Use the file-edit tooling, as `AGENTS.md`
  says; if a shell command must do it, verify the chain's steps separately afterwards.
- **Avira can pin one test-artifact path** (`LNK1104` on a file that does not exist,
  surviving `rm`). Never change security configuration from an agent session; rebuild the
  test profile under fresh metadata (`CARGO_PROFILE_TEST_DEBUG=1`) or move the target dir, and
  disclose the knob.
- **Dispatch a fix round once.** List the running children before dispatching, and check
  the tree rather than the message before assuming a round finished.
- **`pub` items in a library crate are invisible to `dead_code`**, so `clippy -D warnings`
  cannot see a superseded module that kept its public surface (M107). Grep for callers
  when a redesign replaces something.

## Open threads (the ranked list lives in audit/11)

- **The header's numbers still have no legend** — a user sees `?/164k · ↑0 ↓0 · cache — · $0+ ·
  DeepSeek V3.1` and nothing on screen explains the `?`, the `—` or the `+`. Two lines in
  `/help` (`app.rs:1212-1230`) close it. Carried here from the retired
  `metrics-and-reader-honesty` note; it is the smallest slice that directly serves the owner's
  "visibility into the harness's own operation" requirement.
- **A pi session's model switch still does nothing on resume.** M116 made `model_change` and
  `thinking_level_change` heal faithfully and they are typed and durable, but `model_view()` and
  `restore_history` ignore them, so a resumed session keeps the model it was launched with. The
  consumer is the missing half (a small `provider`/`modelId` + `level` mapping) and it was
  deliberately left rather than half-done.
- **The skills collision policy contradicts pi in shipped code**: `crates/epsilon-tui/src/skills.rs`
  says its later-root shadowing "mirrors pi"; pi keeps the FIRST name and reports a collision
  diagnostic (`skills.ts:432-439`). A design decision (which root wins, what the diagnostic says)
  with a test to write — not a patch.
- **The 30-second lockout after a kill** (`crates/epsilon-session/src/lease.rs`): the lease calls
  a holder dead only after `STALE_AFTER`, so relaunching straight after End Task refuses to start.
  A pid liveness check wants a platform call and `unsafe` is forbidden outside a reviewed override,
  so this is a design decision: a narrow liveness probe, or a refusal that waits the window out
  with a countdown.
- **A replaced approval dialog is still silent** (`app/event.rs:191-197` just overwrites
  `app.pending_ask`). Cheap fix: push a notice, batched with an ask FIFO so it costs one battery.
- **Nothing calls `restore`**, so F15's user-facing revert does not exist; and a *skipped*
  capture is never reported to anyone (`files.rs` discards `record()`'s outcome). Latent, not live.
- **G17's class is not closed**: `pub` remains a hole in the dead-code gate, so anything else
  superseded by a redesign and left `pub` is still invisible to clippy.
- **SESSION-01b** (the TUI session surface: `/tree`, `/fork` — both exist as store operations with
  no user path) and **G2/F09** (resolved filesystem containment) are the two design tasks left in
  the port; `f09-path-policy.md` holds the second.
- **`docs/design/epsilon-architecture.md` is stale wholesale** — an audit-era snapshot with at least
  ten verified-stale claims and line references that have drifted onto unrelated code. It needs
  re-derivation or removal, not a patch per line; patching lines leaves a half-current guide, which
  is worse than naming it. Same for `audit/05-verification-log.md`, which AGENTS.md calls the latest
  baseline while its tail is M86–M88.
