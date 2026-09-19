# Terminal-UI design spec — premium agent-harness chrome, for a ratatui 0.30 rebuild

Read-only study of `references/`. Two premises in the brief are wrong and are corrected first, because
they change what can be copied from where.

## 0. Premise corrections (verified)

- **opencode 1.18.31 has no Go TUI.** `Get-ChildItem -Recurse -Include *.go` → 0 files; no `go.mod`, no
  `lipgloss`/`bubbletea`/`tea.Tick`, no `theme.go`/`styles.go`/`view.go`. The TUI is TypeScript +
  **SolidJS** on `@opentui/core` `0.4.5` (`packages/tui/package.json:55`); OpenTUI's renderer core is
  Rust/Zig upstream — the only systems language in that stack.
- **deepseek-harness-dsh v0.1.6-alpha.2 has no terminal UI at all.** No ink/chalk/kleur/ora/blessed/
  boxen/cli-table in any manifest; zero `╭ ╮ ╰ ╯` repo-wide; `apps/cli/src` is a 5-file launcher whose
  only modes are profile/plugin/dump-config (`bin.ts:30-71`). The product UI is a React-DOM **web** GUI
  (`apps/web` + ~70 `packages/client/ui-*` plugins over a `--dsw-*` token sheet) — still §4's
  token-and-metric precedent.

Three real TUIs remain: **pi** (bespoke TS differential renderer, alt-screen), **opencode**
(SolidJS/OpenTUI), **hermes** (vendored Ink fork `@hermes/ink`). §1–§3 paths are relative to each
reference's own root.

---

## 1. pi 0.85.1 — fill and hairline; almost no boxes

**Layout.** `tui-renderer.ts:23-47`: `fullscreen` → `TuiAltScreen`, else `TuiMainScreen` (history in
terminal scrollback). Region order in both (`interactive-mode.ts:890-898`): document · pending · status ·
widgets-above · editor · widgets-below · footer. Fullscreen is a real constraint tree
(`chat-viewport.ts:23-45`): `VStack([transcript{basis:0,grow:1,shrink:1,minSize:1},
dock{basis:"auto",grow:0,shrink:1,minSize:1}])`, the dock itself `VStack([pending(1,0), status(1,0),
editor(1,3), footer(1,1)])`, the transcript a `ScrollView({follow:"end", primary:true,
overscroll:"chain", scrollbar:"auto"})`. **No flexbox, no percentage sizing in stacks** —
`StackLayoutEntry {basis|"auto", grow, shrink, minSize, maxSize, visible}` (`layout-node.ts:10-18`);
percentages are overlay-only (`tui.ts:197-206`). Height is *measured by rendering* the leaf at the child
width, memoised per frame (`layout.ts:68-84`); the allocator is integer-greedy in entry order
(`stack.ts:114-152`). Composer max height `max(5, floor(rows * 0.3))` (`editor.ts:523-525`); footer
right padding `minPadding = 2` (`footer.ts:180`); every blank line is `Spacer(1)` (`spacer.ts:9`). **No
sidebar, no split panel** — the session list and tree *replace* the composer
(`interactive-mode.ts:4550-4551`).

**Chrome is a `─` hairline, never a box.** `DynamicBorder.render` →
`[color("─".repeat(max(1,width)))]` (`dynamic-border.ts:22-24`). The composer is **two horizontal rules,
no sides, no prompt character** — `editor.ts:597`: `// Render the line (no side borders, just horizontal
lines above and below)`. Rules are `hiddenLineCount > 0 ? createScrollBorder("↑", hiddenLineCount,
width) : "─".repeat(width)` (`editor.ts:499-505`); overflow is written *into* the rule —
`` ` ${direction} ${hiddenLineCount} more ` `` centred with `─` fill (`:268-272`), degrading to
`` `─── ↑ N more ` `` (`:275`) then `...` (`:279`). No rounded corner, border-radius or shadow exists.

**Roles and tools.** User = full-width fill `new Box(outputPad, 1, s =>
theme.bg("userMessageBg", s))` (`user-message.ts:40`), `paddingY = 1`, no border. Assistant = no fill,
Markdown at `outputPad`, `Spacer(1)` above (`assistant-message.ts:102-117`). Reasoning = italic
`thinkingText`, or one dim italic `"Thinking..."` line when hidden (`:145-154`), click-toggled. Tool
call = a filled `Box(1,1)` whose **fill alone encodes status** (`tool-execution.ts:298-302`):
`isPartial ? toolPendingBg : result.isError ? toolErrorBg : toolSuccessBg`; compaction/branch/skill
share `customMessageBg` with a bold label `\x1b[1m[compaction]\x1b[22m`
(`compaction-summary-message.ts:16,36`). Collapse caps: read/write 10 lines, grep 15, find/ls 20, bash
5, fallback 10 (`tool-execution.ts:40`); toggle `ctrl+o` or left-click; elision verbatim is
`` `... (${remaining} more lines,` `` + `<dim>ctrl+o</dim> to expand` + `)` (`:167`). Durations exist
**only** for bash/powershell — `(ms/1000).toFixed(1)s`, `Elapsed` while running, `Took` when settled
(`renderers/bash.ts:32-34,118-143`). Diffs render `-{lineNum} `/`+{lineNum} `/` {lineNum} ` with
**word-level inverse** limited to the single-removed/single-added case (`diff.ts:115-141`).

