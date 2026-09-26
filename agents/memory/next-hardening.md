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
- **A green `cargo test -p <crate>` can be lying.** `crates/epsilon-llm/src/lib.rs` gates
  `pub mod providers` on `#[cfg(feature = "real")]`, so `cargo test -p epsilon-llm` collects
  **71** tests while `--features real` collects **109**. The 38 omitted are the whole
  provider-stream surface. `real` IS a default feature, so `cargo test --workspace` and
  `gate.ps1` run them — but a per-crate command without the flag prints
  `test result: ok` and silently drops a third of the crate. Always pass
  `--features real` for `epsilon-llm`. The 71-vs-109 count gap is the tell.
- **Check the test COUNT, not just `test result: ok`.** A filter or feature gate that matches
  nothing still exits 0. Grep the `test result` line for a non-zero passed count.
- **`cargo build` is not evidence for `real`-gated code.** All of `providers` is behind the
  feature, so a plain build compiles none of what a `providers` change touches. Compile with
  `--features real` before believing anything. (M137's first "BUILD=0" proved nothing.)
- **The `patch` tool can write a literal CR into a byte literal.** Patching a Rust string
  containing `\r` through the tool may split `b"\r\n"` across lines and corrupt the file; the
  error surfaces later and elsewhere. Repair byte-exactly with python `open(p,"rb")`, not
  another patch.
- **`execute_code` breaks on multi-line Rust string literals** (`SyntaxError: unterminated
  string literal`). Use the `patch` tool directly for multi-line Rust.
- **The `patch` tool's syntax check reports spurious "Rust 2015" errors** (let-chains,
  `async fn`) on files cargo compiles fine — its checker ignores the edition. Trust cargo.
- **`cargo test ... | tail` masks the real exit code.** Redirect to a file and `echo $?`
  immediately, or a failing run reads as a passing one.
- **Foreground `cargo` caps out at 420 s.** A cold external target makes clippy exceed that;
  run the battery in the background and poll the log.
- **The gate log is gitignored.** `gate.ps1` writes `target/gate-<label>.log`; copy it to
  `audit/evidence/` *before* `cargo clean` or the evidence is gone.

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
- **M137** — F03 closed: several SSE frames in one HTTP chunk no longer lose all but the first (a
  silent P1 truncation reported as success). `ingest` drains the chunk; the body became a boxed byte
  stream because the concrete `Response` had made chunk framing untestable — **that is why the
  defect survived years**. `bytes` added as a direct dep (already compiled via reqwest, same
  1.12.1) — ADR-017 amendment.
- **M143** — `/session` now prints the cost breakdown. The four `CostBreakdown` accessors had **zero callers**
  anywhere: computed, documented, tested, unreachable — dead code that reads like a feature. pi has no `/cost`, so
  this is not a port; it is Epsilon's own dead code connected to the surface it was written for. Reused the `+`
  floor marker so `/session` and the header cannot disagree. **The first negative-control attempt passed all 321
  tests** — `/session` has two branches and the fixture builds a store-less agent, so I had edited the branch the
  test does not execute. **A control can be wrong in a third way: applied where the test does not run.** Re-running
  proves nothing there; reading the fixture does.
  **F22, found and recorded, not fixed:** pi's `getSessionStats` adds `compaction`/`branch_summary` usage so totals
  "reflect what was actually billed". Epsilon has `usage: Option<Usage>` on both entry kinds and **zero read
  sites** — summarization is a real provider call whose cost nothing counts. Needs a design decision (which model
  is a summarization billed to?), so it is not a patch. Evidence + acceptance criteria in the F22 file.
- **M141** — a session's bill is priced per model (M140's second half). The lesson worth keeping: **when a metric is correct only because every input happened to share one property, the accounting is discarding what would make it correct in general.** The first negative-control attempt was also wrong — it priced every model at the first model's rates *inside* the loop, which double-counted instead of repricing, and only one test noticed. A control must BE the old code, not merely differ from it.
- **M140** — the owner's three numbers (context use, cache hit rate, cost) were **present but wrong** on every provider except Anthropic and Mistral. `seed_usage_compat` set `input = prompt_tokens` *and* read the cached count from the same payload, so the prefix sat in two buckets; three consumers read the buckets as disjoint and each failed differently (rate low, gauge high, cost charged twice). Mistral already had the subtraction — **that asymmetry is the tell that it was an oversight, not a decision.** The Responses wire dropped `input_tokens_details` outright, printing a confident `cache 0%` where the header means `—` for unknown. Fixed at the boundary; `ui/cost.rs` and `ui/metrics.rs` were already right. Needed a new `test_support` seam because the arithmetic happens mid-stream on raw JSON, and `wire_fixtures.rs` has **no usage fixtures at all** — that absence is why it survived.
- **M139** — a model switch now survives closing the session. `model_change` was written faithfully
  and read by nobody (`ModelChange` appeared only in the decoder, the search index and tests), so a
  resume silently reverted to the launch model. Fixed by porting pi's `getSessionContextSettings`
  and wiring its result into `build_runtime`. **Two gotchas worth keeping:** the precedence needs an
  explicit `model_explicit` on `RunParams`/`SharedRun` (a `false` literal there breaks *every* mode),
  and a test appended to the end of `main.rs` lands **outside** the `#[cfg(test)]` module and is
  compiled into the release binary.
