# Working notes: this machine, and this repo's open queue

**This is procedure and open threads, not status.** The live baseline is
[`../../audit/11-current-assessment.md`](../../audit/11-current-assessment.md) (gaps,
verdicts, ranked plan) and `git log` (what landed). When a number here disagrees with a
battery log, the log wins.

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

## Traps that have cost real time

- **`Move-Item`/`mv` restores a file with the BACKUP's mtime**, so cargo reuses the stale
  test binary and a green/red result can be exactly backwards. Touch the file after
  restoring. (Cost 10 minutes during M108's negative control.)
- **PowerShell process sweeps match themselves** when the pattern is in their own command
  line — split the literal or filter on `Name`.
- **MSYS `sha256sum` prefixes `\` to backslash paths**, so comparing hashes across
  `$LOCALAPPDATA` and repo paths gives a false MISMATCH; hash through forward-slash paths
  and strip the escape.
- **Avira can pin one test-artifact path** (`LNK1104` on a file that does not exist,
  surviving `rm`). Never change security configuration from an agent session; rebuild the
  test profile under fresh metadata (`CARGO_PROFILE_TEST_DEBUG=1`) and disclose the knob.
- **Dispatch a fix round once.** List the running children before dispatching, and check
  the tree rather than the message before assuming a round finished.
- **`pub` items in a library crate are invisible to `dead_code`**, so `clippy -D warnings`
  cannot see a superseded module that kept its public surface (M107). Grep for callers
  when a redesign replaces something.

## Open threads (the ranked list lives in audit/11)

- **`App::at_file_rows` does not share M99's ignore answer** — its own walker, its own
  hardcoded skip list, so `@` offers files the tools refuse. `epsilon-tui` already depends
  on `epsilon-core`; call `epsilon_core::ignore`.
- **G18: cost prices cached tokens at zero.** Needs a `ModelInfo` field, a column in
  `scripts/regen_catalog.py`, the regenerated table — or label the number honestly as an
  input/output-only estimate.
- **F07: a failed durable write is silent.** `persist` and `pop_last_message` discard their
  errors while the provider reads the store's projection.
- **The 30-second lockout after a kill** (`epsilon-session/src/lease.rs`): the lease calls
  a holder dead only after `STALE_AFTER`, so relaunching straight after End Task refuses to
  start. A pid liveness check needs a platform call and `unsafe` is forbidden outside a
  reviewed override.
- **A replaced approval dialog is still silent** (`app/event.rs:191-197` just overwrites
  `app.pending_ask`). Cheap fix: push a notice, batched with an ask FIFO so it costs one
  battery.
- **M95's read follow-ups** are recorded in its milestone Limits: the `limit:0` count uses
  `total` where pi uses `total - startLine`, an empty file with `limit:0` is silent, and
  `details.read.offset` disagrees with the notice on an empty page.
- **SESSION-01b** (the TUI session surface: `/tree`, `/fork`) is unblocked since the TUI
  crate is committed; **G2** (resolved filesystem containment) is still a design task.
- **The maintained-doc rewrite is incomplete**: `docs/history.md` is a 57 KB timeline read
  by nobody, and `docs/README.md` should be the map a newcomer actually follows.