**Palette** (`theme/dark.json`; a full `light.json` twin exists): accent `#8abeb7`, border `#5f87ff`,
borderMuted `#505050` (**the composer rule**), ok `#b5bd68`, error `#cc6666`, warn `#ffff00`, text
`#d4d4d4`, muted `#808080`, dim `#666666`; fills selectedBg `#3a3a4a`, userMessageBg `#343541`,
toolPendingBg `#282832`, toolSuccessBg `#283228`, toolErrorBg `#3c2828`, customMsgBg `#2d2838`.
Reasoning effort is a **rule-colour ramp**: off `#505050`, minimal `#6e6e6e`, low `#5f87af`, medium
`#81a2be`, high `#b294bb`, xhigh `#d183e8`, max `#ff5fff` (`dark.json:76-82` via `theme.ts:371-391`,
applied `interactive-mode.ts:4166-4172`; bash mode takes green). Tool titles bold in `toolTitle`, paths
accent, key hints `dim`. Resets are surgical — `\x1b[39m` / `\x1b[49m`, never `\x1b[0m`
(`theme.ts:326-332`).

**Footer = two dim rows** (`footer.ts:229-241`). Row 1: cwd with `~` substitution,
`` `${pwd} (${branch})` ``, `` `${pwd} • ${sessionName}` ``. Row 2: left = space-joined `↑in ↓out
RcacheRead WcacheWrite CH% $cost (sub) ctx%/window (auto)`, zero parts omitted (`:130-166`), context
percent escalating `>90` → error, `>70` → warning (`:154-160`); right = model id + `• thinking <level>`,
right-aligned with `minPadding = 2`, right side truncating first (`:180-220`), dim applied to the two
halves separately so the inner colour reset cannot clear it (`:222-230`).

**Motion.** Braille spinner `["⠋","⠙","⠹","⠸","⠼","⠴","⠦","⠧","⠇","⠏"]` at **80 ms**
(`packages/tui/src/components/loader.ts:11-12`), `` `${frame} ${message}` `` (`:95-96`), spinner accent
/ message muted; a one-frame list disables the timer. **The working indicator lives inside the
composer's top rule** (`custom-editor.ts:36-78`): `borderColor("── ") + status + borderColor(`
${"─".repeat(width - statusWidth - 4)}`)`, degrading to a bare spinner — the status region is then left
empty, so working state costs zero extra rows. One pacing constant, `MIN_RENDER_INTERVAL_MS = 16`
(~62.5 fps) with coalesced `requestRender`, and **keyboard input bypasses the throttle**
(`tui.ts:1078-1080`). Streaming rebuilds the open component and lets the renderer row-diff
(`interactive-mode.ts:3244-3275`); alt screen writes only changed rows inside synchronized output
`\x1b[?2026h` (`tui-alt-screen.ts:1703-1714`). Scrollbar thumb `█` active / `┃` idle over track `│`,
auto-hidden after 1000 ms, thumb `max(2, round(track²/content))` (`layout.ts:288-319`,
`scroll-view.ts:53-55`). Chrome truncation is ASCII `"..."`, not `…`.

---

## 2. opencode 1.18.31 — one heavy rail; colour carries everything

**Layout.** Content column `paddingBottom={1} paddingLeft={2} paddingRight={2} gap={1}`
(`routes/session/index.tsx:1177-1197`); messages in `<scrollbox stickyScroll stickyStart="bottom">` with
the scrollbar **hidden by default** (kv `scrollbar_visible=false`, `:265`) and a `height={1}` spacer at
the top of the content (`:1198`). Sidebar `width={42}` only above **120 columns** (`:270`), so
`contentWidth = width - (sidebarVisible ? 42 : 0) - 4` (`:278`). Composer `max_height = max(6,
height/3)` (`prompt/index.tsx:1345`). Dialogs 60/88/116 over a black-alpha-150 scrim
(`ui/dialog.tsx:22-48`).

**One border primitive, no rounded box.** `ui/border.ts:1-21` defines exactly `SplitBorder = { border:
["left"], customBorderChars: { ...EmptyBorder, vertical: "┃" } }`. `┃` (U+2503) is the only box-drawing
border character in the TUI — user messages, block tool results, the subagent footer, permission cards
and the composer all use `border={["left"]}` with it.