- **M138** — the M136 silence is gone for the CLI: `acquire_with_progress` reports each poll of
  the lease wait, and startup redraws one stderr line with a countdown and the reason. The
  callback is `Fn` returning `()` so a frontend can draw the wait but never change the outcome.
  **Its remaining gap is the TUI**: the alternate screen erases the notice, which needs the state
  threaded into the app — and per the repo's own rule that claim has to be *measured* with
  `scripts/console-probe.py`, not asserted from code.

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
- **`os error 2` on a `fs::rename` after a successful `fs::write` is an ENV problem, not a flake.**
  Seen three times (`m142`, `m143`, +1); a *different* test loses it each round, in the same test binary.
  **Cause: this host exports `TMPDIR=/tmp TEMP=/tmp TMP=/tmp`; native Windows resolves those to `C:\tmp`,
  which does not exist.** Always export `TMP`/`TEMP`/`TMPDIR`=`C:/Users/Hp/AppData/Local/Temp` before any
  cargo run that creates files. A still-needed belt is an AV exclusion for `%LOCALAPPDATA%\Temp` — an
  **owner/admin decision**, never from an agent session. Full diagnosis, including why a passing re-run is
  evidence about the environment and not the code: `audit/evidence/temp-var-os-error-2-diagnosis.md`.
  **Never re-run until green and cite only the green one** — that is how a flake becomes a false bill of health.
  Supersedes the AV-quarantine hypothesis in `m142-failed-round.md`.
  — the earlier hypothesis, for history: **A second mid-run artifact deletion (2026-09-26, M142).** `per_kind_counts_tie_out_on_both_sides`
  failed in battery `m142` with `os error 2` on a `std::fs::rename` whose `fs::write` had just succeeded —
  the file was written, then gone. Same signature as the Avira case below, and in a test's own `%TEMP%`
  scratch dir, **not** under the target dir, so the external-`CARGO_TARGET_DIR` mitigation does not cover it.
  Re-run on identical bytes: 5/5 pass. `os error 2` between a successful write and its rename is the AV
  filter, not code. An exclusion for `%LOCALAPPDATA%\Temp` is an **owner/admin decision** — do not change
  security config from an agent session. `audit/evidence/m142-failed-round.md` has the full record.
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

- ~~Session cost is re-priced at the current model.~~ **Closed by M141.** The cause was the accounting, not the pricing: `UiState` kept four bare `u64` totals, so once summed, which model consumed which tokens was unrecoverable. It now keeps a `BTreeMap<String, TokenCounts>` keyed on the message's own `model_id`. The error followed the *direction* of the `/model` switch, which is why it was invisible. **The accumulation gap is now closed by M142** — six tests drive a real `UiState` through both `MessageEnd` and `hydrate_from_history`. The lesson generalises: **when a fix has a producing half and a consuming half, testing the consumer proves nothing about the producer.** M141 tested `breakdown_per_model` with a hand-built map; a collapsed map would have passed all six.
- **The header's numbers still have no legend** — a user sees `?/164k · ↑0 ↓0 · cache — · $0+ ·
  DeepSeek V3.1` and nothing on screen explains the `?`, the `—` or the `+`. Two lines in
  `/help` (`app.rs:1212-1230`) close it. Carried here from the retired
  `metrics-and-reader-honesty` note; it is the smallest slice that directly serves the owner's
  "visibility into the harness's own operation" requirement.
- ~~A pi session's model switch does nothing on resume.~~ **Closed by M139.** `context_settings()`
  is pi's `getSessionContextSettings` over `branch_entries()`, and `resumed_model_to_use` applies it
  when the user passed no `--model`. The rule a faithful port must not drop: an **assistant message
  also names the model** (`session-manager.ts:362-374`, third arm), so a session with no
  `model_change` at all still restores. `thinking_level` is produced by the same walk but **not yet
  applied to the agent** — the next slice, not a bug.
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
