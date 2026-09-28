# Session 2 — TUI truth, then port, metrics, structure (2026-09-28)

**Status: orientation. No product code touched by me yet.**

## What session 1 left (I verified this myself, not inherited)

- HEAD `7145384` (M193). Audit `01`–`07` exist, measurement-based, real work.
- Five defects **D17–D21 all closed and committed**: subagent tokens counted; per-frame
  transcript clone removed (144 µs → 276 ns); panic hook restores the terminal; stale
  `tui.lock` steal window 30 s → ~4 s; `epsilon-lsp` lint exception recorded.
- **The typing-freeze measured NOT to reproduce** (`f9f1d25`): 42 chars typed while the
  header read `running 2s · streaming…`, cmd *and* PowerShell. D20 (stale lock) was the
  thing that *did* match "no input, no output".
- 116,433 Rust LOC · 12 crates · 36 test files · 171 milestone docs · 4 references.

## The lead nobody followed: the word **"shrinks"** — now investigated, and RULED OUT

Owner: *"it shrinks and freezes as soon as I start typing and shows no input or output"*.
The prior audit tested **freeze** and **no I/O** and cleared both. It never tested
**shrinks**. I chased it to ground this session. **It is not a console-geometry bug.**

What I verified, by reading the vendored sources rather than the TUI's own code:

1. `grep` for `set_window|SetConsoleScreenBufferSize|SetConsoleWindowInfo|EnterAlternateScreen`
   across `crates/epsilon-tui` and `crates/epsilon-cli`: **zero hits.** The TUI never asks
   the console to change size. It cannot shrink the window.
2. `ratatui 0.30.2` `try_init()` (read in `ratatui-0.30.2/src/init.rs:397-403`) is exactly
   `set_panic_hook` + `enable_raw_mode` + `EnterAlternateScreen` + `Terminal::new`. No
   viewport option, no size call.
3. The one mechanism that *could* have done it — crossterm's Windows `EnterAlternateScreen`
   creating a **real screen buffer** via `CreateConsoleScreenBuffer`
   (`crossterm-0.29.0/src/terminal.rs:225-230` → `crossterm_winapi-0.9.1/src/screen_buffer.rs:45-65`)
   — **is not taken on this machine.** `Command::execute` picks the WinAPI path only when
   `is_ansi_code_supported()` is false, and `supports_ansi()`
   (`crossterm-0.29.0/src/ansi_support.rs:35-50`) calls `enable_vt_processing()`, which
   **succeeds in modern CMD and PowerShell**. So the VT `CSI ?1049h` is emitted — a pure
   terminal state switch that cannot move a window.

**Conclusion, and it is the useful part:** "shrinks" is almost certainly a *perceived*
frame shrink — the layout recomputing as the composer grows, or a frame the user reads as
smaller — not a window resize. The next measurement is a **frame**-geometry diff across
typing, not a window-geometry one. `Regions::split` is at
`crates/epsilon-tui/src/ui/layout.rs:79-99`; `composer_cap` at `:105-108`.

## Four theories of the freeze, four measured and disproved as *the* cause

| # | Theory | Measured | Verdict |
|---|---|---|---|
| 1 | a permission dialog silently eats keys | it renders `y/a/n/e`; `y` works | **disproved** (session 1) |
| 2 | per-frame transcript deep clone | 144 µs → 276 ns, flat | **disproved** (D18, fixed anyway) |
| 3 | the console window gets resized | TUI touches no console geometry; crossterm takes the VT path, not the buffer swap | **disproved** (this session) |
| 4 | `@` re-walks the workspace per keystroke | **4.2 ms** here, **9.2 ms** worst case = 28% of a 33 ms frame | **real cost, not the freeze** |

