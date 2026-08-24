# PROMPT — M16: TUI parity with pi/opencode (efficiency-first overhaul)

Feed this entire document to the agent in a fresh session. Work inside
`C:\Users\Hp\Desktop\My Agent Space\Epsilon`. Read AGENTS.md first; it governs.

---

## Mission

Epsilon's TUI works but feels clunky next to pi and opencode. Your job this session is a
**parity-and-efficiency pass**: port the interaction patterns those two references use,
and fix the rendering pipeline so the TUI gets *faster* with longer sessions instead of
slower. The user will judge by feel: keystrokes must do the obvious thing, nothing may
stutter, and the screen should always answer "what is the agent doing right now?"

You are NOT redesigning architecture. The 11-crate layout, event bus, approval channel,
and CLI composition are settled law. You ARE allowed to restructure anything inside
`crates/epsilon-tui`.

## Start here (in order)

1. Read `AGENTS.md`, `README.md` status table, and `docs/milestones/M15-tui-overhaul.md`
   (last TUI pass — what exists today).
2. Skim current TUI code: `crates/epsilon-tui/src/{app,state,picker,markdown,scroll,
   editor_ops,theme}.rs` (~2k lines total).
3. Study the reference implementations BEFORE writing code. File-anchored starting
   points (verified 2026-08-23):

### pi (`references/pi-0.84.2/packages/tui/src/`)
- `components/select-list.ts:85-102` — centered scroll window + scroll indicators.
  Already ported into `src/scroll.rs`; reuse it everywhere lists appear.
- `components/editor.ts` — multi-line editor: word segmentation, kill-ring
  (`../kill-ring.ts`), paste markers `[paste #1 +N lines]` treated as atomic segments
  (lines 21-45), word navigation (`../word-navigation.ts`). This is the editor bar.
- `components/footer.ts:24-130` — footer = pwd (~-shortened, git branch appended),
  token totals formatted compactly, **context usage %** computed against the active
  model's context window (handles compaction: unknown until next response → show `?`),
  cache-hit-rate. Port this computation; our catalog already carries context_window.
- `components/markdown.ts`, `loader.ts`, `keybinding-hints.ts`, `theme-controller.ts`
  (+ `dark.json`/`light.json`) — patterns we partially ported in M15; go deeper where
  noted below.

### opencode (`references/opencode-1.18.21/packages/tui/src/`)
- `component/dialog-provider.tsx` + `dialog-model.tsx` — TWO-STAGE model picker:
  provider list first, then that provider's models, with Favorites/Recents sections,
  fuzzysort filtering, "Free" badges. Our single-list picker works; reshape toward
  provider-first when the list would exceed one viewport.
- `component/prompt/index.tsx` — keybinding registry (`prompt.submit`, `prompt.clear`,
  `prompt.history.previous/next`, `prompt.paste`, `prompt.stash/pop/list`,
  `prompt.skills`, `prompt.palette`) with a command palette gathering them (line 568).
  Note the stash concept: park the current draft when opening a dialog, restore after.
- Message rendering is PART-BASED: each message renders per-part components rather
  than one text blob.

## Known inefficiencies you MUST fix (the "more efficient" half)

1. **O(n) markdown re-parse every frame**: `transcript_rows()` calls
   `markdown::md_to_lines()` for EVERY assistant row on EVERY draw (120ms tick). Fix:
   parse once per message at commit time (store rendered `Vec<Line>` or an id-keyed
   cache), never in the draw path. Draw must be span-assembly only.
2. **Full transcript rebuild each frame**: `transcript_rows()` clones every line's
   strings. Add dirty-tracking (UiState revision counter bumped on mutation; skip
   rebuilding unchanged prefix) or virtualize: compute total row count, render ONLY the
   visible window using `scroll::transcript_skip` math you already have.
3. **Picker rebuilds rows on every Up/Down** (`build_rows` re-filters). Filter once per
   input change, cache, navigate the cached vec.
4. Measure honestly: add a `--tui-bench <n>` hidden mode that builds n synthetic
   messages and times 100 draws before/after. Report numbers in the dev log.

## Deliverables (three slices, one commit + dev log each)

### M16a — Rendering efficiency + footer parity
- Id-keyed render cache (or equivalent) killing per-frame md parsing & string cloning.
- Transcript virtualization: only visible rows materialized.
- Footer/status parity with pi: cwd (~-shortened) + git branch (read `.git/HEAD`
  directly — NO gitoxide/git dependency), compact token format (pi `formatTokens`:
  k/M suffixes), context % = latest prompt tokens ÷ active model's `context_window`
  (show `?` right after compaction), session cost you already have.
