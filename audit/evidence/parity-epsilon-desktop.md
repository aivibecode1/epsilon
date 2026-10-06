# `crates/epsilon-desktop` — a source-derived inventory

**What this file is.** The "ours" half of the DSH Desktop parity story: where Epsilon's native
window implements each behaviour it has, with a line to look at. The reference half is
[`parity-dsh-desktop-reference.md`](parity-dsh-desktop-reference.md). The **row-by-row pairing**
of the two lives in [`../../docs/parity/dsh-desktop.md`](../../docs/parity/dsh-desktop.md), which
reads both files and this crate.

**Why this file is an index and not a third row list.** An earlier survey produced 60 behaviour
rows; its session ended before it wrote them out, and the rewrite that followed did not
reconstruct all sixty. Rather than publish sixty plausible sentences this repository cannot
check, the durable parts are written down here — the module map, the command table, the keyboard
surface, the accessibility surface and the findings — and the pairing is done against the source
where it is written down, with a citation. **A summary of an audit is not the audit**, and a
fabricated row is worse than a missing one.

**Every `path:line` in this file was opened while writing it**, not transcribed. Two drifted and
were corrected against the source rather than copied: `lib.rs:run` is at **124** (an earlier draft
said 118), and `commands.rs:refusal_for` is at **296** (an earlier draft said 297).

**No cargo was run for this file.** Every "test that pins it" below is a reading of a test's
source, and is labelled `state` or `frame` to say so: `frame` means the test drives the
production draw path and reads the screen back.

---

## 1. Module map

| File | Owns |
|---|---|
| `src/main.rs` | CLI parsing, exit codes. `main` `src/main.rs:26` |
| `src/lib.rs` | the crate surface, composition-before-window, `Options` `:79`, `run` `:124` |
| `src/app.rs` | **application state and the whole draw path** — ~10,700 lines, the largest file in the workspace |
| `src/layout.rs` | pane geometry, the five degradation tiers, the per-workspace preferences file. `Preferences` `:117`, `IdeLayout::new` `:367` |
| `src/theme.rs` | the palette borrowed from `epsilon_tui`, contrast floors, egui `Visuals`. `IdeTheme` `:141` |
| `src/session.rs` | the agent handle, the event-to-row reducer, the async→frame bridge. `IdeSession` `:736` |
| `src/workspace.rs` | file tree, read-only editor, change review — all painting |
| `src/commands.rs` | typed-slash-command dispatch and the wording of a refusal |
| `src/markup.rs` | assistant markdown → egui layout jobs, with a parse cache |
| `src/diff.rs` | snapshot-and-compare for the files a tool touched |
| `src/frame_raster.rs` | offscreen rasterisation of a real frame — **test-only**, gated `#[cfg(test)]` at `src/lib.rs:56` |

---

## 2. Commands: 7 run, 30 refused by name

Measured from `crates/epsilon-desktop/src/commands.rs`, and re-confirmed by
`the_refusal_set_is_the_parser_heads_the_window_lacks`, which asserts the subtraction rather than
trusting it (`commands.rs:546-565`).

**`COMMAND_HEADS` holds 37 rows** (`crates/epsilon-contracts/src/command.rs:383`). **39** is the
non-`Unknown` *variant* count of `enum Command`, and the two are not meant to match:
`Models` / `Model(String)` are two variants for one head. Write **37** in any denominator.

### Run by this window — `SUPPORTED`, `commands.rs:449`

| head | what it does | dispatch |
|---|---|---|
| `compact` | compact context on a spawned task | `commands.rs:117` → `app.rs:2450` |
| `goal` | show / help / create / pause / resume / clear | `commands.rs:121` → `:198-261` |
| `sessions` | switch the sidebar to Sessions | `commands.rs:124` |
| `session` | one-line report: file, model, context, tokens, run state | `commands.rs:139` → `:315` |
| `tasks` | switch the sidebar to the subagent pane | `commands.rs:132` |
| `clear` | empty the transcript view, keep durable state | `commands.rs:143` |
| `theme` | flip or select a palette, live, never written to config | `commands.rs:157` |

### Refused by name — 30 of them