Theory 4 is the one the port study ranked first, and it is genuinely there:
`completion.rs:43` re-derives whenever `at_fragment` changes, and `at_file_rows` calls
`at_files::walk` every time. The ignore *rules* are cached for 5 s (`AT_IGNORE_TTL`); the
walk is not. The code's own comment says "this runs once per keystroke" — the authors
knew. But `AT_ROW_BUDGET = 200` caps it, so the cost is bounded and small. **Worth caching,
not a freeze.** Four theories down, and the honest position is still "not reproduced".

## The fourth inert control: an assertion that could not see the thing it named

The `@`-cache test passed **three times against a control that re-walked on every
keystroke**. Each version was wrong in a different way, and each was invisible until the
control was armed:

1. **Asserted the cached list's contents before and after.** Identical either way — a
   re-walking implementation still produces the same list for the same fragment. Said
   nothing about the traversal.
2. **Asserted the cache field is untouched.** A re-walking implementation never *writes*
   that field, so it is untouched in both cases. Read the wrong side of the change.
3. **Asserted the popup's rows, with a fixture that could not match the query.**
   `fuzzy_filter` splits on `/` and requires every token, so `created_while_typing.rs`
   against the query `src/app` was absent from the popup whether or not the walk ran —
   **vacuous in both directions**. The control passed.
4. **Renamed the fixture to `src/app_created.rs` so it MATCHES the query.** The control
   then failed with `shown=["src/app.rs", "src/app_created.rs"]` — the re-walking
   implementation offered the file, the cached one did not.

**The rule, now stated once and worth more than the fix:** a control that passes is a
control you have not yet proven works. And an assertion is only worth what it can
*distinguish* — ask what the broken version would produce, and confirm it is different,
before believing a green test says anything. A fixture that cannot match the query tests
nothing in either direction.

**Method note:** measured with a standalone script in scratch, not a Rust test.
`write_file` silently failed to land a test file twice this session, and `patch` then
matched fuzzily and corrupted it. For a throwaway measurement, a scratch file *outside* the
repo is safer than one inside it. Verify what is on disk before believing a compiler
error about a file you have just written.

## M194 — the delivered set, and the gate that passed

**GATE PASS `m194` — all five gates `exit=0` on a frozen tree: fmt, clippy, test, demo,
real-check.** Suite **1,491 passed / 63 targets / 0 failed** (counted from the log, not
estimated). Log copied byte-identical to `audit/evidence/gate-m194.log` (124 KB) because
`target/` is wiped on every cleanup round — the disk rule and the evidence rule collide
here, and the copy is the resolution.

The three killed batteries (`m194-d30`, `m194b-d30`, `m194c`) each edited source while
running. **Three logs lost to that in one session.** Freeze the tree before a battery; a
log from a moving tree is evidence of nothing. Full workspace suite: **1_491 passed, 63 targets,
0 failed** (measured from the log, not estimated — I summed the `test result: ok. N
passed` lines rather than quoting a number I had not counted).

| Area | Change | Why it mattered |
|---|---|---|
| D30 | `SessionState` → pi's camelCase; `QueueMode` split out of core | the inherited, never-committed, never-gated work |
| D31 | `cycle_model` replied `model_id`, `set_model` replied `modelId` | a client read `undefined` after a well-formed frame |
| D32 | `set_model` had no alias for `modelId` | **pi's own documented frame was refused** |
| K1 | `Usage::cache_reported`; header prints `CH n/a` | "cannot measure" no longer wears "measured 0%" |
| C2 | fixed prefix counted on the no-anchor fallback (+1_633 tokens) | the gauge read low exactly after a compaction |
| R1 | `ctx 20%*/200k` marks an estimate | the figure says how it was made |
| $2 | compaction bill reaches the live total | the cost no longer rises on `/resume` |
| G1 | `app.clear` actually clears; `app.exit` is ctrl+q | `/hotkeys` no longer prints a binding that does nothing |
| G2 | `/fork` + `/clone` use `fork_branch_at` | was `fs::copy`: same session id, whole file |
| G3 | branch jump stops dumping 400 notices | it scrolled the reader off the message they jumped to |
| G9 | `/expand` is a real command; `ctrl+x` is the honest hint | a palette entry that silently did nothing |
| G10 | an unbound key says so | "nothing happened" is no longer a reachable UI state |

