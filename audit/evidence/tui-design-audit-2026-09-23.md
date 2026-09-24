# TUI design & usability audit — 2026-09-23

Read-only audit of `crates/epsilon-tui` against the project's own standard
(`.agents/skills/ratatui-tui/`) and against the reference harnesses' TUIs (pi 0.85.1,
opencode 1.18.31). Commissioned after the owner reported: *"The TUI is extremely poor in terms
of design and usability compared to the references."*

## The headline: the transcript's decoration system is dead code

`ui::transcript::decorate` (`ui/transcript.rs:91`) and `row_style` (`:31`) build the two-column
gutter, the `▎` user marker, the per-role coloured gutter glyph and the filled background bands.
**Their only callers are tests in the same file** (`:232`, `:251`, `:265`).

The production path is `draw` → `shell::paint` (`ui/shell.rs:88`) → `draw_transcript`
(`app.rs:3216`) → `RenderCache::visible_rows` (`render_cache.rs:203`) → `logical_lines` /
`style_for` (`render_cache.rs:48`, `:95`), which has no access to the glyph or band decision and
emits plain per-kind foreground text. Grep confirms: `decorate(`/`row_style(` appear only in
`ui/transcript.rs`; `▎` appears in production only in `ui/overlay.rs:204` (list selection).

So the user never sees: a band behind a user message (`theme.rs:482` `surface_user`), a
pending/ok/err band behind a tool call (`:488`, `:493`, `:498`), a `▎` on user turns, or any
marker on a tool **call** (results get `✓`/`✗` at `render_cache.rs:77-86`; calls get nothing).

**This one defect accounts for most of "extremely poor design".** The work was done and never
wired up — and because the tests exercise `decorate` directly, the suite reports it as covered.

## The five commits to make first

### 1. Wire gutter and bands into the draw path (design) — highest impact

Carry the row's `LineKind` alongside each cached row (`RenderCache` already keeps
`reconciled_ids` parallel to its lines, so the kind is reachable), and in `draw_transcript`
run each visible row through `transcript::decorate(theme, kind, body, area.width)`.

Width trap: cached rows are wrapped to `area.width - 1` (`app.rs:3217`) and `decorate` pads to
`width`, so pass `area.width`. **If decoration moves inside the cache, the cache key must gain
the theme** — it is currently `(width, expand_outputs)` (`render_cache.rs:155`) and is correct
only because decoration sits outside (`render_cache.rs:90` comment, `transcript.rs:85-90`).

Also delete the doubled prefix in the same commit: `state.rs:242`/`:288` store
`format!("> {text}")` while `render_cache.rs:72-75` prepends `"❯ "`, and the repo's own test
pins the result — `assert_eq!(text_of(&rows[0]), "❯ > hello")` at `render_cache.rs:394`. Two
redundant markers waste 4 of the first 4 columns and put `>` where the `▎` belongs.

### 2. Cut one rule and give the other a job (design)

`Regions::split` (`ui/layout.rs:82-90`) allocates header 1 + transcript Fill + rule 1 + composer
N + rule 1 + footer 1 — four chrome rows of 24 at 80×24, and **both rules are drawn by the same
function in the same style** (`ui/shell.rs:91` and `:101`, both `composer::draw_rule` at
`ui/composer.rs:37` in `theme.border()`), so neither says what it divides. Epsilon's own
composer doc argues for one rule (`composer.rs:33-36`); pi's dock has no dividers
(`chat-viewport.ts:31-38`). Removing `transcript_rule` recovers one transcript row and a
duplicated concept; update the two assertions that pin the count deliberately
(`app/golden.rs:112`, `ui/shell.rs:204`).

### 3. Make the footer honest (usability)

`Hints::idle()` advertises `? help` (`ui/footer.rs:44`), but `?` is **not a keybinding**
(`keybindings.rs:27-48`) — it works only as literal text parsed into `Command::Help`
(`picker.rs:585`). The keys a real session needs most are never mentioned: PageUp/PageDown
(`keys.rs:1046-1053`), Up/Down history (`:1008-1044`), Ctrl+O external editor
(`keybindings.rs:47`), Ctrl+X expand (`:38`). Build the hints from the live binding table the way
pi does (`keybinding-hints.ts:34-48`), and replace the `need 60×12` string (`keys.rs:268`) with
the real floor — `MIN_WIDTH = 44`, `MIN_HEIGHT = 10` (`ui/layout.rs:36-38`), rendered as "44×10"
at `:118`. The current message is simply false.

### 4. Stop printing a fabricated context percentage (design)

`Metrics::context_used` is `latest_prompt_tokens` (`app.rs:2931`) — the last call's prompt size —
but it renders as `42%/200k`, which reads as window occupancy. Turn 1 on a 200k model with a 14k
prompt shows `7%/200k` while the session context is 14k/200k. The doc comments on
`context_gauge` (`ui/metrics.rs:136`) and the module header (`:10-13`, "How much room is left?")
claim the window reading. **Decision needed:** label it honestly (`prompt 42%/200k`) or feed the
session context estimate. Also `context_gauge`'s two `None` arms are identical (`:146-147`)
while the comment claims `compacted` changes the output.

