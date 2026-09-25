# Open items re-derived at HEAD (`4fbca81`), 2026-09-25

**Why this file exists.** The notes in `agents/memory/` had drifted: they listed blockers B2–B5, the
`auth.lock` retry, and the session-format gaps 4A/4B as "still open", and all of them had been fixed a
round ago (M130; `auth.rs:278-289`; M113 and M119). A queue that lies about what is open costs more
than no queue, so four read-only passes re-derived the open items **from the code** rather than from
the documents — and the code is what is recorded here.

**How to read it — the tiers are the point.**

- **§1 Verified.** I read the cited lines at HEAD myself. Safe to act on; the file:line is current.
- **§2 The other OPEN ledger rows.** A read-only pass re-verified all seven of `audit/02`'s OPEN rows. I
  independently confirmed three (F03, F05, F11 — in §1); the rest are that pass's own reading, and each
  row names its effort and the decision it needs, which is more useful than a yes/no.
- **§3 Port fidelity.** Still-open pi-port rows, with the closed ones struck through so the next reader
  does not re-check them. Unverified beyond the §1 items.
- **§4 TUI.** Ten findings from a dedicated pass. Ranked; §1.2 is the only one I verified, and each
  carries its own evidence.

Nothing here is a task list with owners. It is a map of what is open, verified as far as it says it is.

---

## 1. Verified at HEAD — safe to act on

### 1.1 F03: several SSE frames in one HTTP chunk lose all but the first — **P1, live streaming path**

`WireStream::ingest` (`crates/epsilon-llm/src/providers/mod.rs:780-819`) drains complete lines from
`line_buf` in a `while let Some(pos) = …position(|b| *b == b'\n')` loop, but `return first;` sits
**inside** it (`:815`). Every complete line after the first event-producing line therefore stays in
`line_buf`. `next()` (`:823-848`) pops `self.queued` and, when that is empty, awaits
`self.inner.chunk()` without re-checking `line_buf`.

A provider that delivers a whole response in one TCP chunk loses every frame after the first; if the
body then ends, `next()` returns `None` and **the stream ends cleanly** — the "EOF claims success"
half of the finding, reachable with no slow server at all. EOF feeds the `line_buf` remainder as one
line (`:869-873`), and `:951-963` synthesizes `Done` from whatever text and open tool calls exist.

**Why the fix needs a seam and not just a unit test.** `tests/wire_fixtures.rs` drives
`HttpEventStream` and the line helpers directly and never constructs a `WireStream` — which needs a real
`reqwest::Response`. That is the ledger's own "coverage gap" sentence, and the reusable `LineReassembler`
at `mod.rs:1432` is a `test_support` type production does not use. So: make `ingest` drain every complete
line (queue all, return one), and put the body stream behind a mockable seam, which also makes the M60
idle watchdog testable for the first time. Effort M.

### 1.2 A dead `app.clear` binding that `/hotkeys` advertises — **one line**

`crates/epsilon-tui/src/keybindings.rs:32` declares `("app.clear", &["ctrl+c"])`; the line above is
`("app.exit", &["ctrl+c", "ctrl+q"])`. `grep -rn '"app.clear"' crates/` returns the declaration and
**nothing else** — no key handler matches it. `/hotkeys` prints the live table, so the diagnostics
screen tells the user Ctrl-C quits *and* clears, and the clear never happens: `app/keys.rs:437` checks
`app.exit` first. Delete the row, or give it a chord and a handler. Do not leave a table that documents
a key the product does not implement.

### 1.3 pi's `ignoreCase` silently does nothing — **fidelity, one line plus a test**

`crates/epsilon-tools/src/search.rs:48` is `pub ignore_case: bool`, with no `#[serde(alias)]` and no
`#[serde(deny_unknown_fields)]` anywhere in `crates/`. pi names the flag `ignoreCase`
(`references/pi-0.85.1/packages/coding-agent/src/core/tools/grep.ts:25`). A model carrying pi's tool
description sends `ignoreCase`; serde drops the unknown field, the search runs case-**sensitively**,
and the tool reports success. `files.rs:388-391` already handles pi's `oldText`/`newText` this way, so
the pattern is in the tree.

### 1.4 F05: a failing command is recorded as a passing check — **verified**