## Traps hit while landing these (all caught by existing tests, not by me)

- **`no_slash_command_panics_when_typed` is a good test.** It caught that my new
  `/expand` left no transcript line. It is the sweep that would catch the next one.
- **`app::golden::ctrl_x_opens_the_newest_card…` is byte-exact.** A `push_notice` I added
  to the *shared* method moved the very card the test is about. The fix was to report from
  the *command* arm and leave the *key* path untouched — a shared helper is the right
  shape for the behaviour and the wrong place for the presentation.
- **A `..metrics()` spread silently overrode the fields a test was about.** Three of my
  own assertions were testing the fixture, not the property. Build from `default()` when
  the spread would win.

## Five wrong thresholds before I measured one (the most expensive lesson this session)

`a_compaction_keeps_the_users_own_words` broke on C2. I tried **five** thresholds
(1_800, 3_000, 4_000, 4_500, 1_500) reasoning from the source, and **every one was wrong
in the same direction: each assumed the estimate had *shrank* when it had grown.**

What one `eprintln!` in `compaction_due` actually showed:

```
TRACE due: tokens=2150  limit=1200  tail=2000
```

- `tail` is **clamped up** to `MIN_TAIL_TOKENS = 2_000` (`clamp_tail_tokens`,
  `lib.rs:52-57`) — the fixture's `200` was never in effect. I did not know that, and I
  spent three attempts tuning a number that could not fix it.
- The budget has to sit **above** the first turn so the user's prompt lands outside the
  tail. Fixed by preserving the **relationship** (budget above the turn, tail below it),
  not the digit — a number derived from today's measurement is correct today; a
  relationship stays correct.

**Isolation is cheap and I should have reached for it first.** Setting `let fixed = 0`
for one run proved C2 was the cause in 8 seconds. A `git worktree` at HEAD proved the
*other* failure was mine rather than pre-existing. Both took minutes; the guessing took
an hour.

**And: measure the number the code reads, not the number you reasoned about.** Also note
`write_file` silently failed to land a probe file twice in a row while reporting success
— I only caught it because the compiler kept reporting an error against the old
contents. The memory note about this was right, and I still hit it.

## C2 has a REAL consequence, and a test that was asserting the regression away

`compaction_due` (`epsilon-core/src/lib.rs:2169`) reads `estimated_context_tokens`. So when
C2 started counting the system prompt + tool schemas on the no-anchor fallback, the
pressure trigger **correctly fired earlier** — and a fixture that performed one compaction
now performs two, so `usage_accounts` sums 300 instead of 200.

`a_compaction_records_its_bill_on_its_own_entry` asserted `compaction.input == 200`. I
**proved the causation rather than assuming it**: `git worktree` at HEAD, separate
`CARGO_TARGET_DIR`, same test → **PASSES with 200**. So this was mine, not pre-existing.

The tempting "fix" was to make the number 300. That would have been **asserting the
regression**: a *less* accurate context estimate. The test's own comment already warned it
had failed twice for this reason. Asserted the **property** instead — a billed compaction
has `input > 0` and `output > 0`, which is F23 (the bill being dropped entirely), the
defect the test was written to catch.

**This is the shape to watch for in every accuracy fix:** making a number more truthful
moves thresholds that were calibrated against the lie.

## R1 (references study) — provenance: the header now says how each number was made

The references study's own top recommendation, implemented: `Agent::context_is_estimated()`
(`epsilon-core/src/lib.rs`) returns true whenever `anchored_tokens` would take its `_ =>`
arm. Mirrored per frame beside `context_now` in `App::mirror_context_size`, carried on
`Metrics::context_estimated`, and rendered as a `*` on the **percentage** only —
`ctx 20%*/200k` measured vs estimated. The window is a catalog capacity and is a
measurement whenever known, so the mark never rides on it.

