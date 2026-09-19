# Reference-harness ideas worth adopting (2026-09-19)

Epsilon baseline: `crates/epsilon-tui` (ratatui `=0.30.2`, crossterm `=0.29.0`, `event-stream`),
`crates/epsilon-core/src/{cache_stats,compact}.rs`. The footer already has
`context_parts`/`ch_rate`/`stats_parts` (`crates/epsilon-tui/src/footer.rs:101,71,130`); the gaps below are
architecture, not formatting.

## Method (which files you actually read, per reference)

Four parallel read-only deep-dives, each with a backing findings file in `.tmp-console/research/`
(`pi-`/`dsh-`/`hermes-`/`opencode-findings.md`) holding the complete anchored file inventory — the evidence for
every anchor below. Paths are relative to each reference's root; "not found" statements name the searches run.
No reference's TUI is Rust: pi is TS with its own ANSI renderer, hermes forks Ink, opencode drives an
un-vendored engine (internals unverifiable), and DSH has no TUI at all — so only policies transfer, not code.

- **pi-0.85.1** (TS/npm): `coding-agent` footer-data-provider/cache-stats/usage-totals/agent-session/compaction; interactive `footer.ts`; the whole `packages/tui` render + terminal layer (`tui.ts`, `tui-main-screen.ts`, `tui-alt-screen.ts`, `terminal.ts`, `stdin-buffer.ts`); the `native/win32` console-mode addon; the sigwinch regression test; `tui-plan.md`.
- **deepseek-harness v0.1.6-alpha.2** (root `references/deepseek-harness-dsh-v0.1.6-alpha.2/deepseek-harness-dsh-v0.1.6-alpha.2`): `llm/token-meter` projections + estimator; DeepSeek chat-completions wire translation; `ui-chat` token formatter + stats pills; conversation assembler + session notifier; `compaction-basic/src/region.ts`; JSONL session-persistence Windows file.
- **hermes-agent 2026.9.14** (Python core + vendored Ink fork): `hermes_state_{usage,common}.py`; `agent/{usage_pricing,context_breakdown,context_compressor}.py`; `hermes_cli/{cli_status_bar_mixin,stdio}.py`; the `hermes-ink` fork (`ink.tsx`, `terminal.ts`, `constants.ts`, `components/App.tsx`, `clearTerminal.ts`); `appChrome.tsx`; `useVirtualHistory.ts`; the Windows-footgun lint gate; native-Windows platform docs.
- **opencode 1.18.31** (TS/TSX, SolidJS + un-vendored `@opentui/core` 0.4.5 — **not** Go): `packages/tui/src/{app.tsx,terminal-win32.ts,context/sdk.tsx,component/dialog-status.tsx,component/prompt/index.tsx,feature-plugins/sidebar/context.tsx}`; `packages/core/src/{models-dev.ts,session/compaction.ts}`; the inline-mode run lifecycle.

## 1. Observability: context remaining / cache hit rate / cost

**No reference has a `/cost` command, and one has no money accounting at all.** pi (`slash-commands.ts:19-43`)
and opencode ship none; hermes has `/status /context /usage /insights /topup` but deliberately no `/cost`
(`hermes_cli/commands.py:51-306`); DSH has no observability command and no pricing whatsoever. Epsilon is not
behind on the *command*; it is behind on the *number*.

### pi 0.85.1
- **Hybrid context: provider truth for the prefix, chars/4 for the delta.** `agent-session.ts:3419` →
  `estimateContextTokens`; `estimate.ts:91-98` charges `CHARS_PER_TOKEN = 4` (images 4800 chars) for every
  message after the last valid assistant `usage`, anchored at `compaction.ts:146`
  (`usage.totalTokens || input+output+cacheRead+cacheWrite`). Shown as `NN.N%/<window> (auto)` — never
  tokens-remaining (`footer.ts:150-161`), toned `>90` error / `>70` warning (`:154-160`), `?/<window>` after
  compaction until the next response with the reason in a comment (`agent-session.ts:3390-3416`). `CH%` is
  latest-message-only: `cacheRead/(input+cacheRead+cacheWrite)` (`footer.ts:95-98,134-136`).
