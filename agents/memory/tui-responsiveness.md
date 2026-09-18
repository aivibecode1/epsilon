# TUI responsiveness in cmd/pwsh — owner-reported, in progress

## Symptom (owner, 2026-09-18): "very bad, not responsive in cmd, pwsh"

## Reproduced + measured (real conhost cmd window, installed epsilon.exe 05:32)
- Harness: a cmd window I opened titled `EPSILON-TUI-TEST` running the installed binary; driven
  with `computer_use` (consoles **drop background input** — every input action needs
  `delivery_mode="foreground"`; capture works normally). Window ≈ 120×33 cells.
- **Measurement 1:** typed 85 chars in one burst → ~57 echoed at the immediate capture.
- **Measurement 2 (marker groups):** typed 99 chars as 20 groups (AAAA…TTTT) → ~10 groups
  echoed immediately, all 20 present after ~2–4 s ⇒ **drain rate ≈ 30–50 chars/s** ✗.
- Vision saw "G GGG" where "GGGG" was typed in two independent reads ⇒ **possible character
  loss** (to pin at code level: does crossterm's EventStream drop when the reader outpaces the
  consumer? — the read-only brief `deleg_c3db7b5c` was asked exactly this).
- Input is **delayed, not lost overall** (the tail arrives) ⇒ consistent with a per-event frame
  bottleneck, not a broken decoder.

## Root cause (code + arithmetic agree)
`crates/epsilon-tui/src/app.rs` run loop (≈2664-2773):
```
loop {
    if app.external_editor_requested { ... }
    let _ = terminal.draw(|f| draw(f, &mut app));   // FULL REDRAW, every iteration
    tokio::select! {
        ev = events_rx.recv() => ..., ask = approvals.next_ask() => ...,
        notices_rx.recv(), oauth_rx.recv(), fetched_rx.recv(),
        maybe_ev = term_events.next() => { handles ONE key/mouse/paste event },
        _ = tick.tick() => { spinner phase }        // TICK_MS = 120 (app.rs:61)
    }
}
```
So: **one terminal event per iteration, and one full redraw per iteration.** A 99-key burst =
99 redraws ≈ 99 × ~30 ms ≈ 3 s in conhost ✓ matches the measurement. At idle the tick forces
8 redraws/s (wasteful); during a run the event stream + tick compete with input in the same
one-event-per-iteration loop ⇒ typing lags badly (the owner's complaint).

## Planned fix (minimal, targeted)
1. **Drain every ready input event per iteration** before drawing, without adding a dependency:
   `while let Ok(Some(Ok(ev))) = tokio::time::timeout(Duration::ZERO, term_events.next()).await`
   (timeout-0 polls once and returns `Elapsed` when nothing is ready). Handle each event.
2. **Gate the redraw**: a local `did_something` set by every select arm that fired, plus
   `|| app.ui.running()` so the spinner still animates; then draw at most once per iteration,
   and not at all while idle. (Chosen over a per-mutation `needs_redraw` flag: same win, no
   instrumentation of every mutation site.)
3. `biased;` in the select with the input branch first, so a busy stream cannot delay input.
Expected: burst latency ≈ one frame (~30 ms) instead of seconds; idle CPU → ~0.

## Verification method (re-run the same harness)
Rebuild the dev binary, quit the old TUI in the test console (Esc), run the new binary from the
test console, then repeat the 99-char marker burst: all 20 groups must appear at the *immediate*
capture. Also re-check the idle case and (if a faux provider can be wired) a streaming run.

## Open questions for the round
- The TUI work item's changes are **uncommitted and unreviewed** (`agents/memory/tui-enhancement.md`
  says: not committed/installed, whole-file stage/install would absorb unreviewed baseline, a
  candidate was rejected by an earlier review, and "keyboard/interactive terminal validation" was
  still outstanding — i.e. exactly this work). Decision needed: fix the loop (this slice), then
  get the TUI into a *verified, committed* state rather than leaving it in working-tree limbo.
- Streaming-path testing needs either the owner's go-ahead for a real provider call or a faux
  provider reachable from the CLI (a genuinely useful harness addition).
- The console window `EPSILON-TUI-TEST` is deliberately left open for the verification round.