`crates/epsilon-core/src/exec.rs:717` is `let is_error = outcome.is_err();` — transport only. A shell
command that exits nonzero returns `Ok(ToolOutput)` carrying `details.exit_code`
(`epsilon-tools/src/shell.rs:350-356`), so `lib.rs:1646-1651` computes `ok = !r.is_error` → `true` and
hands it to `evidence.observe`, which accepts a check when `c.ok && c.turn >= *turn`
(`verify.rs:175`). Both halves hold: a red test run satisfies a "checks pass" claim, and `turn_index`
increments per turn, so a check recorded *before* a later edit in the same turn's batch still covers it.
Needs a third outcome (pass / fail / unknown) and a per-call sequence number. Effort S–M.

### 1.5 F11: the bwrap backend ignores its policy and has no deadline — **verified**

`crates/epsilon-safety/src/sandbox.rs:390` takes `_policy: &ConfinementPolicy` — underscore-prefixed,
never read — then `:433-436` is a bare `child.wait().await`, and `grep -c kill_on_drop` over the file
returns **0**. Cancellation elsewhere merely drops the future. A descendant holding stdout after the
direct child exits hangs past every promised deadline. Effort M, and it needs Linux CI to close.

## 2. The other OPEN ledger rows — the pass's reading, not mine

| Row | Still real, because | Effort |
|---|---|---|
| **F09** | `resolve_in_workspace` is lexical by design (`paths.rs:12`); `gate_subject` (`exec.rs:113-123`) decides permission with the same walk, and `ensure_inside` (`files.rs:300-306`) is the whole native gate. | **L — a design task, not a patch** |
| **F12** | `drain_pipe` grows an unbounded `Vec` on both platforms and `truncate_output` runs after it; the image path is bounded for detection but re-reads the file whole; `webfetch` buffers the body before its own size check. | M |
| **F14** | `version` is a literal `2` on every `didChange` (`lsp/client.rs:255`); the `send` happens before the publish timeout is armed (`:259-262` vs `:268`); a timeout returns the last cached diagnostics for the URI, described as current. | S–M |
| **F16** | The threshold mismatch is intact: `oauth_needs_refresh_in` refreshes at `expires <= now + min_validity + 60_000` (`auth.rs:643`) while `store_refreshed_oauth_in` discards a fresh pair under a different one (`:675-676`); Copilot declares `client_id_env: ""` (`oauth.rs:86`), so `refresh_oauth_before_use` (`providers/mod.rs:1174-1176`) returns early and **Copilot never mints at call time**, even though `exchange_refresh_token` handles the mint without a client id. | **M/high — three entry points own credential prep and the fix cannot be local** |

**F09's decision, since it is what blocks that row:** *is the unit of authorization the path string or
the filesystem object?* Canonicalize-then-open is TOCTOU, so the real options are no-follow
handle-relative access, refusing traversal through a reparse point, or allowing linked workspaces behind
a prompt that names the resolved target. One lexical normalizer is load-bearing for three contracts with
different failure costs (permission, resource-conflict key, read-coverage stamp), which is why it cannot
be changed in passing. `audit/04-roadmap.md:84` already says to stop if canonicalization is being
presented as race-resistant.

**Three ledger citations have drifted** (fix when those rows are next touched): F05's `lib.rs:1173-1179`
→ `:1646-1651` and `verify.rs:151-175` → `:133-178`; F14's `files.rs:467-511` → `:515-565`.

## 3. pi-port rows still open, with the closed ones struck

**Closed since the last recheck** (do not re-audit): row 2 queue defaults (M85), row 3 turn callbacks
(M89), row 6 session trees (M113–M116), row 8 compaction and pressure (M86/M122/M124/M133). Row 11's
specific claim — "no abort, no request ID" — is **false now** (`rpc_mode.rs:52-58`, `:71-80`); the
broader gap (no model/session operations) survives.

Ranked by what a user meets first:

1. **Loop tuning is not configurable (row 15).** `LoopLimits` carries the queue modes, turn cap,
   compaction thresholds and four guards (`core/src/lib.rs:153-189`) and *nothing reads it from
   config*: the only keys consulted anywhere are `[tui] theme` and `[editor] command`. A user cannot
   raise the compaction threshold on a large repository without editing source. pi ships
   `CompactionSettings` beside its session settings.