- **Cache-miss attribution — the strongest idea here.** `cache-stats.ts:56-90 detectMiss`:
  `missedTokens = min(prevPrompt, prompt) - cacheRead`, floored at `NOISE_FLOOR_TOKENS = 1024` (`:11`),
  returning `{missedTokens, missedCost, idleMs, modelChanged}` (`:14-23`) — the *cost* of the miss, not just
  its existence; cumulative via `computeCacheWaste` (`:138-140`). Compaction/branch entries reset the
  baseline; **model switches are deliberately not exempt** (`:113-119`). The sticky `reportedCache` bool
  (`:42-47,66`) distinguishes "provider never reports cache" from "provider reported cache, this turn missed
  everything". Data: per-call `usage.{input,output,cacheRead,cacheWrite,cost.*}` + `provider/model` +
  `timestamp` + the entry list; cost is float USD from a static build-time catalog (`ai/src/models.ts:891`) and
  unknown models silently zero-rate (`provider-composer.ts:159`). Weakness: the footer rescans every persisted
  entry each frame (`footer.ts:88-104`).

### deepseek-harness
- **Cleanest occupancy model of the four.** Two last-wins fields: `pressureTokens` = "Provider-reported prompt
  size of the most recent request" and `projectedTokens` = "what the NEXT request's prompt would cost"
  (`llm/token-meter/src/projection.ts:30-47`). Numerator `usage-projection.ts:78 pressureFrom(usage) =
  input + cacheRead + cacheWrite` — **prompt side only, output excluded** (`:161-163`). Denominator from a
  logged route record (`request/context → event.data.contextWindow`, `:181-190`; emitted
  `core/agent-loop/src/agent.ts:598`). Only the delta is estimated:
  `projectedTokens = max(0, pressureTokens + surfaceTokens - sampledSurfaceTokens)` (`:208-216`). The JSDoc
  calls it "a user-facing reference, not a billing or gating input" (`:27`) and says the two halves are
  deliberately **not** one atomic observation (`:22-28`).