## Build-environment lesson (2026-09-18, cost: one failed build diagnosed)
Sharing one `CARGO_TARGET_DIR` between the repo and a Temp *candidate copy* leaves cargo
artifacts that can be stale across the two source trees: the TUI then failed with
`E0425: cannot find function load_config_string` even though the working tree defines it
(`crates/epsilon-core/src/config.rs:32`) — the linked `epsilon-core` rlib had been built from
the candidate's HEAD copy. Fix used: `cargo clean -p epsilon-core -p epsilon-tui` then rebuild.
Rule: a frozen-candidate battery gets its **own** target dir, or the repo is rebuilt after it
with those two crates cleaned.

## Dirty-file attribution (for the eventual TUI commit)
- **TUI item:** `crates/epsilon-tui/**` (app.rs, state.rs, picker.rs, editor_ops.rs, markdown.rs,
  fuzzy.rs, lib.rs, Cargo.toml), root `Cargo.toml`/`Cargo.lock` (M36 unicode-segmentation),
  `crates/epsilon-core/src/config.rs` (`load_config_string`/`_in` — app.rs:145 and :631 call it,
  and it is the only epsilon-core symbol the TUI needs from the dirty set),
  `audit/README.md` (TUI note link).
- **Other workstreams (do not stage with the TUI):** `crates/epsilon-core/src/registry.rs`
  (stable sorted tool order), `crates/epsilon-contracts/src/stream.rs` (DeepSeek/Anthropic
  thinking-vs-text index collision), `crates/epsilon-core/assets/agent-contract.md` (task-tool
  guidance), `crates/epsilon-tools/descriptions/write.txt`, `docs/milestones/M18+M19` (path
  fixes), `crates/epsilon-contracts/src/lib.rs`'s session_entry/tool re-export expansions.
- Verified: the TUI crate greps clean of `epsilon_core::registry` and `epsilon_contracts::stream`
  (zero references), so its commit does not need those files.

## Round-script gotcha (M94a, false alarm)
Comparing an archive-extracted candidate file against `git show HEAD:file` with plain `cmp` can
report a false LEAK: the extraction and the blob can differ in line endings only (M94's
registry.rs: same content, +113 bytes of CRs). Compare EOL-normalized:
`git show HEAD:$f | tr -d '\r' | cmp -s - <(tr -d '\r' < "$B/$f")`. The battery itself is
indifferent to line endings.

## M94 fix round — verified state (2026-09-18 ~22:00)
- The review's three blockers are fixed in the working tree: an approval that was not rendered
  cannot be answered (keys refuse + a notice), the undersized fallback names a pending approval,
  and the approval dialog is drawn LAST (over the timeline). The visibility field is
  `approval_visible` at this point (superseded in the re-review round — see below).
- Three new rendering tests, each proven to fail pre-fix:
  `audit/evidence/m94-newtests-red.log` — reverted-state app.rs sha256 `f3b1856f…`, all three
  FAILED (0 passed / 3 failed, exit 101). The tree was restored to the fixed state
  (sha256 `5ac0f0fd…`, identical to the `Temp/m94-app-fixed.rs` snapshot) — verified by me.
- Focused evidence saved and cited by the milestone: `audit/evidence/m94-focused-tui.log`
  (**161 passed / 0 failed** = 158 + the 3 new) and `m94-focused-cli.log` (**38 passed**).
- All six doc corrections are in the milestone (cached_branch memoization, the Esc citation,
  keybindings/trust_store/epsilon-session scope, the dead `interrupt` binding, the misnamed
  logs, the ignored-test reconciliation).
- NEXT: launch `$LOCALAPPDATA/Temp/m94b-round.sh` (ready) — candidate = HEAD + the TUI set with
  EOL-normalized exclusion checks, a duplicated-test guard, focused runs, then the battery; the
  verdict is read from `gate-m94b.log`'s tail (GATE PASS), never from a printed sentinel.
- Then: history row, commit (carry the TUI set + config.rs + Cargo pins + keybindings/trust_store
  + the evidence logs + docs), reinstall from the frozen candidate, cleanup, retire this note.