**Roles and tools.** User `index.tsx:1394-1418`: left rail coloured **by the agent** (`:1388`), body
`paddingLeft={2}`, fill `backgroundPanel` (`backgroundElement` on hover). Assistant `:1686-1705`:
`paddingLeft={3}`, no border, no fill. Reasoning `:1586-1649`: `paddingLeft={3}`, `▾/▸` header in
`theme.warning` at `thinkingOpacity = 0.6` (`theme/index.ts:292`), body `textMuted`. Tool result
`BlockTool :2005-2043`: left rail with `borderColor={theme.background}` — **an invisible rail** —
`paddingTop/Bottom={1} paddingLeft={2} marginTop={1} gap={1}`. Inline tool rows sit at `paddingLeft={3}`
with a 2-cell icon gutter (`INLINE_TOOL_ICON_WIDTH = 2`, `:1584`); pending text is `~ {pending}` at a
further `paddingLeft={3}` (net 6); **denied calls take `TextAttributes.STRIKETHROUGH`**
(`:1959,:1970,:1977`) — the only strikethrough in the codebase. Inter-row gaps are computed
(`util/layout.ts:8-18`). Icons are ASCII/geometric, never emoji: bash `$`, glob/grep `✱`, read `→`,
write/edit `←`, webfetch/apply_patch `%`, websearch `◈`, todowrite `⚙`, task `✓`/`│`, execute
`✗`/`✓`/`│`, generic `⚙`; the assistant metadata row leads with `▣` (`:1552`); subagent continuations
use `↳`; the subagent status ring is `○ ◍ ◔ ●` (`cli/cmd/run/footer.subagent.tsx:33-44`).

**Palette — 50 tokens, each with dark *and* light variants** (`theme/index.ts:36-91`; asset
`theme/assets/opencode.json`, 33 bundled themes). Default `opencode` dark: primary `#fab283`, secondary
`#5c9cf5`, accent `#9d7cd8`, error `#e06c75`, warning `#f5a742`, success `#7fd88f`, info `#56b6c2`, text
`#eeeeee`, textMuted `#808080`, background `#0a0a0a`, backgroundPanel `#141414`, backgroundElement
`#1e1e1e`, border `#484848`, borderActive `#606060`, borderSubtle `#3c3c3c`; light `#ffffff / #fafafa /
#f5f5f5 / #b8b8b8 / #a0a0a0 / #d4d4d4`, text `#1a1a1a`, primary `#3b7dd8`. Mode auto-detects from
terminal background luminance (`index.ts:353-358`, Rec.601). **Diff is the richest of the three**:
`diffAddedBg #20303b` / `diffRemovedBg #37222c` (light `#d5e5d5` / `#f7d8db`), `diffAddedLineNumberBg
#1b2b34`, `diffRemovedLineNumberBg #2d1f26`, `diffHighlightAdded #b8db87`, `diffHighlightRemoved
#e26a75`, `diffLineNumber #8f8f8f`. Fine type: "dim" is **alpha, not SGR faint** (`fadeColor`
multiplies `color.a`, `prompt/index.tsx:111-113`; fade-in ramps 0→1 over 160 ms at 16 ms ticks with
smoothstep, `util/signal.ts:19-51`); bold marks section headers, selected rows and the `QUEUED` chip;
background is the main emphasis device (hover swaps `backgroundPanel ↔ backgroundElement`, selection
uses `theme.primary`).

**Status/footer.** `routes/session/footer.tsx:52-90` exists but is imported by nothing in this revision;
the live surfaces are the sidebar footer (`• OpenCode <ver>`) and the home footer `Directory · MCP ·
flexGrow · Version`. The `--mini` statusline is the useful model (`cli/cmd/run/footer.view.tsx:818-913`):
one row on `theme.status`, a mode chip in `statusAccent` (`EXIT`/`SHELL`/`BUILD`), then
`` `${Locale.number(total)} (${pct}%)` `` · `` `${text} · ${money.format(cost)}` ``, then model, then
hints joined by `` `· ` `` (`:490`). Responsive widths `{compact:80, commandHint:66, model:120,
spacious:150}` (`footer.width.ts:3-8`).

**Motion.** Renderer `targetFps: 60` (`app.tsx:194-206`). Braille spinner at **`interval={80}`** with a
1-cell gap to the label (`component/spinner.tsx:10-19`); animations-off falls back to `⋯`. A
"Knight-Rider" scanner (`ui/spinner.ts`) runs at **`interval={40}`** with `["⬥","◆","⬩","⬪"]`, blocks
`■`/`⬝`, `width = 8`, trail `alpha = 0.65^(i-1)` with a 1.15× bloom on step 1 (`:199-231`).
**Streaming smoothness is 16 ms event batching**: `if (elapsed < 16) { timer = setTimeout(flush, 16);
return }` wrapping every store write in `batch()` (`context/sdk.tsx:60-80`). Streaming markdown keeps
`streaming={true}` so settled blocks never re-layout. Auto-follow is `setTimeout(() =>
scroll.scrollTo(scroll.scrollHeight), 50)` (`:423-428`); jump dead-zone ±10 rows (`:400-406`).