Deliberately derived from the **same guard the estimator applies**, not from a separate
flag: two derivations of one rule beat a flag that can disagree with the value it labels.
Test: `the_figure_is_measured_exactly_when_the_anchor_survives` drives a real `Agent`
through real turns, with a reporting provider and a silent one.

This is the shape Hermes uses (`context_breakdown.py:94-115` publishes `context_source` +
`context_estimated` on every status payload) and the reason DSH calls its occupancy figure
"a user-facing reference, not a billing or gating input" — a figure is only useful to a
user if it says how it was made.

## K1 — cache hit rate: "not measurable" was rendered as "measured zero" — FIXED

**Goal #2 of the owner's brief: accurate, real-time context / cache / cost.**

The defect, confirmed by reading the code (not inherited): for any wire that reports no
cache fields — the faux provider, ollama, an OpenAI-compatible proxy that omits
`prompt_tokens_details` — `cache_read` is structurally 0, and the header printed
`R0 W0 CH 0%` in the **Value** style. That reads as "a cache that is present and
failing". The codebase had even *named* the failure mode in a comment
(`epsilon-llm/src/providers/mod.rs:1543`) without fixing it.

The fix, and why this shape:
- `Usage` gains `cache_reported: bool` (`epsilon-contracts/src/message.rs`) — set **only**
  by a decoder that saw a cache **key** in the body. Default `false` is the safe
  direction: an unannotated decoder produces "not measurable", never a fabricated zero.
- Deliberately per-**call**, not per-provider: a gateway can route one model id to
  backends that report differently, and only the reply knows which one answered.
- `combine` ORs it (not AND — a tail of unannotated calls would erase the evidence).
- `Metrics` carries it; `cache_hit_rate()` returns `None` when it is false.
- Three distinct renderings now: `CH 75%` measured · `CH —` measurable-but-no-prompt-yet
  · **`CH n/a` not measurable**. Volume (`R…W…`) still shows in every case.

**Correction to `audit/05-metrics.md`, verified myself:** the audit claimed "nothing in the
`Usage` type records whether a provider is able to report cache tokens" and that subagent
tokens were never counted. Both are **stale** — `CacheStats::reports_cache_traffic()`
exists at `epsilon-core/src/cache_stats.rs:182` (the TUI simply never consulted it), and
`record_tool_usage` already counts subagent tokens (commit `12ae518`). Do not re-report
either as open.

## Port fidelity vs pi 0.85.1 — the study, and the four gaps I fixed

Study: `audit/evidence/port-fidelity-study-2026-09-28.md` (delegated, then I re-verified
the load-bearing claims myself). **84 pi TUI/loop capabilities surveyed: 38 DONE, 22
PARTIAL, 24 MISSING.** The 24 missing concentrate in one place: everything pi renders
through a `Component` Epsilon has no equivalent of.

**G1 — `app.clear` was a binding that did nothing.** `keybindings.rs:32` bound it to
ctrl+c, `/hotkeys` printed it, and **nothing read it** — while `app.exit` matched the same
chord first. So Ctrl+C quit with the draft in the buffer and the key table lied.
I verified this myself (`grep 'app.clear'` → exactly one hit, the defaults table).
Fixed: real `clear_editor()`; `app.clear` matched *before* `app.exit`; `app.exit` is now
ctrl+q alone, so no chord names two actions. Tests go through `handle_key`, not the method
— a direct call cannot see whether anything calls it.

**G9 — a palette entry that silently did nothing.** `picker.rs:1005` carried
`"\u{1}expand"`, but `parse_command` requires a leading `/` and returned `None`; choosing
it closed the palette, cleared the composer, ran nothing, no notice. Its `binding: Some("x")`
hint was also wrong (the chord is ctrl+x, and a bare `x` is the binding that was *removed*
so the letter stays typable). Fixed with a real `/expand` command + shared
`toggle_tool_expansion()` so key and menu cannot drift.