## Re-review of the M94 fix round (deleg_c321b021, verdicts + what it changed)
- Item 1 (visibility guard): (a) every draw path writes it, (b) default false is safe (the loop's
  first draw precedes any seated ask), (c) the refusal branch sits before take()/guidance, (d)
  resize recomputes on the following draw — BUT one narrow BLOCKER: the flag is a last-frame fact
  and is never reset when a NEW ask is seated (drive's seating site ~app.rs:2782-2784), so within
  the <=33 ms frame window a second parked ask can be answered before any frame paints it.
  Fix round deleg_d729284e: identity-based guard preferred (paint the ask's id; answer only that
  id), else a structural reset in an extracted `seat_ask`; plus a test that reproduces it.
- Item 2 (draw order): complete and precedence unchanged — but a coherence gap: the dialog is
  drawn over the auth prompt while the auth key branches run first, so /login's masked buffer eats
  y/a/n/A/Esc/Enter. Same fix round routes approval-before-auth when an ask is pending (+ test).
- Item 3: the three tests assert their named properties through real paths; RED evidence matches
  byte-for-byte (the panic strings equal the current asserts; 0/3 with 158 filtered + 3 = 161).
- Item 4: the bounded drain cannot lose events (the 129th is left for the biased select).
- Item 5: numbers verified (161/38, RED 0/3) + logs pin the current app.rs sha `5ac0f0fd…`;
  three wording nits fixed in the same round.
- Battery m94b (proc_7b6f7fc147aa) was STOPPED when the re-review landed (bytes were changing);
  its log tail shows it had already passed fmt + clippy + the full test gate and was in `demo`.
  The post-fix battery re-runs as `m94c` after deleg_d729284e reports.

## Re-review round 2 — the blocker and the coherence gap are fixed (2026-09-18 ~22:40)
- Blocker (item 1, second parked ask answerable before a frame paints it): fixed with the
  IDENTITY-based guard. `App::approval_painted_id: Option<String>` (`app.rs:231`) records the
  `request_id` of the ask the frame painted; `draw` sets it at the paint site and the undersized
  fallback clears it. The approval keys were extracted into `handle_approval_key` (`app.rs:3231`),
  which answers ONLY the ask whose id matches the painted record and otherwise refuses with
  "approval is not visible yet — …" — no reset to forget, no staleness window.
- Coherence gap (item 2): the approval keys now route BEFORE the auth branches when an ask is
  pending (`handle_key`: `if app.pending_ask.is_some() && !app.keybindings.matches("app.exit", &key)`),
  so /login's masked prompt can no longer eat y/a/n/Esc/Enter (Enter used to save junk to
  auth.json). The auth state is untouched and the exit keybind keeps its old precedence (quitting
  drops the parked ask -> Deny). The M30 steer mirror moved above the modal branches (unchanged
  semantics: it is a status badge mirror).
- Two new tests, both proven RED pre-fix and GREEN after: `a_second_parked_ask_is_not_answerable_before_a_frame_paints_it`
  (`app.rs:5697`) and `a_pending_approval_preempts_the_auth_key_prompt` (`app.rs:5779`).
- Evidence: `audit/evidence/m94-focused-tui.log` ROUND 2 — fmt/fmt/clippy clean, tui **163 passed**
  (+2), cli **38 passed**; `app.rs` sha256 `5932c6d0…` unchanged across the round.
  `audit/evidence/m94-newtests-red.log` ROUND 2 — the two tests on the pre-fix bytes: **0 passed /
  2 failed** (`Char('y') answered ask B before any frame painted it`; `'y' must answer the approval,
  not the auth buffer`), the fixed file preserved byte-exactly, restored byte-identically
  (sha256 equal, `cmp` exit 0), restored bytes re-run green.
- Residual (recorded in the milestone's Limits, not fixed here): an ask parked while another is
  pending is overwritten by `drive`'s ask arm (`app.rs:2786`) and resolves to Deny unseen; the fix
  shape is a local FIFO in the run loop.
- Doc nits fixed in the milestone: the guard mechanism restated for the identity form, the Esc
  test's real walk (`app.rs:6844`, popup dismiss then idle quit), and the "not cited anywhere"
  self-reference -> "named only as non-evidence".
