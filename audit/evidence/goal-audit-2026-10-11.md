# Goal audit — 2026-10-11 (in progress, task `audit`)

Evidence-first record. Every claim cites a line; nothing here is asserted from memory
of prior reports. Checked against the tree at HEAD (post-`ca87ba3`).

## Verified this pass

- **One composition point holds.** Every launch path goes through
  `epsilon_runtime::{resolve_launch, build_runtime}`:
  `crates/epsilon-cli/src/main.rs:1181,1269,1459,1714,2024`,
  `crates/epsilon-desktop/src/lib.rs:160`,
  `crates/epsilon-desktop/src/session.rs:1927`,
  `crates/epsilon-desktop/src/app.rs:16034,16574`.
  No second composition found (`grep build_runtime\( / resolve_launch(`).
- **One command registry holds.** `COMMAND_HEADS` lives in
  `crates/epsilon-contracts/src/command.rs:371`; the window's `SUPPORTED`
  (`crates/epsilon-desktop/src/commands.rs:1546`, 37 names) is the subset declaration and the
  palette derives from it. Gate checkers green this pass:
  `check-parity-citations` — 623 citations in `pi-tui.md` + `dsh-desktop.md` resolve.
- **Latest green battery** (per `docs/STATUS.md:343`): `gate-port-r147-2026-10-10`,
  15 legs all `[exit=0]`, 2,339 passed / 0 failed / 19 ignored.
- **Committed `ca87ba3`** (was uncommitted at session start):
  `fix(llm): billing limits are final even when they wear a 429`
  (`crates/epsilon-llm/src/retry.rs`). `is_spending_limit` is checked before the
  rate-limit branch (which matches bare `quota`), classified `Other` (final, not `Auth`).
  Evidence: `retry::tests::a_billing_limit_is_final_even_when_it_wears_a_429` passes,
  all 12 `--lib retry` tests pass, `cargo fmt -p epsilon-llm --check` clean,
  `cargo clippy -p epsilon-llm --all-targets` clean.
  RED basis (code-cited, not re-run): pre-fix, `"429 quota exceeded"` matches the
  rate-limit arm (`m.contains("quota")`), i.e. `RateLimit` → 5 backoff retries.

## Findings

### F2 (stale record): `STATUS.md` "Next" items 1–2 landed but still read as open

- Item 1 (todo double-draw) fixed in `9e8dea3` — `crates/epsilon-tui/src/state.rs:991-1038`
  settles the card, with `CONTROL_RESTORE_TODO_DOUBLE_DRAW` (`:9`) as the negative control.
- Item 2 (session deletion) landed: `crates/epsilon-session/src/delete.rs` (trash-then-unlink,
  `DeleteOutcome::message`), terminal browser wired end to end; P-237 is `partial`
  (`docs/parity/pi-tui.md:667`) with 3 unit + 4 frame tests, desktop deliberately without a
  delete surface. The "written then reverted" sentence predates the landing.
