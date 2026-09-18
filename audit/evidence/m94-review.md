# M94 review record — three read-only rounds, findings and dispositions

All three reviews were read-only (no edits, no cargo, no commits) and each judged a different
state: the slice as written (round 1), the round-1 fix delta (round 2), and the round-2 fix delta
(round 3). This file is the disposition list AGENTS.md requires: no finding disappears.

## Round 1 — the slice (deleg_aee8d4b1)

| # | Finding | Severity | Disposition |
|---|---|---|---|
| B1 | Below 60×12 the new fallback returned before `draw_dialog`, yet `y`/`a`/`n`/`Esc` still dispatched — an **invisible approval could be answered**. HEAD drew the dialog at any size, so the slice introduced it. The only test asserted copy. | blocker | Fixed in round 1: the undersized banner names the pending ask, and the approval keys refuse while it cannot be seen. Test `approval_keys_do_not_answer_an_invisible_approval` (fails pre-fix: `Char('y') answered an approval that was never drawn`). |
| B2 | The pending-approval dialog was drawn *before* the timeline, which clears the centre — an ask arriving during `/timeline` was covered while its keys (including `Esc` = deny) stayed live. | blocker | Fixed: the dialog moved to the end of `draw`, always on top. Test `a_pending_approval_is_drawn_over_the_timeline`. |
| B3 | The Verification numbers (158/38) had no log behind them; the newest TUI logs said 157, from before the loop edit. | blocker | Fixed: focused runs re-executed on the frozen bytes with logs saved (`m94-focused-tui.log`, `m94-focused-cli.log`), and round 2's numbers likewise. |
| F | A dead `app.interrupt` keybinding; the two red logs under green names; the `.git/HEAD` memoization claim was wrong; the Esc test citation was wrong; the commit-scope list omitted `keybindings.rs`, `trust_store.rs`, `epsilon-session`. | findings | All corrected in the milestone (see its Limits); the dead binding is recorded as a limit. |
| — | Loop semantics (dirty coverage, drain, frame ceiling, quit paths): read as sound. | verified | — |

## Round 2 — the round-1 fix delta (deleg_c321b021)

| # | Finding | Severity | Disposition |
|---|---|---|---|
| B1 | `approval_visible` was a last-frame fact, never reset when a new ask was seated — within the ≤33 ms frame window a second parked ask could be answered before any frame painted it (parallel tool calls park asks FIFO). | blocker (narrow) | Fixed in round 2 with the identity-based guard (`approval_painted_id` = the gate's own `request_id`). Test `a_second_parked_ask_is_not_answerable_before_a_frame_paints_it` (fails pre-fix: `Char('y') answered ask B before any frame painted it`). |
| F | The dialog now draws over the auth prompt while the auth key branches run first — `/login`'s masked buffer would eat `y`/`a`/`n`/`Esc`/`Enter`. | finding (coherence) | Fixed: approval keys route before the auth branches when an ask is pending, the exit keybind keeps its precedence, and auth state is provably untouched (test `a_pending_approval_preempts_the_auth_key_prompt`). |
| F | The mechanism claim "true only in the frame that painted the dialog" was false. | nit | Corrected to the identity-based statement. |

## Round 3 — the round-2 fix delta (deleg_20465716)

**No blockers.** Verdicts: the identity guard has no matching-but-unpainted hole (unique ids from
the gate's `new_id("perm")`; no recycling; one assignment site); the auth precedence is safe and the
exit keybind keeps its old path; a replaced ask is denied fail-closed by the channel's drop
semantics and **cannot stall a run**; both tests assert through real paths and the RED evidence
re-hashes correctly; docs match the code; nothing regressed (163/163 on the frozen bytes).

| # | Finding | Severity | Disposition |
|---|---|---|---|
| F1 | A *painted but unanswered* ask that is replaced is denied **silently** (the dialog content swaps on the next frame). | finding | Behaviour verified fail-closed; the notice is queued in `agents/memory/next-hardening.md` (batched with the run-loop ask FIFO so it costs one battery cycle). Limits reworded to describe both cases. |
| F2 | `'e'` guidance clears `app.input`, which doubles as `/login`'s filter — "auth state untouched" is exact for `y`/`a`/`n`/`Esc`, not for `'e'`. | finding | Limits state the precise scope; the shared-input coupling is its own slice. |
| F3 | After a shrink, the painted id persists until the next frame (≤33 ms) — last-paint semantics. | finding | Recorded in Limits as inherent to any last-frame guard. |
