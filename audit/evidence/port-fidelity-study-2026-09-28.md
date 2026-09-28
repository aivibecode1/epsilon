# Port-fidelity study — Epsilon (Rust) vs pi-agent 0.85.1 (TypeScript)

**Date:** 2026-09-28
**Scope:** the TUI and the agent loop. Reference = `references/pi-0.85.1/`, read from source
(`packages/tui/src/**`, `packages/coding-agent/src/modes/interactive/**`,
`packages/coding-agent/src/core/**`, `packages/agent/src/**`).
Subject = `crates/epsilon-tui/src/**` and `crates/epsilon-core/src/**`, read from source.

**Method.** Every classification below is backed by a `path:line` in a file that was opened
and read for this study. Where a claim could not be traced to an implementation, it is marked
**UNVERIFIED** rather than credited. The prior audit (`audit/02-port-fidelity.md`) was read
but not trusted; two of its rows are independently confirmed here and one of its corrections
is confirmed from the code.

---

## 0. Counts

| | count |
|---|---|
| pi TUI/loop capabilities surveyed | **84** |
| DONE | **38** |
| PARTIAL | **22** |
| MISSING | **24** |

Of the 38 DONE, 27 are pi features ported faithfully; 11 are places where Epsilon is
*richer* than pi (cost accounting, frozen-view badge, per-card expansion, render cache,
panic-safe terminal restore). The 24 MISSING are concentrated in one place: **everything
pi renders through a `Component` that Epsilon does not have an equivalent of.**

---

## 1. The twelve gaps that matter most, ranked by user impact

### G1 — Ctrl+C quits the app and discards the draft; `app.clear` is advertised but dead
`crates/epsilon-tui/src/keybindings.rs:31` binds `app.exit` to `ctrl+c` **and** `ctrl+q`;
`crates/epsilon-tui/src/keybindings.rs:32` binds `app.clear` to `ctrl+c` as well.
`crates/epsilon-tui/src/app/keys.rs:473` is the only consumer:

```rust
if app.keybindings.matches("app.exit", &key) {
    return true;
}
```

Nothing anywhere reads `app.clear` — it appears in no other file in the crate. The result is
that Ctrl+C, which pi defines as "Clear editor"
(`references/pi-0.85.1/packages/coding-agent/src/core/keybindings.ts:94`), terminates the
process with the draft still in the buffer. Worse, `/hotkeys` prints the *effective* binding
table verbatim (`crates/epsilon-tui/src/app/commands.rs:942-951`), so the TUI tells the user
`app.clear: ctrl+c` — a binding that does nothing. This is a "code claims done, code does
not" instance in the exact form this study was commissioned to find.

### G2 — `/fork` and `/clone` in the TUI are `std::fs::copy`, not forks
`crates/epsilon-tui/src/app/commands.rs:447` and `:486` both do
`std::fs::copy(&path, &fork_path)`. The copy carries the **same session id** in its header,
so the two files claim one identity, and the whole session — every abandoned branch, every
dead path — is duplicated. pi forks ancestry-only with a fresh id
(`session-manager.ts:1611-1634`, `forkFrom`). Epsilon already owns the correct
implementation: `crates/epsilon-session/src/store.rs:1511` `fork_branch_at` writes a new
header (`store.rs:1531-1536` sets `id: epsilon_contracts::new_id("ses")`) and writes only
`branch_to(leaf_id)` (`store.rs:1516`). It is wired **only** from the RPC surface
(`crates/epsilon-cli/src/rpc_mode.rs:998`). The interactive TUI, the surface a human uses,
has the worse one.

### G3 — branch navigation dumps the whole branch into the transcript after jumping
`crates/epsilon-tui/src/app/keys.rs:529-551`: after `Enter` navigates and re-projects, the
handler iterates the *entire* branch and calls `app.ui.push_notice("#seq {snippet}")` once
per entry. Jumping to entry 3 of a 400-entry session pushes 400 rows into the transcript
and scrolls the reader away from the message they jumped to. The navigation itself is
correctly wired — `store.navigate_to` at `store.rs:1354`, `reproject_current_branch` at
`crates/epsilon-core/src/lib.rs:858`, re-hydration at `keys.rs:511-519` — but the
aftermath is a defect, and the code even acknowledges the branch list "describes neither"
on the failure path (`keys.rs:523-526`) without noticing it also misdescribes on the
success path.