- `--tui-bench` mode + before/after timings in dev log.
- Gates green (fmt/clippy-zero/all tests/`--demo`/`check --features real`).

### M16b — Editor v2 (pi editor.ts behaviors on our single-line-first base)
- Multi-line buffer: Alt+Enter or Ctrl+J inserts newline; editor area grows to fit up
  to N lines then scrolls internally; Enter submits, Esc behaves as today.
- Paste markers: bracketed pastes >1 line become atomic `[paste #N +L lines]` markers
  in the buffer (expand on submit) — see pi editor.ts:21-45.
- Kill-ring: Ctrl+U / Ctrl+W push kills; Ctrl+Y cycles yank. Ring depth ≥ 10.
- Word-nav stays unicode-aware (keep `editor_ops.rs` pure fns + tests; extend them).
- History: persists across restarts (append to global config dir, e.g.
  `~/.config/epsilon/history` capped 500) in addition to in-session recall.

### M16c — Picker/palette parity (opencode patterns)
- Two-stage model picker: stage 1 providers (with 🔑 auth column + recents group),
  stage 2 models of chosen provider (bundled catalog first, Ctrl+F live fetch merges).
- Command palette (Ctrl+P): fuzzy-filtered list of ALL commands (slash commands +
  actions like abort/clear/copy-last) — gather pattern from opencode prompt/index.tsx:568.
- Draft stash: opening any dialog saves the in-progress input; closing restores it
  (opencode prompt.stash).
- Slash autocomplete (M15) upgrades to use the palette's fuzzy matcher.

## Constraints (non-negotiable)

- Dependency law: exact pins in workspace Cargo.toml with stated reason. `pulldown-
  cmark`, `ratatui`, `crossterm` are in; do NOT add gitoxide/arboard/any auth SDK.
  Fuzzy matching: implement trivial subsequence match yourself (≤40 lines) instead of
  adding fuzzy-matcher.
- No secrets on disk ever (ADR-012). Auth stays env-var detection.
- All tests offline; faux providers only. New UI logic goes in pure modules with unit
  tests (see `editor_ops.rs`/`picker.rs`/`scroll.rs` for the house style). Key-path
  regression tests drive `handle_key` with synthetic `KeyEvent`s (see `app::tests`).
- Windows is the dev box: NEVER bulk-edit files with PowerShell string replace
  (backtick-n corruption bit us five times); use structured edit tools only.
- `std::env::set_var` is unsafe in edition 2024 and epsilon-core forbids unsafe —
  config functions take explicit dirs (`*_in` variants exist for this reason).

## Known traps from previous sessions (do not rediscover these)

- crossterm 0.29: bracketed-paste commands live in `crossterm::event::*`, NOT
  `terminal::*`.
- async-trait methods return futures: `.stream(..)` needs `.await` at call sites.
- genai types are `#[non_exhaustive]`: default-construct then mutate fields.
- toml 1.x: `Entry::or_insert_with` takes a closure.
- Guarded match arms must sit ABOVE their unguarded twins or they're unreachable.
- After editing key handlers, RUN THE KEY-PATH TESTS (`app::tests`) — M14 shipped a
  broken handler because nobody did.

## Discipline

- Three slices → three commits: `feat(m16a): …`, `feat(m16b): …`, `feat(m16c): …`.
- Dev log per slice in `docs/milestones/M16{a,b,c}-*.md`: what / decisions /
  test-caught bugs / bench numbers (M16a) / verification output.
- Update README quick-start keys table + CHANGELOG under [Unreleased] as you land.
- Every commit: full gates green first (fmt, clippy -D warnings level cleanliness,
  workspace tests, `--demo` completes, real-feature check compiles).
- Then `cargo install --path crates/epsilon-cli --features real --force` so the user's
  global `epsilon` picks it up, and say so explicitly at the end.

## Acceptance (user-judged)

- Scrolling a 200-message transcript: no perceptible lag; bench numbers prove it.
- Footer answers: where am I, what branch, which model, tokens, context %, cost.
- Editor: multi-line drafts survive opening/closing dialogs; pasted blobs become
  markers; Ctrl+W/Y feel native.
- Model switching is two obvious keystroke chains away, always visible where you are.
- Zero regressions in the 146+ existing tests.

Start by reading the three codebases' listed files, then write `docs/milestones/
M16a-plan.md` (≤60 lines: findings + exact edit list) BEFORE touching code. Then
implement M16a.