---

## 3. hermes 2026.9.14 — gutter glyphs, rails, and a budgeted status rule

Primary renderer is `ui-tui/` — React 19 on a **vendored Ink fork** (`"@hermes/ink":
"file:./packages/hermes-ink"`, `ui-tui/package.json:22`); `chalk` appears only inside the fork.

**Layout and width math.** `appLayout.tsx:549-606`: shell → column → row [rail | pane | rail] → prompt
zone → composer → overlays. Transcript `ScrollBox stickyScroll` over `Box paddingX={1}` (`:184-196`);
composer `paddingX={1}` (`:345`); the status rule is **exactly 1 row** and may sit top or bottom
(`:484-525`). `lib/inputMetrics.ts` centralises the arithmetic: `composerPromptWidth =
max(1, stringWidth(promptText)) + 1` (`:173-175`); `transcriptGutterWidth = role === 'user' ?
composerPromptWidth : 3` (`:177-179`); `transcriptBodyWidth = max(20, totalCols - gutter - (termux ? 2 :
4))` (`:181-192`); `stableComposerColumns = max(1, totalCols - promptWidth - 2 - (afterPrompt >= 24 ? 2 :
0))` (`:194-202`). Overlays: `FloatBox` double-border opaque (`appChrome.tsx:842-856`), `Dialog` round
`paddingX={2} paddingY={1}` (`overlay.tsx:81-89`), scrim = rows of spaces filled with `statusBg`.

**Roles and rails.** Fixed 2-or-3-column gutter glyph (`domain/roles.ts:4-9`, rendered
`messageLine.tsx:307-315`): assistant `┊` in `border`, user `❯` in `label` **and bold**, system `·` in
`muted`, tool `⚡` in `muted`, events `◈` dim with the gutter blanked (`:147-159`). A later user turn is
preceded by `───` in `border` (`appLayout.tsx:201-205`). The only bordered transcript element is the
tool result: `borderStyle="round" borderColor={muted} marginLeft={3} paddingX={1}`, `maxChars = max(24,
cols - 14)`, `'(empty tool result)'` fallback (`:124-143`). Assistant bodies carrying details get a
`└─ Response` label (`:285-294`). **Tree rails are what makes tool/thinking output look designed** —
`TreeNode` composes `` `${rails.map(on => on ? '│ ' : '  ').join('')}${branch === 'mid' ? '├─ ' : '└─
'}` `` (`thinking.tsx:54-55`) with `▾ `/`▸ ` chevrons on every collapsible header. Trail lines are
`` `${formatToolCall(name, context)} (${duration.toFixed(1)}s)${detail ? ` :: ${detail}` : ''} ${error ?
'✗' : '✓'}` `` (`lib/text.ts:182-193`), the duration split back into a dim `statusFg` span (`:251-255`).
Defaults `thinking: expanded, tools: expanded, activity: hidden` (`domain/details.ts:23-27`); `/details`
cycles hidden/collapsed/expanded; Shift/Ctrl-click expands everything (`thinking.tsx:996-1013`).

**Palette — seeds → mix ladder, not an enumerated palette** (`theme.ts:319-370`, `:613-639`). Dark seeds
(`:372-393`): accent `#FFBF00`, primary `#FFD700`, border `#CD7F32`, bg `#101014`, surface `#1a1a2e`,
text and prompt `#FFF8DC`, label `#DAA520`, ok `#4caf50`, error `#ef5350`, warn `#ffa726`, selection
`#3a3a55`, activeRow `#333355`, shellDollar `#4dabf7`. Secondary tones are derived, never authored:
`muted = desaturate(mix(accent,bg,.19),.16)`, `label = desaturate(mix(accent,bg,.13),.16)`,
`statusFg = grayOf(mix(text,bg,.24))`, `border = mix(accent,bg,.25)`, `surface = mix(bg,
desaturate(accent,.15), .09)`. Stated principle (`:272-278`): *"A palette is BUILT, not enumerated …
'dim' is definitionally a derivative of the theme's own base colors and can never be incoherent."*
Contrast is enforced in one place (`:474-543`): `DISPLAY_MIN_CONTRAST = 1.45`,
`SEMANTIC_MIN_CONTRAST = 2.2` (light 1.18 / 1.6), `liftForContrast` toward the readable pole, and a fill
whose polarity is wrong for the detected background falls back to the derived value
(`LIGHT_BG_MIN_LUMINANCE = .4`, `DARK_BG_MAX_LUMINANCE = .35`). Diff: `diffAdded rgb(220,255,220)` /
`diffRemoved rgb(255,220,220)` as line backgrounds, `diffAddedWord rgb(36,138,61)` / `diffRemovedWord
rgb(207,34,46)` as ink (`:305-317`).