**G10 — unbound keys were silent.** `keys.rs` ended in `_ => false`. Fixed to notice
`"<key> is not bound here — /hotkeys lists what is"`. This is the invariant the whole
investigation produced: **"nothing happened" must never be a reachable UI state** — a
swallowed keystroke is indistinguishable from a frozen program.

**G2/G3/G4 (verified, NOT yet fixed — the next work):** `/fork` and `/clone` in the TUI are
`std::fs::copy` (same session id, whole file) while `store.rs:1511 fork_branch_at` is
correct and is wired **only** from RPC. Branch navigation dumps the whole branch as notices
after jumping. `branch_with_summary` still has no production caller — now the **fourth**
independent confirmation; it is a faithful port that pi calls during navigation.

## D32 — a pi client could not send pi's own field name (found by my OWN test being wrong)

Writing the D31 test, I sent `{"op":"set_model","modelId":"faux-1"}` — the frame pi
documents (`rpc-types.ts:33`) — and got **"set_model needs a model"**. The field was
`model_id` with **no alias**, because the enum carries `rename_all = "snake_case"`. So
every pi client sending the documented frame was refused, with an error that blames the
model rather than the key. `fork` right below it already had `rename = "entryId", alias =
"entry_id"` — `set_model` was the lone miss.

**This is D26 (`follow_up`) recurring on a field instead of a variant**, and it survived
for the same reason: the tests built the frame from the *Rust field name*, so they agreed
with the code by construction and could never see the wire.

**The lesson worth more than the fix:** I twice "fixed" my own test's frame by guessing a
key (`model`, then `model`) and both guesses were wrong. The third attempt used pi's
*documented* frame and found a real product bug. **When a test fails on a frame, check
whether the frame is the bug before concluding the code is** — but check it against the
*reference*, never against the Rust identifier, which is the thing that was wrong.

## D31 — two ops, one question, two spellings (found by a FAILING test, not a reading)

`set_model` replied `modelId`; `cycle_model` replied `model_id` (`rpc_mode.rs:766`). Both
frames well-formed, both tests green — and a client that read `modelId` after setting a
model got a value, cycled, and read **`undefined`**. No error; the field just stopped
existing.

It surfaced only because the full suite ran: `cycle_model_stays_within_the_current_provider`
asserted on `modelId` and failed with `right: ""`. **A test asserting a wire key is worth
more than a test asserting behaviour** — the behavioural assertion passed on both spellings.

Fixed to `modelId` (pi's spelling, `rpc-types.ts:33`). The regression test compares the
**two ops to each other** rather than to a constant — the whole point is that one of them
was wrong, and pinning a literal passes against the broken build. That is the AGENTS.md
rule "when two paths serve the same reference type, compare THEM to each other", now
earned a fourth time.

## In-flight uncommitted work — D30, now complete and gate-verified

8 modified files + untracked `crates/epsilon-contracts/src/queue_mode.rs` (145 lines).
The diff is **D30**: `SessionState` moves `rename_all` from `snake_case` to `camelCase` to
match pi, and gains `steering_mode`/`follow_up_mode` from a new `QueueMode` enum;
`epsilon-server/openapi.rs` changes with it deliberately, so the two transports cannot
disagree (the D29 lesson). **Never committed, never gated.** Must compile-check first.

## Machine (re-verified this session)

**Build on C: only** (owner's rule 2026-09-28). No `CARGO_TARGET_DIR`; `JOBS=1`.
`pwsh -File scripts/gate.ps1 <label>` is the battery. `D:/eps-target` is deleted and must
never be recreated — see [`05-machine.md`](05-machine.md) for how it got there and what to
check before trusting a path in a note.
**One build owner at a time** — a gate and a delegated build must never share the target.

## Next safe action

1. Finish `cargo check` on the in-flight tree; land or deliberately park D30.
2. **Measure "shrinks"** — `scripts/console-probe.py` recording window *and* buffer
   geometry before start / after start / after typing, at more than one width.
3. Then: TUI correctness → metrics truth → port parity → repo layout + docs rewrite.