`model`, `think`, `branch`, `tree`, `fork`, `clone`, `new`, `resume`, `editor`, `trust`, `reload`,
`export`, `import`, `restore`, `share`, `copy`, `name`, `changelog`, `hotkeys`, `settings`,
`scoped_models`, `expand`, `skills`, `skill`, `templates`, `search`, `logout`, `exit`, `help`,
`login`.

Lookup is by discriminant over `COMMAND_HEADS` (`commands.rs:372-380`), so a new registry head is
refused **by name with no edit to this crate**. `window_runs` is at `commands.rs:393`;
`refused_heads` at `:410`.

### Palette ids with no slash spelling — 8

Declared in `app.rs:806-838`, dispatched by `run_palette_action` `app.rs:962`:
`view.files`, `view.subagents`, `view.sessions`, `tree.reload`, and four goal verbs
(`goal.create`, `goal.pause`, `goal.resume`, `goal.clear`).

**The desktop imports `SLASH_ITEMS` nowhere**, so its seven working commands are advertised by no
autocomplete, no palette row and no composer hint. `/help` and `/hotkeys` are refused here even
though the window has at least eight live bindings.

---

## 3. Keyboard: literals, not a registry

**`keybindings`, `KeybindingsManager` and `key_text` appear nowhere in `crates/epsilon-desktop`.**
Every chord is a literal egui `consume_key`:

| chord | action | where |
|---|---|---|
| `Ctrl+P` | toggle the quick-open palette | `app.rs:880` |
| `Enter` / `Escape` | open / close the palette | `app.rs:897-898` |
| `↑` / `↓` | move the palette selection | `app.rs:898-901` |
| `Ctrl+B` | fold / unfold the sidebar | `app.rs:1748` |
| `←` / `→` | resize the sidebar by 16 pt | `app.rs:1757-1758` |
| `Shift+F6` / `F6` | move the focus ring | `app.rs:1830-1831` |
| `Escape` | leave a focused pane for the composer | `app.rs:1867` |
| `↑` / `↓` / `Enter` | tree cursor and open | `app.rs:1870-1875` |
| `Ctrl++` / `Ctrl+-` / `Ctrl+0` | zoom | `app.rs:2669-2672` |
| `Enter` | send the draft | `app.rs:5500-5520` |
| `Escape` | clear draft / abort run / reclaim focus | `app.rs:5345`, `:5204-5223` |
| `Enter` / `Escape` | allow once / deny, on a parked ask | `app.rs:5530`, `:5358` |

The hint text comes from a hand-written constant, `SIDEBAR_TOGGLE_KEY: &str = "ctrl+b"`
(`app.rs:759`), used at `:2766`, `:2768`, `:3186` and `:3198`.

**This contradicts `AGENTS.md` §2.4**, which says keybindings are "declared once and read by every
front end". The comment above the constant says the registry "does not exist yet" and that the two
front ends are separate namespaces — **the code is honest and the locked rule overstates.** The
practical consequence: **the window's shortcuts cannot be rebound, while the terminal's can.**
Filed as proposal 8 in [`../../docs/AGENTS-PROPOSALS.md`](../../docs/AGENTS-PROPOSALS.md).

---

## 4. Accessibility

egui allocates an AccessKit node per focusable widget; this crate registers its own names and
roles on top. **13 `name_region` call sites, 10 `name_node`, 3 `name_role`, 4 `name_scrollbar`** —
call sites, not a count of live nodes.

| surface | role | where |
|---|---|---|
| title bar | `Banner` | `app.rs:2839-2847` |
| status bar | `Status` | `app.rs:2909-2918` |
| activity bar strip | `Navigation` | `app.rs:3193-3202` |
| sidebar resize handle | **`Splitter`** | `app.rs:3376-3392` |
| files / subagents / sessions pane | `Region` | `app.rs:3771`, `:3361`, `:3492` |
| file-tree row | **`TreeItem`** | `workspace.rs:383-435` |
| editor pane (3 states) | `Region` | `app.rs:3883-3915` |
| goal band | `Status` | `app.rs:4759-4771` |
| transcript | **`Log`** | `app.rs:4601-4617` |
| changed-file row | **`Link`** | `app.rs:5074-5075` |
| scrollbars (4) | egui `ScrollBar` | `app.rs:4566`, `workspace.rs:451, 586, 701` |

**Two known limits, both documented in the source rather than hidden:**