**The status rule is the best of the three** (`appChrome.tsx:487-840`). One row split `left │ right` by
`statusRuleWidths` (`:280-299`: separator 3 when `width >= 24`, `minLeftWidth = max(8, minLeftContent)`,
cwd yields first). Left→right: `` `─ ` `` · battery · busy face / idle status / notice · ` │ ` + model ·
` │ ` + context · `◉ focus` · `[████░░░░░░] 42%` · duration · `✓ {idle}` · `cmp N` · `◎ {cache}%` ·
`◷ {latency}s` · `↑ {tps} t/s` · voice · `N sessions` · `N bg` · `⛓ N` · `↩ resumes when subagent
finishes` · `Δ N.N¢`; right side `' ─ '` + title/cwd. Context bar `` '█'.repeat(filled) +
'░'.repeat(w - filled) ``, `w = 10` (`:269-274`), with its own colour ladder (`>=95 critical, >80 bad,
>=50 warn, else good`, `:197-215`). **Progressive disclosure is width-keyed** (`:319-334`): `compactCtx
<72`, `bar ≥72`, `duration ≥76`, `compressions ≥80`, `voice ≥84`, `bg ≥88`, `subagents ≥92`, `cacheHit
≥96`, `latency ≥104`, `tps ≥110`, on top of a per-segment `fits(SEP + width)` tail budget so
status/model/context are never crushed (`:584-594`). Busy indicator is user-selectable: braille (floor
`max(100, interval)`), `|/-\` ascii at 100 ms, emoji at 600 ms, or kaomoji faces every 2500 ms.

**Typography, and four documented footguns worth obeying in Rust.** Placeholders use explicit colour,
**never SGR dim or inverse** (`textInput.tsx:55-58`: on transparent profiles both composite against a
black RGB the user never sees, "the hint rendered as a slab"); scrollbars never use SGR dim
(`overlayPrimitives.tsx:24-25`); never bold a full run of box-drawing dashes because it paints an opaque
black cell background (`branding.tsx:88-93`); `inverse` is rejected for selection rows because it swaps
against the terminal's unknowable defaults (`overlayPrimitives.tsx:102-104`) — selected rows use a
tinted chip plus a luminance-picked ink (`:88-99`).

**Motion.** Streaming caret `▍` blinking at **420 ms** (`thinking.tsx:213-227`). Constants
(`config/timing.ts:1-6`): `STREAM_BATCH_MS 16`, `STREAM_SCROLL_BATCH_MS 96`, `STREAM_TYPING_BATCH_MS 80`,
`TYPING_IDLE_MS 250`, `REASONING_PULSE_MS 700`; `RESIZE_COALESCE_MS = 32`. Streaming commits only
*settled* top-level blocks so "every block tokenizes exactly once. Only the in-flight tail re-parses per
delta (O(tail))" (`streamingMarkdown.tsx:1-29`). Wheel step is 1 row to keep the DECSTBM fast path
smooth (`limits.ts:23-26`). Every clock returns early while the status rule is occluded: *"every tick
below is a re-render nobody can see — in an Ink TUI that churn reads as the dialog tearing"*
(`appChrome.tsx:148-158`). Virtualisation: `ESTIMATE 4, OVERSCAN 20, MAX_MOUNTED 120, SLIDE_STEP 12`.

---

## 4. DSH's contribution: the token and metric layer (no TUI, still load-bearing)

`packages/client/ui-theme/src/styles/design-platform.css` is a two-tier token system whose *discipline*
is the most portable artifact here: 71 `--dsw-static-*` primitives → semantic `--dsw-alias-*` aliases,
light on `body{}` and dark on `body[data-ds-dark-theme]{}`, all `rgb()` triples so a truecolor Rust port
needs zero translation. `docs/web-styling.md:17-18`: *"Use `--dsw-alias-*` semantic tokens in feature
components. Do not copy static palette values or write literal colors there… Keep theme selectors out of
feature component CSS."* Canvas light `rgb(255,255,255)` / dark `rgb(21,21,23)`; dark layers
`rgb(35,35,36)` / `rgb(44,44,46)` / `rgb(53,54,56)`; hairline ladder `rgba(255,255,255,.06/.12/.16/.2)`
(light `rgba(0,0,0,.04/.1/.12/.16)`); text ramp `rgb(249,250,251)` → `rgb(207,211,214)` →
`rgb(173,178,184)` → `rgb(129,133,140)` → `rgb(67,69,74)`; brand `rgb(65,118,230)` (dark
`rgb(103,158,254)`, running accent `rgb(86,134,254)`); success `rgb(34,197,94)`; error
`rgb(236,19,19)`/`rgb(242,90,90)`; warn `rgb(245,158,11)`/`rgb(247,173,49)`.

Five metrics to steal verbatim:
- **One row grammar for the whole product** (`ui-tool/.../ToolRow.module.css:1-2`): `[16 leading] gap6
  [title 13/24] gap8 [2x2 dot] gap8 [summary FILL truncate]`; row height 24, glyph 14 inside the 16 box,
  and a **nesting step of exactly 22px** = 16 leading + 6 gap (`ReasoningRow.module.css:100-104`).
- **Hairlines are 0.5px, not 1px** (`docs/web-styling.md:25`); dashed and state-coloured keep 1px.
  Terminal analogue: a `text.dim` rule, never a `border`-coloured box.
- **Weights 400/500/600 only, 700 reserved for markdown h1–h3**, intermediate weights banned
  (`design-platform.css:1-3`: Figma's `font-weight: 510` must flatten to `500`).
- **Motion is two-tier**: *cadence* — stream batches publish at most once per 3 animation frames
  (`assembly.ts:131-148`), store flushes 1 frame, scroll coalescing 500 ms, clocks 1 s, and **the elapsed
  clock stays hidden for the first 15 s** (`ChatView.tsx:191`); *token* — 100/120/150/180/200/220/300 ms
  on `cubic-bezier(0.4,0,0.2,1)`. The one accent loop is a 2.6 s gradient sweep across a running row,
  and the running state dot uses **flat keyframe holds, no tweening** (`StateDot.module.css:68-73`).
- **Diff signs are chrome, not content**: signs drawn as CSS `content` (`'+ '`, `'- '`, `'  '`) while the
  text keeps the raw patch lines, no line-number gutter, `white-space: pre` with horizontal scroll (*"a
  diff is read by its indentation"*), and a literal `⋯` gap row instead of elision prose.

---

## 5. Composite recommendation — ratatui 0.30

Where the three disagree, choose one and do not blend — separators (pi hairline / opencode invisible rail
/ hermes glyph gutter + rails); user turn (fill / agent-coloured rail / bold `❯` gutter); tool result
(status fill / invisible rail / rounded box); borders (composer rules only / composer + rails / round
panels + double floats). The composite takes pi's chrome economy, hermes' glyph gutter and status-rule
discipline, and opencode's token table.

### 5.1 Layout tree

```
Layout::vertical([Length(1), Fill(1), Length(1), Length(1)])          // header|body|hint|status
  body = Layout::horizontal([Length(26), Length(2), Fill(1)])        // sidebar < 90 cols: drop
    sidebar = Layout::vertical([Length(1) /*label*/, Length(n_sessions),
                                Length(1) /*label*/, Fill(1) /*tools*/])
    main    = Layout::vertical([Fill(1) /*transcript*/, Length(composer_h)])
      composer_h = clamp(3, lines + 2, max(5, frame_h / 4))