- **Honest cache percentage**: disjoint buckets, `inputTokens = prompt_tokens - cacheRead`
  (`chat-completions/translate.ts:47-71`, because DeepSeek's `prompt_tokens` *includes* hits);
  rate = `cacheReadTokens/(uncachedInput+cacheRead+cacheWrite)` (`StatsPills.tsx:109-121`), formatted by
  `token-format.ts:67-98` whose binary-search tail exists so `99.6%` never prints as `100`. And it **refuses to
  invent money** — no price table, absence documented in code (`llm-llm-pi-ai/src/catalog.ts:32-37`).

### hermes-agent
- **Money is `decimal.Decimal` end to end** (`usage_pricing.py:7,15-22,95-122`) with a per-snapshot
  `pricing_version` (`anthropic-pricing-2026-05`, `deepseek-pricing-2026-09-10`), context-tiered rates
  (`input_cost_per_million_above` + `tier_threshold_tokens`, `:104-112`), and **`cost_status` separating
  billed from estimated** (`hermes_state_common.py:363-365`); totals resolve
  `COALESCE(actual_cost_usd, estimated_cost_usd, 0)` (`hermes_state_usage.py:387-400`). The $0.25 cost cap
  (`empty_response_guard.py:56`) throttles *retries*, not spend, and `/usage` prints no cost line
  (`cli_info_mixin.py:664-743`) — per-session cost appears only in the desktop sidebar.
- **Context carries its provenance**: `{context_used, context_max, context_percent, context_source,
  context_estimated}` with `context_source ∈ config|default|detected`; `context_display_source` returns
  `"local_estimate"` iff the shown number differs from the provider's (`context_breakdown.py:94-114`), and
  **compaction never triggers on an estimate** (`context_compressor.py:2480-2484`). **Zero cache reads hide the
  segment rather than printing a misleading 0%** (`cli_status_bar_mixin.py:368-375`). Usage is recorded
  per-API-call through one chokepoint (`hermes_state_usage.py:304-309`) into
  per-`(session, model, billing_provider, billing_base_url, billing_mode, task)` rows
  (`hermes_state_common.py:420-440`), so a mid-session `/model` switch cannot misattribute tokens.

### opencode 1.18.31
- Context = `input + output + reasoning + cache.read + cache.write` of the last assistant message over
  `model.limit.context` (`feature-plugins/sidebar/context.tsx:19-44`, duplicated at
  `component/prompt/index.tsx:264-282`). **This denominator is wrong** — output/reasoning are not in the next
  prompt, so it overstates occupancy; worth knowing precisely so Epsilon does not copy it. `/status`
  (`component/dialog-status.tsx:53-165`) shows MCP/LSP/formatter/plugin counts only; pricing is a
  **fetched + disk-cached + build-time-snapshotted dataset resolved disk → embedded → network**
  (`packages/core/src/models-dev.ts:79,136,198-206`).
- **The sharpest cautionary finding in any reference: the displayed number and the trigger number are
  different, and nothing says so.** The V2 auto-compaction trigger is a 4-chars-per-token heuristic over the
  serialized request (`packages/core/src/util/token.ts:3-5`, used at
  `packages/core/src/session/compaction.ts:83,237-241`), while the UI shows provider usage; the legacy V1
  trigger uses provider tokens instead (`packages/opencode/src/session/overflow.ts:31-33`). Separately, the V2
  runner **hardcodes per-step `cost: 0`** (`packages/core/src/session/runner/llm.ts:338`) — a live cost gauge
  that silently reads zero.

### Cross-reference: the best composite design (recommendation)
One struct, four independently-sourced numbers, each honest about provenance.
1. **Occupancy — keep DSH's split.** `pressure_tokens` (provider, prompt side only) as anchor + chars/4 for
   only what was appended since = `projected`. Epsilon already owns this and does not show it:
   `compact.rs:24-46` (`UsageAnchor`/`anchored_tokens`) vs the footer's raw `latest_prompt_tokens`
   (`footer.rs:178-182`) — after any tool call the displayed figure is stale by exactly the delta.
2. **Cache — keep the rate, add pi's waste, add DSH's honesty.** `prompt = input + cache_read + cache_write`;
   rate = `cache_read/prompt`; plus `missed_tokens/missed_usd/miss_count` from `cache_stats.rs` with pi's
   1024-token floor and sticky capability flag, and DSH's never-print-100-for-a-partial-hit formatter.
3. **Cost — follow hermes, not pi.** Integer micro-USD (Epsilon already: `epsilon-contracts/src/message.rs:117`)
   plus a `cost_source` (`catalog|provider|unknown`) so a model missing from `epsilon-llm::catalog` renders
   `$?`, never `$0.000`.
4. **Freshness — add `updated_at` per figure and a stale marker.** No reference does this; pi's post-compaction
   `?`, hermes's `context_display_source` and DSH's model-switch skew note all imply it is needed.
5. **Surface — a `/status` overlay** (Epsilon has none) laid out by hermes's priority-ordered progressive
   disclosure (`appChrome.tsx:556-614`) plus the one-line footer.

## 2. Terminal UI event loop + Windows specifics

Epsilon today (`crates/epsilon-tui/src/app.rs:2602-2837`): `ratatui::try_init()`, `EnableBracketedPaste`, a
bounded 128-event input drain, a bounded 128-event agent coalesce, one `dirty` flag, `FRAME_MIN = 33 ms`
(`:66`), biased `select!` serving input first.

### pi 0.85.1 — init, resize, frame pacing, input
- Init (`terminal.ts:175-206`): raw mode → utf8 → resume → `\x1b[?2004h` → stdout `"resize"` listener →
  refresh dims → **`enableWindowsVTInput()`** → Kitty query with modifyOtherKeys fallback. Alt screen/mouse
  live elsewhere (`tui-alt-screen.ts:61-67`). Resize is that stdout event, no polling and no debounce (`:191`);
  `refreshTerminalDimensions()` returns immediately on `win32` and swallows `EACCES`/`EPERM` (`:49-56`) — what
  `regression-sigwinch-kill-eacces.test.ts` encodes.
- **Frame pacing: two paths, input preempts the throttle.** `MIN_RENDER_INTERVAL_MS = 16` (`tui.ts:477`);
  `requestRender()` coalesces via `process.nextTick` (`:952-961`); `scheduleRender()` waits out the interval
  (`:986-1004`); `requestImmediateRender()` (`:963-978`) cancels the timer so a keystroke never queues behind
  it. Comment `:1078-1080` names the reason: "Avoid the throttled timer path, where even `setTimeout(0)` can
  take a full 16 ms tick **on Windows**."
- **Synchronized update on every path**: `\x1b[?2026h`…`\x1b[?2026l` around line-diff writes
  (`tui-main-screen.ts:280/302, 403/435, 460/567`) via a `BoundedTerminalWriter` capped at 1 MiB (`:9,18-66`)
  so a huge frame is chunked without splitting surrogate pairs *and* without losing the end-of-sync marker.
  Diffing is **line strings, not cells** (`:362-388`, `\x1b[2K` rewrites `:490-546`); full redraw only on width
  change, height change (except Termux), `firstChanged < viewportTop`, or clear-on-shrink.
- Windows: N-API addon `native/win32/src/win32-console-mode.c` exposes `enableVirtualTerminalInput` — adds
  `ENABLE_VIRTUAL_TERMINAL_INPUT` (0x0200) to the *stdin* handle so Shift+Tab arrives as `\x1b[Z`;
  `terminal.ts:372-400` documents it **must run after `setRawMode(true)` because raw mode resets console mode
  flags**, else libuv discards modifiers and Shift+Tab degenerates to `\t`.

### deepseek-harness — no TUI; transferable frame *policy*
Ships `base|headless|sdk-app|sdk-minimal|acp-app|web-app`; the only terminal is an `@xterm/xterm` widget
*inside* the browser GUI. No alt screen, no `?2026h`, no SIGWINCH, no `setRawMode` (greps over every
`package.json` and `src`: zero); the one raw-mode mention is an extension point nothing uses
(`boot/app-boot/src/index.ts:611-618,633`). Its contribution is UI-neutral scheduling:
- **Cadence declared per event class, not per rate.** `publication?(match): 'none'|'immediate'|'animation-frame'`
  (`client/ui-conversation/src/client/contract/conversation.ts:217-222`); text deltas → `'animation-frame'`,
  `usage`/`finish` chunks → `'none'` (**no repaint at all**), else immediate
  (`client/ui-chat/.../conversation-nodes/assistant.ts:321-326`), aggregated by rank (`assembler.ts:54-58`).
  Streaming coalesces across **three** paint opportunities, single-flight (`assembly.ts:131-148`), and
  `immediate` bypasses with `cancelFrame(); flush()` (`:146-147`).
- **Three notifier verbs, the third for input latency**: `markDirty()` → microtask, `markFrameDirty()` → rAF,
  `notifyNow()` → synchronous, because "controlled-input writes must notify in the same tick as `onChange`"
  (`client/api/session-controller/src/client/sessions/notifier.ts:31-55`). Honest gap: `markFrameDirty` has
  **zero production callers**; three docs claim it is the streaming mechanism.

### hermes-agent — hermes-ink (vendored Ink fork); richest Windows quirk list
- `throttle(deferredRender, FRAME_INTERVAL_MS, {leading,trailing})`, `FRAME_INTERVAL_MS = 16`
  (`ink/constants.ts:2`), `deferredRender = () => queueMicrotask(onRender)` so `useLayoutEffect` cursor state
  lands in the same frame (`ink/ink.tsx:373-386`).
- **Backpressure-aware frame skipping — no other reference has this.** If the previous `stdout.write` has not
  drained, skip the frame and retry on a drain tick at `FRAME_INTERVAL_MS >> 2` (4 ms), capped by
  `MAX_COALESCED_BACKPRESSURE_FRAMES = 10` (`ink.tsx:778-806`) — the cap exists because a backed-up pipe once
  starved the stdin `readable` callback and wedged *input*.
- **Resize healing** (`ink.tsx:497-567`): dims captured synchronously, pending timers cancelled, alt-screen
  buffers reset, mouse tracking re-asserted, React commit deferred to one microtask per burst — "vscode fires
  many SIGWINCHes per panel drag, each ~80 ms uncoalesced = event loop visibly locks up". Two deliberate
  non-actions: never write `ENTER_ALT_SCREEN` on resize (iTerm2 treats `?1049h` as a clear) and never
  `ERASE_SCREEN` first (`render()` can take ~80 ms). Clear+paint is one patch list so an erase never precedes a
  paint (`:1072-1102`), one `writeDiffToTerminal` per frame with an empty-diff zero-write fast path.
- **Synchronized update computed once at module load, tmux and Zellij deliberately excluded** because they
  re-chunk the stream, so the markers "buy no atomicity and stale frames get pushed into main-screen
  scrollback" (`ink/terminal.ts:66-146`). **Transcript virtualization** replaces write-once
  (`useVirtualHistory.ts:13-52`): `MAX_MOUNTED = 120`, `OVERSCAN = 20`, `SLIDE_STEP = 12`, with measurements
  in-file (260 mounted items → ~23k Yoga nodes, p99 106 ms), scroll subscription binned so sub-bin wheel ticks
  skip the commit. **Frame telemetry is an API**: `onFrame({durationMs, phases:{…}, flickers})`
  (`ink.tsx:1275-1295`) behind `HERMES_TUI_FPS=1`. **Input escape-timeout trap, with its cause**: 50 ms normal /
  500 ms paste (`ink/components/App.tsx:184-185`); `flushIncomplete` **re-arms instead of flushing** when
  `stdin.readableLength > 0`, because "Node's event loop runs the timers phase before the poll phase" — a
  heavy render otherwise turns a buffered wheel event into a spurious Escape plus a lost scroll (`:511-538`).
- Windows: **legacy console cannot clear scrollback**, so `ESC[3J` only for Windows Terminal / VS Code+ConPTY /
  mintty, else erase + legacy HVP home `CSI 0 f` (`ink/clearTerminal.ts:8-9,29-46`); **conhost's
  `SetConsoleCursorPosition` follows the cursor into scrollback** (microsoft/terminal#14774) ⇒
  `hasCursorUpViewportYankBug()` true for `win32` *or* `WT_SESSION` (`ink/terminal.ts:339-347`); **OSC 9;4
  progress disabled in Windows Terminal**, read as a notification (`:34-38`). Windows footgun linting is a
  **gate** (`scripts/check-windows-footguns.py`) with dedicated CI lanes.

### opencode 1.18.31 — sharpest Windows console workaround in any reference
- `packages/tui/src/terminal-win32.ts` is a whole `bun:ffi` shim over `kernel32.dll` (`GetStdHandle`,
  `GetConsoleMode`, `SetConsoleMode`, `FlushConsoleInputBuffer`, `:7-13`): `win32DisableProcessedInput()`
  (`:30-42`) clears `ENABLE_PROCESSED_INPUT = 0x0001` on the **stdin** handle so Ctrl+C arrives as stdin bytes,
  not a console-wide `CTRL_C_EVENT`, and the comment names both traps — "the flag is console-global, not
  per-process" and "various runtimes can re-apply console modes (sometimes on a later tick)" (`:58-68`). It
  therefore **wraps `stdin.setRawMode`**, re-enforces twice via `setImmediate`, and runs a **100 ms polling
  interval** as backstop (`:91-95,99-107,112`); `win32FlushInputBuffer()` (`:47-54`) discards queued console
  input at startup *and* shutdown (`app.tsx:214,358`).
- **Streaming coalescing with a zero-latency fast path** (`context/sdk.tsx:48-80`): push the event, compute
  `elapsed = now - last`; if a timer is pending return; **if `elapsed < 16` arm `setTimeout(flush, 16)`,
  otherwise `flush()` immediately** — all deltas inside one 16 ms window collapse into one `batch()` → one
  store write → one repaint, and a stream slower than 16 ms pays no added latency. Resize has no SIGWINCH at
  all: components read reactive `useTerminalDimensions()` (`app.tsx:371`) and shrink is per-widget clamping —
  which is how a ratatui `Constraint` layout already behaves. **Synchronized update: not found** — no `?2026`,
  no "synchronized" in the tree; whether the engine brackets frames is internal to un-vendored `@opentui/core`.
  Do not assume it does. Windows: `terminal.suspend` disabled with `ctrl+z` rebound to undo
  (`config/index.tsx:101-111`); **ConPTY sends CR-only newlines in bracketed paste**, so pastes are normalized
  `\r\n|\r → \n` (`component/prompt/index.tsx:1402-1405`).

### Cross-reference: concrete recommendations for a Rust/crossterm/ratatui TUI on Windows

Verified against the crates Epsilon actually compiles (registry sources, exact pinned versions).

1. **Epsilon never gets Ctrl+C on Windows, and crossterm cannot be blamed.** `ratatui::try_init()`
   (`ratatui-0.30.2/src/init.rs:397-403`) calls `enable_raw_mode()`, whose Windows implementation clears
   exactly `NOT_RAW_MODE_MASK = ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT`
   (`crossterm-0.29.0/src/terminal/sys/windows.rs:18,36`) — so `ENABLE_PROCESSED_INPUT` **is already clear** and
   Ctrl+C should arrive as a `KeyEvent`. Yet `app.rs` has no Ctrl+C arm while `docs/qa/07-tui-keymap-audit.md:27`
   documents Ctrl+C as quit; resolve that by *reading the mode*, not by guessing, using opencode's shim as the
   model for asserting the flag.
2. **Wrap every frame in a synchronized update — neither ratatui nor its crossterm backend does.** The
   `Backend` trait has no sync hook (`ratatui-core-0.1.2/src/backend.rs:169-331`) and
   `ratatui-crossterm-0.1.2/src/lib.rs` contains no `synchronized`/`2026` string, so wrapping the existing
   single draw site (`app.rs:2751`) is the whole change:
   ```rust
   use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
   let sync = std::env::var_os("TMUX").is_none() && std::env::var_os("ZELLIJ").is_none();
   if sync { let _ = execute!(std::io::stdout(), BeginSynchronizedUpdate); }
   let _ = terminal.draw(|f| draw(f, &mut app));
   if sync { let _ = execute!(std::io::stdout(), EndSynchronizedUpdate); }
   ```
   crossterm emits `CSI ?2026h`/`?2026l` (`crossterm-0.29.0/src/terminal.rs:433-450,486-490`); conhost ignores
   unknown DECSETs, Windows Terminal honours them. Gate it like hermes (`ink/terminal.ts:66-146`); re-emit per
   frame, since it is not sticky.
3. **Do not add resize handling.** `ratatui` calls `autoresize()` inside every `draw`
   (`ratatui-core-0.1.2/src/terminal/resize.rs:64-73`), and `resize()` already converts a *horizontal shrink*
   into `clear_region(ClearType::All)` precisely "to avoid line wrapping issues" (`:41-45`). Epsilon has zero
   `TermEvent::Resize` arms and needs none — it only has to keep *drawing* on resize, and its 33 ms ceiling
   already provides hermes's microtask-per-burst coalescing.
4. **Split the frame budget by event class.** Epsilon's single `dirty` flag lets a keystroke wait up to
   `FRAME_MIN` = 33 ms (`app.rs:2750-2754`):
   ```rust
   #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
   enum Cadence { None, Coalesced, Immediate }   // usage/tick | stream delta | input, approval
   ```
   `Immediate` → draw now and reset `frame_due` (pi's `requestImmediateRender`, `tui.ts:963-978`);
   `Coalesced` → the existing dirty/due gate; `None` → update state, never set `dirty` (DSH's `'none'` for
   `usage`/`finish`). This also takes the spinner tick's unconditional `dirty = true` (`app.rs:2828-2833`) out
   of competition with typing.
5. **Adopt opencode's zero-latency batching rule, and hermes's backpressure skip.** Epsilon drains up to 128
   broadcast events then waits out `frame_due` (`app.rs:2725-2742`). opencode's rule (`context/sdk.tsx:48-80`)
   is strictly better: *batch if the last flush was recent, flush now otherwise* — draw immediately when
   `now - last_draw >= FRAME_MIN`, else coalesce, with a keystroke bypassing the gate (item 4). On conhost,
   where a frame costs tens of ms (Epsilon's own M94 measurements), add hermes's skip: when the previous write
   has not drained, skip the frame and retry on a 4 ms tick, **capped at 10 skips so input cannot wedge**
   (`ink.tsx:778-806`).
6. **Three cheap Win32 escape hatches Epsilon lacks.** crossterm 0.29 provides only the first; the other two
   need `windows-sys`, possibly already in-tree via the Job-Object confinement work (root `Cargo.toml:47`).
   **`DisableLineWrap`** ships in crossterm (`terminal.rs:157-174`) and on Windows clears
   `ENABLE_WRAP_AT_EOL_OUTPUT` directly (`:166-173`); it is **not** part of `enable_raw_mode` and Epsilon never
   issues it. **`ENABLE_VIRTUAL_TERMINAL_INPUT`** (pi's fix, `terminal.ts:372-400`) makes conhost deliver
   modifiers as VT sequences, which is what makes Shift+Tab reliable — Epsilon already records that conhost
   problem at `docs/milestones/M21-thinking-fidelity.md:61`. **`FlushConsoleInputBuffer`** at startup
   (opencode `terminal-win32.ts:47-54`) discards keystrokes typed before the TUI owned the console.
7. **Normalize CR on paste, avoid `CSI 3J`, and add frame telemetry.** opencode's `\r\n|\r → \n`
   (`prompt/index.tsx:1402-1405`) is required on ConPTY and is a small change in Epsilon's `TermEvent::Paste`
   arm (`app.rs:2859+`). Hermes found conhost follows the cursor into scrollback and suppresses cursor-up
   scrolling on `win32` *or* `WT_SESSION` (`ink/terminal.ts:339-347`), and that legacy console cannot clear
   scrollback at all — so never emit `ClearType::Purge`/`3J`. Finally, hermes's `onFrame` payload
   (`ink.tsx:1275-1295`) is the only reference that can *prove* a rendering change helped: an
   `EPSILON_TUI_FPS=1` printing `frame_ms phases{tokens wrap draw write}` would turn the M94 conhost claims
   into measurements.

## 3. Compaction/session architecture (brief)

Epsilon's `compact.rs` is already the most sophisticated port of the four (pi's `legal_cut_point`, opencode's
protected-tail clamp, DSH's tool-pairing and hermes's amend-mode delta summary are cited in its doc comments,
`crates/epsilon-core/src/compact.rs:66-124`). Three reference ideas worth naming:

- **Anti-thrash breaker (hermes).** Two consecutive *ineffective* compactions or two fallback summaries trip a
  cooldown, judged only where the provider's real prompt count is visible
  (`context_compressor.py:2486-2513`, `:2399-2431`). Epsilon's threshold is window-minus-reserve
  (`epsilon-core/src/lib.rs:1665+`); a *streak* of compactions that did not move the anchored token count is
  the cheap detector, and `compact.rs`'s `UsageAnchor` already supplies the before/after pair.
- **A summary must be smaller than the span it shadows, and history is never deleted (DSH).** Compaction
  appends `compaction/summary` plus a replacement `user/message` carrying
  `surfaceOp: {op:'replace', startSeq, endSeq}` (`compaction-basic/src/region.ts:492-510`), and **rejects a
  summary not smaller than that span** (`:419-423`) — a one-line invariant Epsilon does not obviously have.
- **Continuation chain with a watermark clone (hermes).** The child summary row is published atomically with
  the parent's close, and the tail that arrived *during* the slow summary is cloned into the child past a
  `watermark` (`hermes_state_compression.py:228-297`) — what makes a summary call safe while the user types.

## Ranked list of ideas to adopt (most valuable first)

1. **Synchronized-update frames (`CSI ?2026h/l`), tmux/Zellij-gated.** *pi* `tui-main-screen.ts:280/302,460/567`;
   *hermes* `ink/terminal.ts:66-146`; *crossterm* `terminal.rs:433-450`. Sketch: the wrapper in §2 item 2.
2. **Cadence by event class: input preempts the frame ceiling; `usage`/tick never mark dirty.** *pi*
   `tui.ts:963-978,1078-1080`; *DSH* `assistant.ts:321-326` + `notifier.ts:31-55`; *opencode*
   `context/sdk.tsx:48-80`. Sketch: the `Cadence` enum in §2 item 4, matched at the two drain sites.
3. **Anchored occupancy in the footer, with an "estimated" marker.** *DSH* `usage-projection.ts:78,208-216`;
   *pi* `agent-session.ts:3419`; *hermes* `context_breakdown.py:94-114`. Epsilon owns both halves; wiring only:
   ```rust
   // footer.rs context_parts(): swap latest_prompt_tokens for the anchored projection
   let used = anchored_tokens(history, anchor);          // provider truth + delta estimate
   let exact = anchor.is_some_and(|a| a.covered == history.len());
   let label = if exact { format!("{p:.1}%/{win}") } else { format!("~{p:.1}%/{win}") };
   ```
4. **Cache-miss waste with pi's noise floor, sticky capability flag, and DSH's honest percentage.** *pi*
   `cache-stats.ts:11,14-23,42-47,56-90`; *DSH* `token-format.ts:67-98`; *hermes* `cli_status_bar_mixin.py:368-375`.
   Epsilon has `cache_stats.rs` but surfaces it only inside `/session` (`app.rs:1605-1609`). Sketch: three
   accessors (`missed_tokens`, `missed_micro_usd`, `miss_count`) plus a
   `format_cache_hit_percent(cache_read, prompt, places)` that refuses `100` for a partial hit.
5. **A `/status` overlay with priority-ordered progressive disclosure.** *hermes* `appChrome.tsx:556-614`.
   Sketch: a `Command::Status` arm rendering a `Clear`+`Block` overlay from `FooterStats` + `CacheStats` +
   `agent.cache_report()`, dropping tail rows that exceed `area.height`.
6. **The three Win32 hatches at init (§2 item 6).** *crossterm* `terminal.rs:157-174`; *pi* `terminal.ts:372-400`;
   *opencode* `terminal-win32.ts:30-54`. Sketch: one `win32_console_init()` calling `DisableLineWrap`,
   `SetConsoleMode(ENABLE_VIRTUAL_TERMINAL_INPUT)`, and `FlushConsoleInputBuffer`.
7. **Backpressure-aware frame skipping with a 10-frame cap.** *hermes* `ink.tsx:778-806`. Sketch: `paint()`
   returns flush cleanliness; on pending keep `dirty = true`, set `frame_due = now + FRAME_MIN/8`, and force a
   draw when the skip counter reaches 10.
8. **Frame telemetry behind `EPSILON_TUI_FPS`.** *hermes* `ink.tsx:1275-1295`; *opencode* `runtime.ts:407-414`.
   Sketch: a `struct FrameStat { ms, wrapped, drawn }` in the draw path, ring-buffered, dumped on exit.
9. **Compaction anti-thrash breaker + "summary must shrink the span it shadows".** *hermes*
   `context_compressor.py:2486-2513`; *DSH* `region.ts:419-423`. Sketch: compare `anchored_tokens` before and
   after; two non-reducing attempts set a cooldown that skips auto-compaction.
10. **Cost provenance instead of silent omission.** *hermes* `usage_pricing.py:146-284`; *opencode*
    `models-dev.ts:160-206`. Sketch: `cost_source: CostSource { Catalog, Provider, Unknown }` beside
    `cost_micro_usd` in `FooterStats`, rendering `$?` when `epsilon_llm::catalog::lookup_model` misses.