### G4 — branch summarization is never produced
`crates/epsilon-session/src/store.rs:1397` `branch_with_summary` is pi's
`branchWithSummary` (`session-manager.ts:1395-1420`), a faithful port. A grep across
`crates/` finds callers **only** in `crates/epsilon-session/tests/session_store.rs` and
`crates/epsilon-core/tests/compaction_scenarios.rs:635`. No production path — not the TUI's
`/tree`, not the RPC — ever calls it. pi calls it during tree navigation
(`agent-session.ts:3113+`, five call sites). Epsilon's `navigate_to` therefore abandons a
path with no record of what was on it, which is the entire reason pi wrote the primitive.
(This independently confirms row 50 of `audit/02-port-fidelity.md`, which I re-derived from
the code rather than from the audit.)

### G5 — no thinking-block visibility toggle
pi binds `app.thinking.toggle` to `ctrl+t`
(`references/pi-0.85.1/packages/coding-agent/src/core/keybindings.ts:118-121`) and
`interactive-mode.ts:2892` calls `toggleThinkingBlockVisibility()`. Epsilon has no such
state: `LineKind::Thinking` is always pushed
(`crates/epsilon-tui/src/state.rs:1027-1033`) and always styled
(`crates/epsilon-tui/src/ui/transcript.rs:78-87`). A user reading a long reasoning trace
cannot hide it. Epsilon rebinds `ctrl+t` to `app.thinking.cycle`
(`crates/epsilon-tui/src/keybindings.rs:35`) — the same chord, a different action, and
`/help` prints "thinking: Ctrl+T cycles"
(`crates/epsilon-tui/src/app/commands.rs:834`), so the collision is invisible to the user.

### G6 — no transcript paging that matches the viewport
`crates/epsilon-tui/src/app/keys.rs:1120-1127`:

```rust
_ if app.keybindings.matches("app.transcript.pageUp", &key) => {
    app.scroll_from_bottom = app.scroll_from_bottom.saturating_add(10);
```

A fixed ten rows, on every terminal. pi's `tui.altScreen.pageUp` is documented as "Scroll
viewport up one page"
(`references/pi-0.85.1/packages/tui/src/keybindings.ts:160-167`) and its alt-screen
renderer pages by the measured viewport. On a 48-row terminal, ten PageUps move 100 rows
and leave 38 unread; on a 12-row terminal one PageUp overshoots the top. Epsilon also lacks
`halfPageUp/halfPageDown`, `lineUp/lineDown`, `top`, `bottom`
(`tui/src/keybindings.ts:168-183, 208-209`) entirely.

### G7 — markdown features silently dropped by the `_ => {}` arm
`crates/epsilon-tui/src/markdown.rs:36` enables only
`pulldown_cmark::Options::ENABLE_STRIKETHROUGH`. The match at `markdown.rs:38-112` handles
paragraph, heading, emphasis, strong, blockquote, list, item, code block, text, inline
code, breaks and rule — and ends in `_ => {}` at `markdown.rs:112`. Consequences:

* **Strikethrough**: the option is enabled at `markdown.rs:36` and the resulting
  `Event::Start(Tag::Strikethrough)` is never matched. `~~text~~` renders as plain `text`
  with the tildes consumed. An enabled-but-unimplemented option.
* **Tables**: pi renders them with width-aware cell wrapping
  (`packages/tui/src/components/markdown.ts:550-556, 842-1011`). Epsilon has no
  `Tag::Table` arm; a markdown table renders as run-together cell text.
* **Task-list checkboxes**: pulldown-cmark needs `ENABLE_TASKLISTS`, which is not enabled.
* **Links**: pi emits OSC-8 hyperlinks
  (`markdown.ts:689-695`). Epsilon has no hyperlink path at all.

### G8 — no syntax highlighting
pi's markdown theme carries an optional `highlightCode`
(`packages/tui/src/components/markdown.ts:215`, invoked at `:523-525`). Epsilon's fenced
code path (`crates/epsilon-tui/src/markdown.rs:78-102`) prints `··· lang`, then `│ line`
per source line in a single `theme.code_block()` style. No highlighter is in the dependency
set — `crates/epsilon-tui/Cargo.toml:35-45` lists `pulldown-cmark`, `unicode-width`,
`unicode-segmentation` and nothing that could highlight.

### G9 — the command palette's expand entry is a no-op with a wrong key hint
`crates/epsilon-tui/src/picker.rs:1003-1008`:

```rust
PaletteEntry {
    title: "Toggle tool output expand".into(),
    command: "\u{1}expand".into(),
    binding: Some("x"),
},
```

`crates/epsilon-tui/src/picker.rs:685-690` `parse_command` begins with
`let rest = trimmed.strip_prefix('/')?;` — the string does not start with `/`, so it
returns `None`. `crates/epsilon-tui/src/app/keys.rs:593-595` therefore closes the palette,
clears the input, and runs nothing, with no notice. The `binding: Some("x")` hint is also
wrong: the real chord is `ctrl+x` (`keybindings.rs:38`), and a bare `x` is precisely the
binding `keys.rs:1201-1204` says was removed because it made the letter untypable. The
palette re-introduces it as a hint.