```
Header, hint and status are exactly 1 row and **never boxed**; the sidebar is 26 cols with a 2-col gap,
dropped below 90 columns and reachable via `Ctrl-N`; the transcript carries
`.vertical_scrollbar(ScrollbarVisibility::Automatic)`; **at most one border between the terminal edge and
any content**, so in practice only the focused composer and an expanded tool card are boxed; geometry is
recomputed from `frame.area()` every draw and never cached.

### 5.2 Color and attribute tokens

| Token | Dark | Light | Role |
|---|---|---|---|
| `bg.base` | `Reset` | `Reset` | never paint the canvas |
| `bg.surface` | `#1a1a2e` | `#f5f5f7` | sidebar, header strip |
| `bg.overlay` | `#141414` | `#fafafa` | palette, dialogs |
| `bg.user` | `#2a2a35` | `#ececf1` | user turn fill only |
| `bg.tool.p/ok/err` | `#282832`/`#283228`/`#3c2828` | `#e8e8f0`/`#e8f0e8`/`#f0e8e8` | tool card fill |
| `text.primary` | `#e6e6e6` | `#1a1a1a` | body |
| `text.muted` | `#8a8a8a` | `#6b6b6b` | meta, labels, placeholders |
| `text.dim` | `#5c5c5c` | `#9a9a9a` | timestamps, rules, chrome |
| `accent` | `#fab283` | `#3b7dd8` | brand, focus, `❯`, spinner |
| `border` | `#3c3c3c` | `#d4d4d4` | hairline rules, unfocused frame |
| `border.focus` | `#fab283` | `#3b7dd8` | focused composer frame |
| `status.ok` | `#7fd88f` | `#3d9a57` | completed tool |
| `status.warn` | `#f5a742` | `#d68c27` | retry, ctx > 70 %, pending |
| `status.err` | `#e06c75` | `#d1383d` | failed tool, ctx > 90 % |
| `status.info` | `#56b6c2` | `#318795` | streaming, running |
| `diff.add` fg/bg | `#b8db87`/`#20303b` | `#4db380`/`#d5e5d5` | `+` lines |
| `diff.del` fg/bg | `#e26a75`/`#37222c` | `#f52a65`/`#f7d8db` | `-` lines |
| `diff.ctx` | `#828bb8` | `#7086b5` | context lines, line numbers |