- **Every node attaches flat to the AccessKit root.** The pane `Ui`s carry no node, so the
  `Region` pane and its `TreeItem` rows are **siblings, not a tree** (`workspace.rs:414-434`).
  A comment once claimed this was invalid ARIA nesting and that making the pane a `Role::Tree`
  was the fix; that claim was **wrong** — flipping the role would change one label and create no
  structure.
- **Nothing carries a description or a relation.** `name_node` writes only `Node::label`
  (`app.rs:268-272`).

`no_node_in_the_window_is_unroled` (`app.rs:9233`) asserts roles only; the naming half is
`the_accessibility_names_read_correctly_and_the_filter_is_named` (`app.rs:9182`). **A node with a
role and an empty label passes both**, which is the gap those two tests leave between them.

---

## 5. Findings, ranked

1. ~~**`shift+enter` is advertised and does the opposite.**~~ **CLOSED (M313).** The composer
   placeholder says `ask anything · enter sends · shift+enter for a new line`, and
   `draw_composer`'s doc repeated it — but the handler took
   `consume_key(Modifiers::NONE, Key::Enter)` **before** the `TextEdit` is drawn, and egui's
   `consume_key` matches modifiers **logically** (`egui-0.36.2/src/input_state/mod.rs:717-718`),
   so `Shift+Enter` matched that read and the draft was **sent**. Fixed by matching the most
   specific chord first, as egui's own doc instructs (`mod.rs:719-721`) — and by reading the
   **event's** modifiers, not `InputState::modifiers`, which `begin_pass` never fills from an
   event. Pinned by `shift_enter_is_a_newline_in_the_composer_not_a_send`.
2. ~~**The sessions sidebar is unreachable by keyboard.**~~ **CLOSED (M313).** `pane_keys`
   returned early unless `sidebar == SidebarView::Files`, so `F6` into a Sessions sidebar gave a
   focus ring and **no key at all** — WCAG 2.1.2, a keyboard trap. The gate is now on *which list
   is on screen*, each list answering with its own rows: a `session_cursor` path, a `▸` marker
   **drawn** on the row (a cursor a reader cannot see is not a cursor), and `Enter` calling the
   same `switch_session` the click path calls. Pinned by
   `the_sessions_sidebar_answers_the_keyboard_and_shows_where_it_is`.
3. **Zoom keys, `←`/`→`, the `Allow`/`Refuse` buttons and the `Send` button have no test that
   drives the key or the click.** Every zoom test calls `ctx.set_zoom_factor` directly, so deleting
   the whole zoom block would leave them green. **Partly unblocked at M313**: the in-crate
   `Harness` built its `RawInput` **unfocused**, and egui drops keyboard events for an unfocused
   window — so no in-crate test could have typed a key at all. It is focused now.
4. **The file-tree footer and the palette overflow note are asserted nowhere.**
5. **`FrameSummary::tabs` is written and read by no test** — dead instrumentation.
6. **Palette rows are `Sense::empty()`**, so they are not focusable; a keyboard reader must use
   arrows and `Enter`. Consistent, and tested — but worth knowing against a reference whose palette
   rows are options.
7. **Tool-call diffs only cover paths the call names** (`diff.rs:203-209`). A `bash` command that
   edits files is not reviewed.

---

## 6. M313 — what changed after this survey

**Nothing in this crate.** The M313 code work landed in the terminal UI
(`crates/epsilon-tui`), not the window: a reachable `unreachable!()` panic in the model picker, two
bindings the footer printed and no handler read, and the streaming tail's text-safety gap. The
M313 code work is recorded in
[`../../docs/parity/pi-tui.md`](../../docs/parity/pi-tui.md) and in
[`../../docs/STATUS.md`](../../docs/STATUS.md).

Four **documentation** fixes landed here, all because a survey read the code against the prose:

| Where | Said | Now |
|---|---|---|
| `commands.rs` (three places) | "six commands", "the other thirty-six" | the hand-written counts were **deleted**, not corrected |
| `src/lib.rs` module map | 3 modules | all 9, plus the test-only rasteriser |
| `README.md` entry points | 4 of 5 rows stale | all re-derived, plus a row for `try_command` |
| `src/main.rs` | "the CLI can offer the same window behind a flag" | `epsilon-cli` has no dependency on this crate |

Findings 1 and 2 above are **still open** and this file is where a later session should come.