### 5. Reshape the approval prompt (usability)

`draw_dialog` (`app.rs:3571`) is 11 rows for a yes/no where the skill asks 5–7
(`references/layout.md:114-123`). Its own doc comment (`app.rs:3561-3567`) claims danger styling
that the code does not apply — it passes `focused: true`, which yields `border_focus()`
(`ui/overlay.rs:112-116`). Fix both: ~6 rows and let `Panel` take a border style.

## Then, larger items (not in the first five)

- **No tool cards; the diff is uncoloured prose** (`state.rs:305`, `:353`): a call is one flat
  line, an edit's diff is `format!("{name} → diff:\n{diff}")` pushed through the *markdown*
  parser, expansion is one **global** toggle (`keys.rs:1130`). pi colours per side with line
  numbers and inverse-videos changed words (`components/diff.ts:26-66`, `:124-141`); opencode
  shows the diff **inside the approval prompt** (`permission.tsx:38-78`).
- **No light theme** (`theme.rs:201-203`, `:221`, `:293`): one dark palette and `Color::White`
  as body text, which the skill forbids (`references/visual-system.md:32`). pi ships dark+light
  with an auto probe (`theme.ts:407-411`, `:663`); opencode's tokens are `{dark, light}` pairs.
- **No frozen-view signal and no jump-to-live** (`keys.rs:1046`, `:1168`): scrolling up during a
  run is silent, `G` does nothing, and there is no scroll position indicator anywhere outside the
  `/timeline` overlay (`app.rs:3492`). pi has a transcript scrollbar (`chat-viewport.ts:23-30`).
- **`NO_COLOR` collapses roles** (`theme.rs:300-307`): every `Emphasis::Plain` role becomes
  `Style::default()`, and `LineKind::ToolCall` and `LineKind::System` share `glyphs.bullet`
  (`transcript.rs:58-63` vs `:76-81`), so pending and notice are the same pixels. The glyph
  table has no `has_surfaces()` branch even though the docs promise one.
- **No OSC 52** (`app.rs:2730`): `/copy` uses arboard only and prints success even over SSH,
  where nothing was copied. The skill calls this out by name.

## What is genuinely good (an audit that only flatters the complaint is useless)

- **Event loop**: draws only when dirty, at most once per `FRAME_MIN`, bounded input drain
  (`MAX_INPUT_DRAIN`), coalesced agent events, 1-hour park when idle (`app/event.rs:158-170`). An
  idle TUI repaints zero times.
- **Render cache**: id+width keyed, prefix-sum offsets, binary-searched visible window, O(visible)
  per frame (`render_cache.rs:203-232`).
- **Approval key routing**: identity-based `approval_painted_id`, checked ahead of every other
  branch so `n` cannot fall through to new-session (`keys.rs:255`, `:266`), with tests.
- **Theme discipline**: every `Color::*` in the crate lives in `ui/theme.rs`; `has_surfaces()`
  withholds filled bands on non-truecolor rather than approximating them (`theme.rs:515`).
- **Composer**: bracketed paste enabled/restored, paste becomes an atomic `[paste #N +L lines]`
  marker (pi's own convention), grapheme-aware cursor motion, kill-ring, undo/redo, external
  editor hand-off.

## Checklist answers (the skill's review items)

| # | Item | Verdict |
|---|---|---|
| 1 | panic restore + color-eyre order | restore pass (`app.rs:2614`, `:2627`); **`color-eyre` absent from every manifest** |
| 2 | crossterm once | pass |
| 3 | no unconditional draw loop | pass, better than required |
| 4 | focus/mode matrix vs key handler | pass |
| 5 | resize and minimum size | pass |
| 6 | theme tokens vs raw colours | pass, cleanly |
| 7 | streaming does not clone history | pass |
| 8 | approval not skippable / no key collision | pass, notably careful |

## Layout by width (from the constraint code, not a drawn buffer)

No sidebar and no horizontal split exist (`ui/layout.rs:48-99`), so the three-pane collapse the
skill warns about cannot occur. `composer_cap(h) = min(8, max(1, h/3))`.

| width×height | composer | transcript | header |
|---|---|---|---|
| 160×48 | ≤ 8 | ≤ 37 | all fragments fit |
| 120×30 | 8 | ≤ 19 | fits |
| 80×24 | 8 | 18 (11 with a full draft) | right half cut from its near end; model survives (`header.rs:208-230`) |
| 60×20 | 6 | ≤ 11 | context and tokens dropped; no breakpoint between 60 and 44 |
| floor | — | — | `terminal too small — widen to 44×10` (`layout.rs:118`) |

4 chrome rows of 24 (17%) at 80×24, two of them the redundant rules from commit 2.

**Not verified by this audit**: the exact drawn frame at 120/160 for a live session (the numbers
are constraint arithmetic), and real clipboard/OSC 52 behaviour.