Attributes, exhaustively: **bold** = focused pane title, selected row label, user gutter glyph, section
headers. **dim** = every timestamp, duration, token count, cwd, rule, placeholder, unfocused label.
**italic** = reasoning body only. **underline** = OSC-8 links only. **reverse** = the composer cursor
cell only, never a list row. **background fills** = user turn, tool card body, selection chip, overlay
scrim. Never blink. Honour `NO_COLOR` by collapsing every token to `Reset` plus bold/dim/reverse. Derive
`text.muted`/`text.dim`/`border` from the accent instead of authoring them (hermes §3), and choose the
brightness trio by *terminal* background luminance so one palette survives both polarities.

### 5.3 Glyph vocabulary — complete, do not extend

```
frames      ╭ ╮ ╰ ╯ (focused composer, expanded tool card)   │ ─ (everything else)
rails       "│  " / "   " then "├─ " / "└─ "     (3-cell tree unit)
roles       ❯ user (bold accent)  ┊ assistant (border)  · system (dim)  ◈ event (dim)
tools       ● running (accent)  ✓ ok (status.ok)  ✗ err (status.err)  ⊘ denied  ○ queued
disclosure  ▾ open   ▸ closed
marks       • bullets  ⚠ warn  ↳ continuation/child row  → ← file in/out  $ shell
spinner     ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ @80 ms   (static ⋯ ; ascii |/-\ with --ascii)
meter       █ filled / ░ empty, width 10
streaming   ▍ caret, blink 420 ms
separators  " │ " between status fields · " · " inside a field · "───" before a new user turn
ellipsis    … in prose, ASCII ... inside chrome that must not reflow
```
Forbidden: emoji as the only status signal, `▎` inside status fields, `⏺`, more than one glyph family
per row.

### 5.4 Rendering rules

**Transcript** (a pure function of state; virtualise with a per-message `Vec<Vec<Line>>` and append only
the live tail):
```
row = [gutter 3][body = area.w - 3 - 4]
user      gutter "❯ " bold accent · wrapped body, bg.user fill across the FULL row incl. the
          gutter, padding 1 above/below, one blank row after
assistant gutter "┊ " border      · wrapped body, no fill, one blank row after
reasoning inside a rail node: "▾ Thinking · 412 tok" (chevron accent, label bold while live else
          dim, count dim) then italic text.muted lines at rail depth
event     gutter blank            · "◈ model switched to glm-5 · 12:04" dim
error     blank gutter            · "✗ read_file · ENOENT" — glyph status.err, text muted
```
Exactly one blank row on a group change (new role, or first row after a tool group). **Never a rule per
message** — only `───` before a new user turn, or nothing at all.