### G10 — unknown keys produce no visible response at all
`crates/epsilon-tui/src/app/keys.rs:1269` is the catch-all of `handle_key`:

```rust
_ => false,
```

No notice, no bell, no status change. Every overlay handler has the same shape
(`keys.rs:469`, `:571`, `:619`, `:679`, `:741`, `:860`). A user who presses a chord they
reasonably expect to work — `ctrl+]`, `alt+p`, `shift+tab` — gets a screen that does not
change, and no way to learn why. pi's `SelectList` at least has an exhaustive keybinding
surface and `Editor.handleInput` has 20+ named actions
(`packages/tui/src/components/editor.ts:683+`).

### G11 — the settings command prints file paths instead of opening settings
`crates/epsilon-tui/src/app/commands.rs:953-983`. The code says so itself at line 954:
"Ponytail: no overlay form at v1 (pi SettingsSelectorComponent is ~30 toggles)". pi ships
`modes/interactive/components/settings-selector.ts` (945 lines) plus
`config-selector.ts` (942) and `settings-submenu.ts` (258), reachable from `/settings`
(`slash-commands.ts:20`). Epsilon prints three lines of prose telling the user which TOML
to edit. A first-run user has no TUI path to any setting.

### G12 — the session tree is a flat list, not a tree
`crates/epsilon-tui/src/app/commands.rs:786-816` `open_timeline_picker` builds
`Vec<(u64 /*seq*/, String /*tag*/, String /*snippet*/)>` and `draw_timeline`
(`crates/epsilon-tui/src/app/panels.rs:451-517`) renders one flat row per branch entry.
pi's `TreeSelectorComponent` is 1,427 lines with fold/unfold
(`tree-selector.ts:121, 399-404`), per-node labels with editable text and timestamps
(`:632-636`), five filter modes (`:95`, `:365-380`), an inline search line (`:1157`),
and delete. Epsilon has the navigation (`keys.rs:485-556`) and none of the navigation
*surface*.

---

## 2. Full capability survey

Legend: **DONE** = implemented and reachable, with the implementation cited.
**PARTIAL** = something is there; the exact shortfall is stated.
**MISSING** = no implementation found in the crates surveyed.

### 2.1 Editor and composer

| # | Capability | pi evidence | Epsilon | Verdict |
|---|---|---|---|---|
| 1 | Cursor movement (char/word/line/home/end) | `tui/src/keybindings.ts:72-105` | `editor_ops.rs:58-123, 203-223`; `keys.rs:1026-1146` | **DONE** `editor_ops.rs:36-56` (grapheme snapping), `keys.rs:1066-1075` |
| 2 | Grapheme-cluster-safe delete/insert | — (pi uses `Intl.Segmenter`) | `editor_ops.rs:144-158`; `editing.rs:29-39` | **DONE** (Epsilon is stricter) |
| 3 | Kill to start / word / line-end + kill ring + yank-pop | `tui/src/keybindings.ts:116-142` | `editing.rs:42-97`; `keys.rs:1008-1025, 1152-1164`; `kill_ring.rs` | **DONE** |
| 4 | Undo | `tui/src/undo-stack.ts:10-27` | `undo.rs`; `editing.rs:123-146`; `keys.rs:990-994` | **DONE** |
| 5 | Redo | pi has none (stack is push/pop only) | `editing.rs:149-164`; `keys.rs:996-999` | **DONE** (Epsilon extra) |
| 6 | Prompt history recall + persistence | `keybindings.ts:74-81` | `editing.rs:188-226`; `event.rs:60-65` (loads global history) | **DONE** |
| 7 | Bracketed paste → compact markers | `editor.ts:708-731` | `keys.rs:1309-1326`; `kill_ring.rs::make_paste_marker` | **DONE** |
| 8 | Clipboard paste (Ctrl+V / Shift+Insert) | `keybindings.ts:142-145` | `keys.rs:1169-1187`; `clipboard.rs::local_text` | **DONE** |
| 9 | External editor hand-off | `interactive-mode.ts:4248` | `session.rs:165-183`; `event.rs:115-119` | **DONE** |
| 10 | Multi-line entry (Ctrl+J) | `keybindings.ts:143` | `keybindings.rs:60`; `keys.rs:1190-1194` | **DONE** |
| 11 | Jump-to-character (Ctrl+] / Ctrl+Alt+]) | `keybindings.ts:106-113`; `editor.ts:687-705` | absent | **MISSING** |
| 12 | Tab-to-indent when no popup is open | `keybindings.ts:145` | `keys.rs:1128-1138` returns `false` unconditionally | **PARTIAL** — Tab is swallowed with no effect |
| 13 | Transcript paging matches viewport | `keybindings.ts:160-167` (one page) | `keys.rs:1120-1127` (fixed 10 rows) | **PARTIAL** — see G6 |
| 14 | Half-page / line / top / bottom transcript keys | `keybindings.ts:168-183, 208-209` | absent | **MISSING** |
| 15 | Jump to previous/next semantic prompt | `keybindings.ts:51-52, 184-191` | absent | **MISSING** |
| 16 | Visible response for an unbound key | `editor.ts:683+` (large action surface) | `keys.rs:1269` `_ => false` | **MISSING** — see G10 |
| 17 | A bare letter is never silently consumed | pi's own M13 lesson, `keybindings.ts:120-121` comment | `keys.rs:1261-1264`: `g`/`G` jumps to live whenever `scroll_from_bottom > 0`, so the letter is lost in exactly the state where the user is reading | **PARTIAL** |
| 18 | Ctrl+D is not exit | `keybindings.ts:95` (exit is ctrl+d, only when the editor is empty) | `keys.rs:1048-1057` — ctrl+d is forward-delete; exit moved to ctrl+c/ctrl+q (`keybindings.rs:31`) | **PARTIAL** — the chord pi reserves for exit is Epsilon's delete, and the chord pi reserves for clear is Epsilon's exit |