2. **Branch navigation is a timeline, and fork does not move you (row 7).** The store machinery is
   complete and unused: `navigate_to`, `reset_leaf`, `branch_with_summary` and `fork_branch_at` have one
   caller between them (the picker), which moves the leaf and writes no summary; `/fork` is a raw
   `fs::copy` that tells you to relaunch. After exploring a wrong path the model's context changes
   silently and nothing records what the abandoned attempt learned.
3. **Skills are found one level deep, and collisions resolve backwards (row 14).** `skills::discover`
   reads direct children only, so a `SKILL.md` under a category directory never appears; on a collision
   it **replaces** the earlier entry and reports nothing, while pi keeps the first and emits a
   `collision` diagnostic — and the doc comment at `skills.rs:65` claims this "mirrors pi". An invalid
   name is dropped silently where pi warns.
4. **`.epsilon/prompts/` and `.epsilon/themes/` are trust-gated and then ignored (rows 13, 12).** The
   runtime enumerates both purely to decide whether to prompt for trust, and no reader exists for
   either. Asking a user to trust a directory and then discarding it is worse than not asking.
5. **Twelve tools where pi hands four (row 5's remaining half).** Twelve unconditional registrations and
   no subsetting; plus §1.3's `ignoreCase`.
6. **No Codex/Azure/Cloudflare/Radius transports (row 9).** Total for a ChatGPT-subscription user,
   invisible to everyone else; `openai-codex` is withheld by design.
7. **Extensions can run tools, never contribute UI or commands (row 10).**
8. **Images attach unresized (row 17).** No resize function exists; the TUI's placeholder is honest about
   it, so this costs request budget rather than trust.

**One dead seam worth a look:** `register_builtins_with_skills` (`epsilon-tools/src/lib.rs:99`) has zero
callers, while `epsilon-cli/src/main.rs:1700` registers `SkillTool` itself. Verify before assuming the
wiring is intended.

## 4. TUI — what a user would still find poor

1. **The duration test still pins `0ms`.** The B1 *code* fix landed (`state.rs:576-578` inherits the
   call's start), but the golden test applies a tool result with no preceding call, so `settle_call`
   returns `None` and the drawn frame is `0` — the test named for the property passes on the exact value
   the review called a defect. Two-part fix: assert a real span through a call+result pair, and make the
   orphan case honest (no number) rather than a fabricated zero.
2. **`/help` and `/hotkeys` print literal chords** (`app.rs:2252-2260`, `:1291-1298`) while the footer
   resolves through the table. `/hotkeys` also advertises `Ctrl+U/Ctrl+W/Ctrl+Y/Ctrl+F`, which are not in
   `DEFAULTS` at all — they are hardcoded match arms. The mechanism to fix this already exists
   (`KeybindingsManager::key_text`); the screens just do not use it.
3. **The composer's prompt glyph is the system-notice bullet** (`ui/composer.rs:85`), so the input line
   and the user's own transcript row disagree about who is speaking, in the same column, at the same
   time — and no test asserts the composer's glyph at all. One-line fix plus the missing assertion.
4. **No runtime theme toggle.** `ThemeMode::flipped()` has zero production callers (only its own test),
   `set_palette` has no key that calls it, and there is no `/theme`. The palette is start-only.
5. **`SLASH_ITEMS` and `parse_command` are two tables with no test comparing them**, so a command added
   to one and forgotten in the other is completely invisible — it works and is undiscoverable.
6. **`cache_read_session` / `cache_write_session` are computed every frame and never drawn.** Either
   emit them at a width breakpoint or delete them; the third option is what is happening now.
7. **`CardStatus::Denied` is never constructed.** A denied tool call renders as `failed`, which is a
   different claim about what happened — the tool did not fail, the user refused it. That is the whole
   reason the variant exists.
8. **The welcome screen hardcodes `ctrl-p`** for the palette, on the first screen a new user reads.
9. *Not a defect, and flagged so nobody "fixes" it:* the frame ceiling, bounded input drain, dirty-flag
   frames and the idle park are right. An idle TUI repaints zero times.

**Genuinely good, do not redo:** the event loop; the render cache (O(visible) per frame, with the
subtle prefix truncation commented and correct); decoration living outside the cache; the gutter/band
system's no-colour fallback markers; diff rendering; the `whole_span` mechanism, where a truncated
`^ 12/40` would be a *wrong number* so the badge is emitted whole or not at all; cost arithmetic with a
derived `is_floor`; approval routing, where a too-small frame clears the painted approval so no key can
authorise work the user never saw.
