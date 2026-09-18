# M94 — TUI responsiveness (owner-reported) and landing the in-tree TUI work

## What

The owner reported the TUI "very bad, not responsive in cmd, pwsh" (2026-09-18). It reproduced on
the first try in a real cmd window (conhost) running the installed binary: a 99-character burst
echoed at roughly **30 characters/second** — about half the burst was still missing at the
immediate capture, and the rest took ~3 s to appear.

The cause is in the run loop (`crates/epsilon-tui/src/app.rs`), which this slice fixes:

- it drew a **full frame before every input event** and consumed **exactly one** terminal event per
  iteration, so N keystrokes cost N frames — and in a Windows console a frame is a full console
  write (tens of milliseconds);
- Windows delivers **Press *and* Release** per keystroke, so half of those frames were pure
  no-ops;
- during a run it drew once per **streamed delta**, and each streaming frame re-wrapped the whole
  in-flight answer (the in-progress TUI work made that per-frame cost heavier than HEAD's), which
  is why long answers felt worse than short ones;
- at idle the 120 ms tick forced ~8 redraws/second for nothing.

**The loop now**: drains already-ready input before drawing, bounded at 128 events per turn
(`MAX_INPUT_DRAIN`, `app.rs:73`) so a sustained flood cannot starve the frame decision
(zero-timeout poll — no new dependency), coalesces agent events the same way (bounded `try_recv`),
draws **only when something changed** and at most ~30 fps (a due-frame timer wakes the loop so a pending redraw
never waits for the next event), and biases the select so input and approval prompts outrank the
agent stream. The event body is extracted into `handle_term_event` so the drain and the select
share one implementation.

**Measured after** (same console, same burst): all 99 characters present at the immediate capture
— echo latency ~one frame instead of seconds.

**Two approval-visibility defects (one class) are fixed in the same slice; the re-review round
made the guard identity-based and the key precedence coherent with the draw order.** Below the
60×12 floor `draw` returned after the size banner, so the approval dialog was never painted while
`handle_key` still routed y/a/n/Esc to the pending ask — a keystroke could authorize (or, via Esc,
silently deny) a request the user could not see. Separately, `draw` painted the dialog *before*
`draw_timeline` (whose widget clears its own centre), so an ask arriving while `/timeline` was
open was covered while the keys still routed to it. Now the fallback names the pending approval
(capability · pattern) and its keys; `draw` records **which** ask it painted
(`App::approval_painted_id`, `app.rs:231`) and `handle_approval_key` (`app.rs:3231`) answers only
an ask whose `request_id` matches that record — anything else is refused with "approval is not
visible yet — it appears on the next frame; enlarge the terminal (need 60×12) if it does not" and
stays pending, so a second parked ask seated inside the same frame window is unanswerable until a
frame paints it (no reset to forget, no staleness window); and the dialog is painted last, after
auth prompts, timeline, @-file popup and slash autocomplete, so it is always on top. The same
round fixed the key precedence that draw order implies: with an ask pending the approval keys
route BEFORE the auth branches (the masked `/login` prompt used to receive y/a/n/Esc/Enter —
Enter could save junk to `auth.json` — while the user was looking at a dialog they could not
answer), leaving the auth state untouched so the login flow resumes where it was; the exit
keybind keeps the precedence it had (quitting drops the parked ask, which resolves to Deny).
Rendering tests: `approval_keys_do_not_answer_an_invisible_approval` (`app.rs:5593`, y/a/n/Esc
all refused), `an_undersized_terminal_names_a_pending_approval` (`app.rs:5639`),
`a_pending_approval_is_drawn_over_the_timeline` (`app.rs:5661`),
`a_second_parked_ask_is_not_answerable_before_a_frame_paints_it` (`app.rs:5697` — the re-review's
race: ask A painted and answered, ask B seated with no frame in between, y/a refused with a notice,
and the next frame's paint makes B answerable) and
`a_pending_approval_preempts_the_auth_key_prompt` (`app.rs:5779` — `/login`'s masked prompt open,
'y' answers the approval and the prompt's state is untouched).

This slice also lands the in-tree TUI enhancement that was sitting uncommitted and unreviewed
(composer/caret, wrapping/height, transcript slicing, pickers, markdown LaTeX preprocess
(`latex.rs`), fuzzy matching, editor ops, the modal key/draw-order routing fix), together with the
carried crate changes it cannot be split from — not "two things"; the real scope is:

- `crates/epsilon-core/src/config.rs` — the `epsilon_core::config::load_config_string*` helpers the
  TUI calls; the working tree does not compile without them, the interlock that kept this work
  stranded;
- the M36 `unicode-segmentation` exact pin in the root `Cargo.toml` + `Cargo.lock`;
- `crates/epsilon-tui/src/keybindings.rs` (M37 key indirection —
  `~/.config/epsilon/keybindings.json`);
- `crates/epsilon-tui/src/trust_store.rs` (M45 per-path trust memory) — a deliberate sibling of
  `epsilon-cli/src/runtime.rs`'s store: it lives in the TUI crate so the TUI can record
  acceptances without depending on the CLI (tui → cli would be a dependency cycle);
- a new `epsilon-session` dependency in `crates/epsilon-tui/Cargo.toml`
  (`epsilon_session::list_sessions`, used by the `/sessions` browser at `picker.rs:941`).

The commit boundary is enumerated below; the other dirty workstreams' files are provably excluded
(the crate greps clean of `epsilon_core::registry` and `epsilon_contracts::stream`).

## Key decisions

- **Coalesce + cap + gate**, rather than a per-mutation `needs_redraw` flag: the same win without
  instrumenting every mutation site (a local `dirty` set by each select arm is enough).
- **No new dependency**: `tokio::time::timeout(Duration::ZERO, term_events.next())` polls once and
  reports `Elapsed` when nothing is ready, which is exactly a non-blocking drain.
- **The tail re-wrap cost is bounded, not cached**: the frame ceiling makes an O(answer) wrap cost
  a bounded fraction of wall time; caching it is queued as a follow-up if measurements still show
  it (Limits).
- **The hint copy stays as it is.** The diagnosis brief called "Esc quit" stale; the crate's own
  test pins that idle Esc *does* quit (`esc_dismisses_popup_before_quitting_or_aborting`,
  `app.rs:6844` — Esc dismisses the slash popup first, then a second Esc quits from idle; the
  test walks exactly those two steps and does **not** walk the "aborts a live run" arm). The
  brief's claim was wrong and is corrected here rather than acted on.
- **Approval visibility is recorded, not inferred, and the record is an identity**: `draw`
  writes `App::approval_painted_id` — the `request_id` of the ask that frame painted — and the
  approval key branch refuses any ask whose id is not that name, so no reset is needed and no
  staleness window exists: the ask becomes answerable the moment a frame paints it. Asking "would
  the dialog fit?" at the key site instead would duplicate the layout rule and drift from it; a
  draw fact cannot drift from what was drawn. The first shape of this guard — a per-frame
  `approval_visible` flag — was a last-frame fact that *persisted* between frames, so a second
  parked ask inherited the previous ask's credit and could be answered before any frame painted it
  (the re-review's blocker); the identity form removes the inheritance instead of adding a reset
  that every new seating site would have to remember to call.
- **The pre-draw drain is bounded** (`MAX_INPUT_DRAIN = 128`, `app.rs:73`): the coalescing wins
  stay, but a sustained input flood can no longer keep the loop from reaching its frame decision —
  the remainder is served by the biased select, which puts input first.
- **The commit carries the TUI's own dependency** (`epsilon-core/src/config.rs`): the working tree
  does not compile without it, which is the interlock that kept this work stranded.

## Gotchas

- **A frame on conhost is a full console write.** Everything here follows from that: the fix is
  about how *often* frames happen, not how cheap each one is.
- **Press+Release doubling** is Windows-specific and doubled the cost of the naive loop.
- **A shared `CARGO_TARGET_DIR` between the repo and a Temp candidate copy leaves stale
  artifacts**: the TUI then failed with `E0425: cannot find function load_config_string` although
  the working tree defines it (`crates/epsilon-core/src/config.rs:32`). Fixed with
  `cargo clean -p epsilon-core -p epsilon-tui`; rule recorded in `agents/memory/next-hardening.md`.
- **An earlier independent review rejected an earlier candidate** of this TUI work for an
  undersized fallback that hid approval details while leaving approval keys live (its report is
  `audit/10-tui-enhancement.md`, with the review itself at
  `audit/evidence/m86-candidate-review.json`). The modal routing fix and its denial regression are in
  this state; the M94 review re-opened the same class — the fallback was still answerable and a
  live ask could be covered by `/timeline` — and both halves are now fixed and pinned by rendering
  tests. The M94 re-review then found the same class one level down: the guard itself was a
  last-frame flag a second parked ask could inherit, and the re-review round replaced it with the
  identity record (see What and Key decisions).

## Verification

**Focused runs on the working tree, raw logs in `audit/evidence/`** (HEAD `5ab723f` plus this
uncommitted slice):

- **Round 1** — the slice as first written; `app.rs` sha256
  `5ac0f0fd44b2102f898492467798db2c9ddf997a028284e4449ad00a7dcc2fa8` at the start *and* the end of
  both runs, so the numbers describe one unchanged source state:
  - `m94-focused-tui.log` — `cargo fmt --all -- --check` clean (exit 0) and
    `cargo fmt -p epsilon-tui -- --check` clean (exit 0);
    `cargo clippy -p epsilon-tui --all-targets -- -D warnings` clean (exit 0); `cargo test -p
    epsilon-tui` **161 passed / 0 failed** (exit 0) — the 158 of the previous state plus the three
    rendering tests added for the approval-visibility fixes.
  - `m94-focused-cli.log` — `cargo test -p epsilon-cli --lib` **38 passed / 0 failed** (exit 0).
  - `m94-newtests-red.log` — the three new tests against the **pre-fix behaviour** (the key guard,
    the truthful undersized banner and the draw-order move temporarily reverted; nothing else
    changed): **0 passed / 3 failed**, each for the reason it names — `Char('y') answered an
    approval that was never drawn`; `the fallback did not name the ask`; `the timeline covered the
    approval dialog`.
- **Round 2 (re-review)** — the identity-based guard and the auth key precedence; the ROUND 2
  section of `m94-focused-tui.log` records `app.rs` sha256
  `5932c6d01c892fc8d6ef3d9ed19c5719eb9e5f5431bcf7033a380f617f66609e` at the start *and* the end of
  the round: `cargo fmt --all -- --check` clean (exit 0), `cargo fmt -p epsilon-tui -- --check`
  clean (exit 0), `cargo clippy -p epsilon-tui --all-targets -- -D warnings` clean (exit 0),
  `cargo test -p epsilon-tui` **163 passed / 0 failed** (exit 0) — the 161 above plus the two
  re-review tests — and `cargo test -p epsilon-cli --lib` **38 passed / 0 failed** (exit 0), with
  the ROUND 2 section of `m94-focused-cli.log` naming the same `app.rs` hash.
  `m94-newtests-red.log` ROUND 2 — the same two tests against the **pre-fix bytes of this round**
  (the round-2 diff absent, the two test functions only): **0 passed / 2 failed**, each for the
  reason it names — `Char('y') answered ask B before any frame painted it` and `'y' must answer
  the approval, not the auth buffer`; the fixed file was preserved byte-exactly before the run and
  restored byte-identically after it (sha256 equal, `cmp` exit 0), and the restored bytes re-ran
  the same two tests green (2 passed).
- the **real-console before/after measurement** described above. Before: the installed binary the
  owner was running (the 2026-09-18 05:32 `epsilon.exe`, built from HEAD plus the then-current
  slice) — a 99-character burst echoed at ~30 chars/s, half of it missing at the immediate capture.
  After: the debug build of exactly these bytes (`epsilon-m93-target/debug/epsilon.exe`) — the same
  burst shows all characters at the immediate capture. The release build that ships is the same
  code; the loop cannot be unit-tested (it owns the real terminal), so this measurement is the
  evidence for the loop change.

**Frozen-candidate battery — label `m94c`, PASS.** `audit/evidence/gate-m94c.log`: all five gates
exit 0 (fmt 2 s, clippy 38 s, test 1839 s, demo 17 s, real-check 36 s) and the log ends
`===== GATE PASS =====`. Aggregate **863 passed / 0 failed / 0 ignored across 55 summaries**;
per-binary reconciliation against the M93 baseline (`gate-m93b`, 798/0/0/55) shows the two logs
identical index-for-index except **`epsilon_tui` 98 → 163 (+65)** — the entire delta is this
crate's own suite, nothing else moved. Candidate composition: `git archive HEAD` plus exactly the
slice's files; the other workstreams' files (`registry.rs`, `stream.rs`, `agent-contract.md`,
`write.txt`, and the `contracts/lib.rs` re-export hunks) were proven equal to HEAD
(EOL-normalized) before the build, and the shipped code bytes were re-verified hash-identical to
the candidate's after the battery (`app.rs 5932c6d0…`, `state.rs 123ebcf7…`, `picker.rs
fe8f65a9…`, `config.rs b3d3e654…`, `Cargo.toml d48d161f…`). This document was edited after the
battery to fold the round-3 re-review's findings (documentation only; the pinned code hashes are
the claim).

**Independent reviews** (read-only; delegations `deleg_aee8d4b1`, `deleg_c321b021`,
`deleg_20465716`). Round 1 found three blockers — an invisible approval answerable below 60×12 (a
regression this slice introduced; HEAD drew the dialog at any size), the timeline covering a live
approval, and unevidenced Verification numbers — all fixed, each with a test shown to fail
pre-fix. Round 2 found one narrow blocker: the visibility flag was a last-frame fact, so within
the ≤33 ms frame window a second parked ask could be answered before any frame painted it — fixed
by the identity-based guard, and the critical residual (a replaced ask resolves to **Deny** and
cannot stall a run) was verified against the channel's drop semantics. Round 3 re-reviewed the
delta and returned **no blockers**, with three non-blocking findings folded into Limits.

## Limits

- **The streaming path is verified by construction, not by measurement.** Exercising it live needs
  a real provider call (not spent without the owner's go-ahead) or a faux provider reachable from
  the CLI (queued as a harness improvement). What is proven: the loop now coalesces deltas, caps
  frames, and never draws anything on an idle TUI.
- **The loop's drain has no unit test** (it owns stdout); the real-console measurement is the
  evidence, and it is recorded in this document rather than in a test.
- **Carried over from the TUI work item** (still open, recorded in
  `audit/10-tui-enhancement.md`): paste mutating input during approval;
  unconditional composer cursor; late autocomplete drawing; guidance and the other approval
  decisions untested; and the five static objections from its earlier review (wrapped tab stop,
  full-row newline caret, scalar-editor/grapheme-renderer mismatch, frozen-viewport live growth,
  wrapper clipping long words).
- **Mouse wheel**: no `EnableMouseCapture` is set, so the app only sees what the terminal
  forwards — conhost consumes the wheel for its own scrollback, so in-app wheel scrolling does not
  work there.
- **Bracketed paste on legacy conhost** is unverified: if conhost does not translate DECSET 2004,
  a paste arrives as raw key records (the frame ceiling still bounds the damage).
- **The un-memoized per-frame file I/O is the auth-store read**, not the branch probe: the hint
  bar's `epsilon_llm::catalog::credentials_ready` check re-reads the stored credentials on every
  frame (`resolve_api_key` → the auth file) — the frame ceiling bounds how often it happens; it is
  not memoized. The `.git/HEAD` branch probe is **not** in that set: `App::cached_branch` memoizes
  it with a 2 s TTL (`app.rs:596`).
- **Blocking work reachable from key handlers** (@-file walk, `/sessions` rescan, clipboard open,
  `gh` spawn, fork/clone copies) is untouched by this slice; it needs its own measurements.
- **A second ask that parks while one is already pending replaces it** (re-review residual, **not**
  fixed in this round): `drive`'s ask arm assigns `app.pending_ask = Some(pending)`
  (`app.rs:2786`), so two asks that park without a key in between leave only the later one on
  screen. Both cases are fail-closed: the earlier ask's reply channel closes and the blocked
  `ask()` resolves to **Deny** (channel drop semantics, pinned by `dropped_ask_denies_fail_closed`
  — a replaced ask cannot stall a run, verified in the round-2 re-review). The identity guard
  cannot be fooled by it: the surviving ask still needs its own frame. Two remainders, both
  queued: (a) the denial is **silent** — the dialog content simply swaps on the next frame, so a
  *painted but unanswered* ask is denied with no notice (fix shape: a notice at the replacement
  site); (b) the seating should become a local FIFO in the run loop (drain the queue, seat one ask
  per frame).
- **Approval keys outrank the auth prompt, but `'e'` guidance still shares its input**: with an ask
  pending, `y`/`a`/`n`/`Esc` never touch the auth prompt's state (asserted by
  `a_pending_approval_preempts_the_auth_key_prompt`), while `'e'` clears `app.input`, which doubles
  as the `/login` picker's filter — so "auth state untouched" is exact for y/a/n/Esc and not for
  `'e'` (round-2 re-review finding F2; the shared-input coupling is its own slice).
- **Last-paint semantics after a shrink**: a resize does not retroactively unpaint. Until the next
  frame (≤ `FRAME_MIN`, 33 ms) the recorded id still names the ask whose dialog the *larger*
  terminal showed, so a key inside that window can answer an ask the smaller terminal no longer
  displays in full. Inherent to any last-frame guard; the following draw recomputes it (rounded
  re-review finding F3).
- **`app.interrupt` is a dead keybinding**: `keybindings.rs:28` declares it (default `escape`) and
  nothing reads it — the only other mention in the tree is its own test at `keybindings.rs:270`.
  A user who rebinds it changes nothing and gets silence. (The idle-Esc behaviour users actually
  see comes from `handle_key`'s Esc arm, not from this id.)
- **Two files in `audit/evidence/` are red runs under green names**: `tui-full-green.log`
  ("156 passed; 1 failed") and `tui-floor-green.log` ("4 passed; 1 failed"). They are pre-fix red
  runs that were never renamed; they are **not** evidence of anything green — these two logs are
  named only as non-evidence. The green runs for this milestone are `m94-focused-tui.log` and
  `m94-focused-cli.log`.