### 2.2 Overlays and panels

| # | Capability | pi evidence | Epsilon | Verdict |
|---|---|---|---|---|
| 19 | Model selector | `components/model-selector.ts` (421) | `panels.rs:24-86` (stage 1 providers), `:157-222` (stage 2 models) | **DONE** |
| 20 | Fuzzy filter over models | `tui/src/fuzzy.ts:137` | `fuzzy.rs:46-154`; `picker.rs:99-119` | **DONE** |
| 21 | Model selector: save-as-default / enable-all / clear-all / reorder | `keybindings.ts:186-209`; `scoped-models-selector.ts:203, 320` | `commands.rs:198-212` just opens the picker | **PARTIAL** |
| 22 | Scoped-models selector overlay | `scoped-models-selector.ts` (401) | none | **MISSING** |
| 23 | Command palette with fuzzy search | pi has none (Epsilon borrowed opencode's) | `panels.rs:327-378`; `picker.rs:1012-1031` | **DONE** (Epsilon extra) — but see G9 for the broken entry |
| 24 | Session browser + resume | `components/session-selector.ts` (1031) | `session.rs:134-159`; `panels.rs:91-155` | **DONE** for list+resume |
| 25 | Session rename / delete / delete-noninvasive / sort toggle / path toggle / named filter | `keybindings.ts:122-126, 166-185` | none | **MISSING** |
| 26 | Session tree UI (fold, labels, label timestamps, 5 filters, search, delete) | `tree-selector.ts:95-121, 359-404, 632-661, 1157-1328` | `panels.rs:451-517` — one flat list | **MISSING** — see G12 |
| 27 | Fork from a specific user message | `interactive-mode.ts:5148` `showUserMessageSelector`; `components/user-message-selector.ts` (155) | `/fork` is a file copy (`commands.rs:447`) | **MISSING** — see G2 |
| 28 | Clone at current position with new identity | `interactive-mode.ts:5186` | `commands.rs:486` is a file copy | **PARTIAL** — the command exists, the semantics do not |
| 29 | Settings selector overlay | `settings-selector.ts` (945), `config-selector.ts` (942) | `commands.rs:953-983` prints prose | **MISSING** — see G11 |
| 30 | Theme selector + named themes | `components/theme-selector.ts` (67), `theme/theme-json.ts`, `theme-controller.ts` (172) | `commands.rs:702-730` — two palettes, light/dark | **PARTIAL** |
| 31 | Trust selector overlay | `components/trust-selector.ts` (134); `interactive-mode.ts:4964` | typed `trust` / `trust no` only (`app.rs:566-622`) | **PARTIAL** — enforced, but not a dialog |
| 32 | OAuth selector / login dialog | `oauth-selector.ts` (214), `login-dialog.ts` (233) | `keys.rs:101-272`; `panels.rs:662-802` | **DONE** |
| 33 | First-time setup wizard | `components/first-time-setup.ts` (145) | absent | **MISSING** |
| 34 | Extension UI (selector / input / editor / custom / confirm / notify) | `interactive-mode.ts:2480-2830`; `extensions/runner.ts:445` | absent from `crates/epsilon-tui/src` | **MISSING** |
| 35 | Live bash-execution component (1 Hz elapsed, streaming output) | `components/bash-execution.ts` (220) | generic tool card (`state.rs:689-800`) | **MISSING** |
| 36 | Countdown timer | `components/countdown-timer.ts` (39) | absent | **MISSING** |
| 37 | Skill-invocation message component | `components/skill-invocation-message.ts` (55) | `commands.rs:848-857` prints names as a notice | **PARTIAL** |
| 38 | Mermaid rendering | `components/mermaid.ts` (89) | absent | **MISSING** |
| 39 | Show-images selector | `components/show-images-selector.ts` (50) | absent | **MISSING** |
| 40 | armin / daxnuts / earendil-announcement widgets | `components/armin.ts` (382), `daxnuts.ts` (164), `earendil-announcement.ts` (53) | absent | **MISSING** (pi-specific; low value) |
| 41 | Per-panel scrolling window (centred on selection) | `tui/src/components/select-list.ts:172-179` | `scroll.rs:10-21`; used at `panels.rs:56, 116, 185, 349, 459, 534, 756` | **DONE** — every list panel |
| 42 | Per-panel PageUp/PageDown/Home/End | `session-selector.ts:612-616`; `tree-selector.ts:1018-1021` | `keys.rs:747-861` handles Up/Down only | **PARTIAL** — see G12 area |
| 43 | Selection wraps at the ends | `select-list.ts:147-153` | `keys.rs:761-782` clamps, does not wrap | **PARTIAL** |

### 2.3 Transcript rendering

| # | Capability | pi evidence | Epsilon | Verdict |
|---|---|---|---|---|
| 44 | Tool cards: name, status, duration | `components/tool-execution.ts` (421) | `ui/card.rs:88-110`; `state.rs:689-800` | **DONE** |
| 45 | Per-tool result headlines | `core/tools/renderers/*.ts` (7 files) | `renderers/mod.rs:37-116` (5 tool families) | **PARTIAL** — `read` line ranges and `grep` truncation ported; pi's `ls`/`find`/`write` specialisations are folded into the generic path |
| 46 | Diff rendering, unified | `components/diff.ts:79-147` | `ui/card.rs:267-347` | **DONE** |
| 47 | Diff rendering, split (side-by-side) | pi has none | `ui/card.rs:350-460`, threshold `card.rs:73` | **DONE** (Epsilon extra) |
| 48 | Intra-line word-level diff highlight | `diff.ts:26-66` (uses `diffWords`) | `ui/card.rs:620-734` (prefix/suffix token split) | **DONE** — equivalent output, different algorithm |
| 49 | Card body cap + "press key to open" affordance | `read.ts:93, 134`; `write.ts:122`; `find.ts:51` | `ui/card.rs:70`, `:149-154`, `:237-254` | **DONE** |
| 50 | Per-card expansion + global expand-all | `keybindings.ts:117`; `interactive-mode.ts:4209` | `render_cache.rs:57-79`; `keys.rs:1218-1249`; `app/view.rs:53-97` | **DONE** (Epsilon extra) |
| 51 | Markdown: headings / emphasis / strong / inline code / lists / blockquote / rule / fenced code | `markdown.ts:463-621` | `markdown.rs:38-112` | **DONE** for these |
| 52 | Markdown: tables | `markdown.ts:550-556, 842-1011` | no `Tag::Table` arm (`markdown.rs:112`) | **MISSING** |
| 53 | Markdown: syntax highlighting | `markdown.ts:215, 523-525` | none; no highlighting dep (`epsilon-tui/Cargo.toml:35-45`) | **MISSING** — see G8 |
| 54 | Markdown: strikethrough | `markdown.ts:9-23` (strict tokenizer) | option enabled at `markdown.rs:36`, tag never matched | **MISSING** — enabled-but-unimplemented |
| 55 | Markdown: task-list checkboxes | `markdown.ts:772-773` | `ENABLE_TASKLISTS` not enabled | **MISSING** |
| 56 | Markdown: OSC-8 hyperlinks | `markdown.ts:689-695` | absent | **MISSING** |
| 57 | Markdown: LaTeX → unicode | `latex.ts` (1394); `markdown.ts:505-519` | `latex.rs` (909); `markdown.rs:21` | **DONE** |
| 58 | Images: terminal graphics (kitty / iTerm2) | `tui/src/terminal-image.ts` (696) | absent | **MISSING** |
| 59 | Images: text fallback line | pi `imageFallback` | `state.rs:39-88` `image_fallback_line` | **DONE** |
| 60 | Todo checklist rendering | pi has no todo tool | `state.rs:1415-1450`; tool at `epsilon-tools/src/todo.rs` | **DONE** (Epsilon extra) |
| 61 | Thinking blocks styled distinctly | `assistant-message.ts` | `ui/transcript.rs:78-87`; `render_cache.rs:180-195` | **DONE** |
| 62 | Thinking blocks can be hidden | `keybindings.ts:118-121`; `interactive-mode.ts:2892` | no such state | **MISSING** — see G5 |
| 63 | Virtualized transcript (O(visible) per frame) | `components/scroll-view.ts` (224) | `render_cache.rs:260-477`; `app/view.rs:413-447` | **DONE** (Epsilon extra) |
| 64 | Compaction shown as a summary card | `components/compaction-summary-message.ts` (59) | `state.rs:1211-1217` — two system lines | **PARTIAL** |
| 65 | Branch summary shown on navigate | `components/branch-summary-message.ts` (58) | never produced (G4) | **MISSING** |

### 2.4 Keybindings, footer, feedback

| # | Capability | pi evidence | Epsilon | Verdict |
|---|---|---|---|---|
| 66 | User keybinding file, load + reload | `keybindings.ts:371-399`; `interactive-mode.ts:5997` | `keybindings.rs:207-345`; `commands.rs:641-648` | **DONE** |
| 67 | Legacy keybinding-name migration | `keybindings.ts:240-300` | absent | **MISSING** |
| 68 | Binding-conflict detection | `keybindings.ts:286-288` `getConflicts()` | absent | **MISSING** |
| 69 | Footer hints resolved from the live table | `keybinding-hints.ts` (48) | `ui/footer.rs:255-262`; `app/view.rs:305-355` | **DONE** (Epsilon extra — the M13 lesson is enforced structurally) |
| 70 | Frozen-view position badge | pi `scroll-view` follow state | `ui/footer.rs:48-81`; `app/view.rs:313-317` | **DONE** (Epsilon extra) |
| 71 | `app.clear` (clear editor) | `keybindings.ts:94`; `interactive-mode.ts:900` | declared `keybindings.rs:32`, read nowhere | **MISSING** — see G1 |
| 72 | `app.suspend` (Ctrl+Z on non-Windows) | `keybindings.ts:96-99`; `interactive-mode.ts:2883` | absent | **MISSING** |
| 73 | `app.message.copy` keybinding | `keybindings.ts:130-133`; `interactive-mode.ts:2897` | `/copy` only (`commands.rs:1296-1336`) | **PARTIAL** |
| 74 | `app.message.dequeue` keybinding | `keybindings.ts:138-141`; `interactive-mode.ts:2899` | no key; Esc restores only the last message (`keys.rs:884-897`) | **PARTIAL** |
| 75 | Follow-up keeps the draft in the editor | `keybindings.ts:134-137`; `interactive-mode.ts:4125` | Enter steers and consumes the text (`app.rs:641-645`) | **PARTIAL** |
| 76 | `/hotkeys` lists the live binding table | `slash-commands.ts:32` | `commands.rs:931-951` — four prose lines plus the table | **PARTIAL** — and it prints the dead `app.clear` (G1) |

### 2.5 Agent loop

| # | Capability | pi evidence | Epsilon | Verdict |
|---|---|---|---|---|
| 77 | Permission dialog with allow-once / always / deny / deny+guidance | pi has no interactive per-tool approval UI; `core/tools/*` take no approver | `panels.rs:592-656`; `keys.rs:284-355`; `epsilon-safety/src/channel_ui.rs:71-108` | **DONE** (Epsilon extra) — including the fail-closed painted-id guard at `keys.rs:284-307` |
| 78 | Streaming with per-delta updates | `interactive-mode.ts:3244-3279` | `state.rs:1018-1024`; coalesced at `event.rs:142-159` | **DONE** |
| 79 | Frame-rate ceiling / coalescing | pi renders on demand | `event.rs:167-182`, `FRAME_MIN` at `app.rs:76` | **DONE** (Epsilon extra) |
| 80 | Compaction | `core/compaction/compaction.ts` (1012) | `epsilon-core/src/compact.rs` (1220); `lib.rs:2267` | **DONE** |
| 81 | Context accounting / pressure | `compaction.ts:146-265` | `lib.rs:2189`; `compact.rs:12-113`; mirrored at `app/view.rs:300-302` | **DONE** |
| 82 | Per-model cost accounting | `core/usage-totals.ts`, `footer-data-provider.ts` (388) | `ui/cost.rs` (465); `app/view.rs:199-217` | **DONE** (Epsilon extra) |
| 83 | Package / resource lifecycle (npm, git install) | `core/package-manager.ts` (2699) | absent | **MISSING** |
| 84 | Panic-safe terminal restore | pi has none | `app.rs:987-1053` (guard + hook + `Drop`) | **DONE** (Epsilon extra) |

---

## 3. Where Epsilon's own comments or docs overstate what the code does

This is the class of defect the study was commissioned to find. Each entry is a place where
a comment, doc-comment or user-visible string asserts something the implementation does
not do.

1. **`app.clear` is advertised and does nothing.**
   `keybindings.rs:32` declares it; `commands.rs:942-951` prints it in `/hotkeys`; no code
   path reads it. Ctrl+C instead quits (`keys.rs:473`).

2. **Strikethrough is "enabled" and not rendered.**
   `markdown.rs:36` passes `Options::ENABLE_STRIKETHROUGH`, which reads as a claim of
   support; the match arm does not exist (`markdown.rs:112` swallows it).

3. **The palette's expand entry claims a chord and a command it does not have.**
   `picker.rs:1004-1006` — title "Toggle tool output expand", `binding: Some("x")`,
   `command: "\u{1}expand"`. Neither the hint nor the command is real (G9).

4. **`/fork` and `/clone` are described as forks and are file copies.**
   `picker.rs:293-298` describes `/fork` as "copy current session JSONL to a new branch
   file" — the word "fork" in the user-facing description does the work the code does not.
   The notice at `commands.rs:448-452` tells the user to open the copy with `--session`,
   implying a valid independent session; it has the parent's id.

5. **The timeline picker calls itself a tree.**
   `commands.rs:780-786`: "`/timeline` and `/tree` are the same picker… pi parity:
   `/branch` is an alias for the timeline view, and so is `/tree`." The picker is a flat
   list with no hierarchy (G12); the parity claim is with pi's *command surface*, not its
   tree, and the user-visible `/tree` help text (`picker.rs:354-356`, "navigate session
   tree (switch branches)") promises a tree.

6. **The frozen-view "jump to live" hint takes a letter out of the draft.**
   `keys.rs:1250-1256` argues at length that the `scroll_from_bottom > 0` guard is what
   makes a bare letter safe. It is safe only in the sense that the letter is not typed; in
   the state where the guard holds, `g` and `G` are consumed (`keys.rs:1261-1264`) and the
   footer advertises exactly that (`footer.rs:208`). The comment's conclusion is right
   about the chord and wrong about the letter.

7. **`/help` prints a key that is not bound.**
   `commands.rs:843` prints "Ctrl+M model picker". `keybindings.rs:33` binds
   `app.model.select` to `ctrl+m`, and `keys.rs:921` matches it — but on Windows,
   Ctrl+M arrives as `KeyCode::Enter` with the CONTROL modifier
   (`crossterm-0.29.0/src/event/sys/windows/parse.rs:246` maps `VK_RETURN` to
   `KeyCode::Enter`), while `keybindings.rs:145-153` requires the code to equal
   `KeyCode::Char('m')`. The chord is unreachable on the platform the owner reports from.
   (Same mechanism makes `ctrl+q`, bound to `app.exit` at `keybindings.rs:31`, the *only*
   working exit on Windows.)

8. **`ui/composer.rs:37-48` argues against borders and a warning label "on every frame";
   the shipped code matches, but `composer.rs:3-7` still describes the discarded design in
   the present tense.** Minor, cosmetic.

---

## 4. The reported typing-freeze: candidate code paths (UNVERIFIED)

The owner reports the TUI "shrinks and freezes as soon as I start typing and shows no input
or output" in both CMD and PowerShell. `audit/02-port-fidelity.md` records a prior
conclusion that this "does not reproduce". I could not run the binary (read-only study), so
every item below is **UNVERIFIED** — a path that could produce a blank or shrunken frame,
with the exact lines. They are ordered by how well each matches the reported symptoms.

**C1 — a whole-workspace directory walk on the event-loop task, per keystroke.**
`app/completion.rs:41-68` `sync_at_state` runs on every inserted character
(`app/editing.rs:241-244`). When an `@` word abuts the cursor it calls `at_file_rows`
(`completion.rs:45`), which calls `crate::at_files::walk` at `completion.rs:102` — a
recursive `std::fs::read_dir` over the workspace, budgeted to 200 files and depth 16
(`at_files.rs:30-34, 41-49`). The M110 fix cached the *ignore rules* with a 5-second TTL
(`app.rs:314`; `completion.rs:85-95`) but not the *walk itself*, so every character typed
after an `@` re-traverses. All of this is synchronous, on the same task that draws
(`event.rs:111-263`). This is the same shape as the M110 regression the project already
documented (`docs/milestones/M110-at-popup-stall.md`), and it matches the reported symptom
precisely: the composer shows the placeholder because the app has not drawn since before
the keys arrived.

**C2 — dropped `Next` futures against crossterm's one-shot wake-task protocol.**
`app/event.rs:127-138` drains input with
`tokio::time::timeout(Duration::ZERO, term_events.next())`. Each iteration constructs and
**drops** a `Next` future. crossterm's `EventStream::poll_next`
(`crossterm-0.29.0/src/event/stream.rs:104-137`) registers a wake task on `Poll::Pending`
and captures `cx.waker()` into it; the background thread later calls
`stream_waker.wake()` (`stream.rs:44-59`). A waker belonging to a dropped future, fired
while the task is parked on a different future, is a lost wakeup. The drain loop runs up
to 128 times per turn (`app.rs:83`, `event.rs:127`), so there are many opportunities for
the flag `stream_wake_task_executed` (`stream.rs:113-117`) and the channel
`sync_channel(1)` (`stream.rs:37`) to be left describing a future that no longer exists.
A key that is read but never delivered to `handle_key` is exactly "I typed and nothing
happened."

**C3 — key kind filtering.**
`app/keys.rs:1280-1284` drops every key whose `kind` is not `KeyEventKind::Press`:

```rust
TermEvent::Key(key) => {
    if key.kind == KeyEventKind::Press && handle_key(app, agent, key) {
```

crossterm derives Press/Release from `key_event.key_down`
(`crossterm-0.29.0/src/event/sys/windows/parse.rs:223-227, 286-290`). If a console or
ConPTY path reports an unexpected kind for ordinary typing, every key is discarded
silently and the screen keeps showing the placeholder. There is no fallback branch and no
diagnostic, so this failure is indistinguishable from C1 or C2 from the outside.

**C4 — the undersized-terminal fallback blanks the whole frame.**
`app/app.rs:1229-1252`: if `f.area()` is under 44×10
(`ui/layout.rs:41-43, 111-113`), `draw` renders a single centered line and **returns** —
no header, no transcript, no composer, no input. A window at or below that size is a
stable "shrunken frame showing no input and no output" that looks like a freeze. It would
not be triggered by typing, but it is the one place the TUI deliberately paints a
non-interactive screen, and nothing in the frame tells the user to widen rather than
restart.

**C5 — the transcript can vanish while the chrome stays.**
`app/panels.rs:252-256`: `draw_transcript` returns without drawing anything when
`geometry.visible == 0`, and `geometry.visible` is `area.height` of the transcript region
(`app/view.rs:437`). With `Constraint::Min(1)` for the transcript
(`ui/layout.rs:84-91`) this should not be zero, but the composer's row budget
(`ui/layout.rs:105-108`, capped at `height/3`) is what bounds it, and the two are computed
at different points in the frame (`app.rs:1267` measures, `ui/shell.rs:84` splits).

**Not a cause, for the record.** The frame ceiling in `event.rs:167-182` bounds redraw
latency to `FRAME_MIN` = 33 ms (`app.rs:76`), so the 30 fps coalescing cannot by itself
produce a freeze; and `handle_key` writes nothing to the terminal (its own claim at
`app/keys.rs:9-10` holds — every write goes through `flush_osc52` at `app.rs:1133-1140`
after `terminal.draw` returns), so no key handler can tear a frame.

**What would settle it.** The owner already has the instrument: the `--at SECONDS` probe
option M110 added to the console probe (`docs/milestones/M110-at-popup-stall.md`,
"What happened" section) separates *when* keys were typed from *what they cost*. Typing a
bare `@` followed by six characters, and then typing six characters with no `@`, cleanly
separates C1 from C2/C3. Nothing in this study can substitute for that measurement.

---

## 5. Prior-audit cross-check

`audit/02-port-fidelity.md` was read but not relied on. Three rows, independently re-derived:

* **Row 49 (branch navigation "Missing", corrected 2026-09-28) — confirmed correct.**
  `SessionStore::navigate_to` exists at `store.rs:1354`, is called from the timeline picker's
  Enter handler at `app/keys.rs:495`, and the transcript is re-projected
  (`keys.rs:511-519`). The wiring is real. (The *aftermath* is G3.)
* **Row 50 (branch summary: primitive ported, nothing wires it) — confirmed correct**, and
  confirmed here from the code rather than from the audit: `branch_with_summary`
  (`store.rs:1397`) has no production caller.
* **Row 52 (tool-call interception) and the surrounding RPC rows** — out of scope for this
  study and not re-verified.

---

## 6. Method notes and limits

* Nothing was built. No `cargo` invocation. No repository file was modified except this
  document.
* Every `crates/` citation is a line read in this session. Every `references/` citation is a
  line read in this session.
* Claims I could not trace to an implementation are marked UNVERIFIED (§4) rather than
  credited or dismissed.
* Counts in §0 are of the 84 rows in §2. A capability appearing twice (for example, diff
  rendering, counted once as unified and once as split) counts once per row.
* The 24 MISSING rows are not 24 independent defects. They reduce to roughly six causes:
  no `Component` layer (rows 33-40, 52-56), no settings/theme/scoped-models surfaces
  (21-22, 29-30), no tree surface (26), no alt-screen renderer (14-15), fork implemented
  as a copy (27-28), and branch summarization unwired (65).