**Tool card** — collapsed 1–3 rows, expanded inline, never a modal:
```
collapsed  ┊ ● read_file · src/ui/layout.rs                     ✓ 81ms
           ┊ ⠹ edit · src/ui/layout.rs                 running · 0.4s
expanded     ╭─ edit ────────────────────────────────── ✓ 812ms ─╮
             │ 412 -  let area = frame.size();                    │
             │ 412 +  let area = frame.area();                    │
             │ 413    let chunks = Layout::vertical([              │
             │ ⋮ ... (37 more lines, ctrl-o to expand)             │
             ╰─────────────────────────────────────────────────────╯
```
Preview 10 lines (pi's `FALLBACK_PREVIEW_LINES`), then the overflow row in `text.muted` with the key
hint in `text.dim`. Card fill escalates pending → ok → err through the background token only; the frame
stays `border`. Durations come from a monotonic `Instant`, rendered `81ms` under 1 s and `1.2s` above.
Denied calls take strikethrough. Diff body: line background `diff.add.bg` / `diff.del.bg`, line numbers
in `diff.ctx` over the add/del line-number backgrounds, changed words overpainted with `diff.add.fg` /
`diff.del.fg`, and a literal `⋯` row for elided hunks.

**Composer** — the composer is the one framed region, and the screen itself is never boxed:
```
╭─ ⠹ running · 12.4s ──────────────────────────────── ⏎ send · esc stop ─╮
│ ❯ make the sidebar collapsible at 90 cols                             │
╰───────────────────────────────────────────────────────────────────────╯
```
The **working spinner and elapsed time live inside the top rule** (pi) so working state costs zero extra
rows; when the rule is too narrow, degrade to `── ⠹ ────`. Inner padding 1 cell; `❯ ` accent + bold on
the first row only; continuation rows indented to the same column; soft wrap; height `clamp(3, lines + 2,
max(5, h/4))`. Placeholder `Ask anything…` in `text.muted` — explicit colour, never SGR dim. The frame
is `border.focus` while focused, `border` otherwise, and dims while streaming.

**Status and hint rows** — the hint row is one row of context-sensitive keys; the status row is one row
of fields separated by ` │ `, each label `text.muted` and value `text.primary` unless semantic. Field
order, shed from the right as width shrinks: `model` · `{used}/{window} tok` · `[████░░░░░░] 42%` ·
`↑in ↓out` · `$cost` · `R{cache}%` · `⏱{elapsed}` · `N tools`. Breakpoints: `<100` drop tools, `<90` drop
cost, `<80` drop cache, `<72` collapse the meter to `{used} tok`. Always keep the mode, the keys that
work *right now*, and the focus name; never list a key that does nothing in the current mode.

**Motion budget.** Redraw only on input, data or a tick — never a frame loop. Stream batches at **16 ms**
through one drain. Spinner **80 ms**; elapsed timer **1 s**, hidden for roughly the first 15 s of a turn;
streaming caret blink **420 ms**; scrollbar auto-hide **1 s**; resize reflow coalesced at **32 ms**.
Append one span to the open block and commit settled blocks (blank-line-delimited, outside a code fence)
to an immutable list so each tokenises exactly once. Keyboard input bypasses the frame throttle while
data redraws obey it. Follow-end only while `follow == end`; wheel step 1 row; nested scroll chaining
returns unused delta. Suspend every timer whose region an overlay occludes.

### 5.5 Target mock-up — 100 × 30 (trailing spaces elided)

```
 Epsilon ───────  glm-5 · 42.1k/128k tok · ↑38.2k ↓3.9k · $0.0142 · ⠹ running · 12m ⏎ send · ? help
 sessions                   ❯ pin the layout on resize and keep the composer docked
  ▎refactor-tui
   docs-sweep               ┊ Reading src/ui/layout.rs
   audit-3
                            ┊ ● read_file · src/ui/layout.rs                     ✓ 81ms
 tools                      ┊ ● grep · "ScrollView" (14 matches)                ✓ 1.2s
  read_file                 ┊ ⠹ edit · src/ui/layout.rs               running · 0.4s
  grep                      ╭─ edit ────────────────────────────────── ✓ 812ms ─╮
  edit                      │ 412 -  let area = frame.size();                  │
  cargo                     │ 412 +  let area = frame.area();                  │
                            │ 413    let chunks = Layout::vertical([            │
                            │ ⋮ ... (37 more lines, ctrl-o to expand)           │
                            ╰───────────────────────────────────────────────────╯

                            ┊ The dock is now measured with `Length(3)`, so the
                            ┊ transcript takes every remaining row                ▍

                            ▾ Thinking · 412 tok
                            │  Width math needs exactly one owner; the composer
                            └─ must not assume a fixed gutter.

                            ◈ compaction ran · 128k → 42k

                            ───
                            ❯ make the sidebar collapsible at 90 cols
                            ╭─ ⠹ running · 12.4s ───────────────────── ⏎ send · esc stop ─╮
                            │ ❯ make the sidebar collapsible at 90 cols                    │
                            ╰─────────────────────────────────────────────────────────────╯
 insert · enter send · esc stop                       focus: composer
 glm-5 │ 42.1k/128k tok │ [████░░░░░░] 33% │ ↑38.2k ↓3.9k │ $0.0142
```
The sidebar occupies columns 1–26, a 2-cell gap follows, and the transcript gutter starts at column 29 so
the body is 70 cells. Every main-column row above begins at the same column — that alignment is the whole
effect. Chrome is 1 header + 1 hint + 1 status + 3 composer-frame rows ≈ 20 % of the screen; hold that
ratio. The only rounded frames are the composer and the expanded tool card; the sidebar is flat, its
current row carrying `▎` and bold.

### 5.6 Floor test

- **80×24**: sidebar dropped, header truncates to `Epsilon · glm-5 · ⠹`; status keeps mode + keys + ctx %.
- **60×20**: composer 3 rows max, tool cards collapsed only, hint row halves to `enter send · esc stop`.
- **< 40×10**: render a centred `terminal too small — 40×10 minimum` and ignore every other widget.
- **`--ascii` / non-UTF-8 `LANG`**: `+ - |` frames, `>` for `❯` and the selection bar, `\|/-` spinner,
  `#-` meter, ASCII `...` everywhere.