- Item 5 (transcript filter cycle) landed after it was written: P-294 is `partial`
  (`docs/parity/pi-tui.md:705`) with `ctrl+o`/`shift+ctrl+o` cycling three of five modes on the
  live transcript (frame-pinned), `TreeFilter::next` keeping Pi's order
  (`crates/epsilon-tui/src/app.rs:2139-2149`), and the `/tree` picker supporting all five
  (LabeledOnly via labels map, `app/commands.rs:159`; cycle on `f`, `app/keys.rs:853).
  The item's cautions (pinned order, taken chord) were honored, not open.
- Item 3 (tool-card caps) decided and landed: `body_keeps_tail`
  (`crates/epsilon-tui/src/ui/card.rs:274-292`) — text keeps the tail, diffs keep the head,
  with Pi's non-uniformity cited; P-142 (`pi-tui.md:503`) and P-156 (`:528`) are `partial`
  with frame tests. The "wants a written decision" is written — in code and rows.
- Item 4 (model picker) landed: provider-first order pinned (`pi-tui.md:680`, P-250),
  uncatalogued ids synthesized, struck and badged (`:690`, P-260), scoped-overlay opposite
  order deliberately not built (one picker, one order). Remaining half is recorded, not open.

All five read as "what to take next"; all five are done-or-recorded.

Both still sit under `## Next` as "what to take next". Fix direction: move them to the
phase log / mark landed with the commit + battery.

### F1 (stale doc, user-visible reasoning): `commands.rs` module doc describes the M267-era split

`crates/epsilon-desktop/src/commands.rs:74-80` says the window-level set is
"`/sessions`, `/tasks`, `/session`, `/clear`, `/theme`, `/help` — exactly `SUPPORTED`, no more"
and lists "`/new`, `/resume`, `/fork`, `/clone`" plus "`/timeline`, `/editor`, `/hotkeys`" as
terminal-only, refused by name. The tree contradicts it:

- `SUPPORTED` (`:1546-1582`) holds **37** names including `new`, `resume`, `fork`, `clone`,
  `hotkeys`, `tree`, `preset`.
- Dispatch arms exist: `Command::Fork`/`Clone` at `:149-150`, `Command::Tree` at `:293`.
- The "thirty-seven names" sentence at `:30` is still numerically right, so
  `check-verb-counts` cannot see this drift — the stale-sentences class `STATUS.md` warns
  `check-links` is blind to.

Fix direction (not yet applied): rewrite the split bullets to the current 37-vs-1 shape
(`/editor` the single refusal) or delete the enumeration and point at `SUPPORTED`.

## Carried (re-confirmed, not re-investigated)

- **Plaintext credentials in `auth.json`** vs locked `AGENTS.md` §6
  (`crates/epsilon-llm/src/auth.rs:6`, `:222-224`); plan 026 R6, proposal 7 unanswered.
  Needs owner signature — cannot be closed by an agent alone.
- **No real screen reader run, no dated human keyboard-only pass** (plan 026 R5/H).
- **Desktop pixels never seen on this machine** (offscreen raster crashes; round 58 re-proved).
- **Release untagged** (plan 026 R7/I); **compaction digest replay+markdown halves open**
  (P-054/P-183 `partial`).

## Core mapping (verified)

- Agent loop: `Agent` at `crates/epsilon-core/src/lib.rs:697`, `run` at `:2145`
  (file 4,709 lines — the known monolith; M161–M166 carved seams out of the TUI side,
  the core side's size is audit-noted, not yet judged).
- Permission gate seam: `crates/epsilon-core/src/gate.rs:1` (+ ADR-032 presets,
  `preset_ruleset` in `epsilon-safety`, `--preset` through launch).
- Tools: `crates/epsilon-tools/src/` — files, shell, task/subagent (`task.rs`), goals
  (`goal.rs`, `goal_gate.rs`), checkpoints (`checkpoint.rs`), skills (`skill.rs`),
  todo (`todo.rs`), webfetch, mutation queue.
- Session store: `crates/epsilon-session/src/` — `store.rs`, `delete.rs`, export;
sessions rooted per workspace (M217).

## Repo checks (run this pass, 2026-10-11)

- `check-links`: 1007 relative links, 0 broken.
- `check-line-endings`: 1360 tracked files, 0 CRLF.
- `check-control-bytes`: 1360 tracked, 0 control bytes.
- `check-parity-citations`: 623 citations resolve.
- Docs map (`docs/README.md`) matches the tree: design/, user/ (10 guides), parity/,
  plans/ (incl. 024/025/026), qa/, reviews/, research/ all present as listed.

## Desktop spot-checks (verified this pass)

- Quit guard sends `CancelClose` (`crates/epsilon-desktop/src/app.rs:8394`; rationale at
  `:4820`, hole analysis at `:15606`) — round 17's fix present.
- New-session button in the sessions pane (`:6457-6483`, chord in tooltip + a11y name) —
  rounds 10/16/63 present.
- Failure notices draw as failures (`RowKind::NoticeFail`, `app.rs:391,589,8633`,
  `session.rs:152`) — round 107 present.

## Landed slices (this goal)

### S1 — TUI compaction replay half (`143e0af`, core task)

P-054/P-183 replay half for the terminal: `COMPACTION_MARKER` once in contracts,
`SessionStore::compaction_digests()` (branch order), one `compaction_cost_line`
builder, one `hydrate_compaction_digests` helper for all four hydrating paths,
one `strip_line` funnel. Evidence: store + 2 ui + 1 e2e (wire-removal control) +
1 render-equivalence test (80/100, frame read: cost row + `… 1 more line`).
Suites: contracts 93, session 80+42+6, tui 609, core 206+… all green; fmt clean;
clippy clean on all four crates; parity checkers green. Still open: markdown body,
desktop replay half.

## Next in this task

TUI/desktop surface spot-checks against the matrices, docs-structure contradiction sweep,
then close `audit` with the report + STATUS fix-ups (F1/F2) or file them as work items.
