# Plan 024 §5 — the desktop's command coverage, measured against the tree

**Date:** 2026-10-04 · **Scope:** read-only investigation · **Build owner:** another process; no
`cargo` was run, and no file outside this one was touched.

**Method.** Read, Grep and Glob only. Every count below was produced by a pattern that matched
against the file it is counted in, and the line numbers were opened, not remembered.

---

## 0. The headline number

**The plan's "3 of 38" is right about the numerator and loose about the denominator.**

| Reading | Total | Run by the desktop | Ratio |
|---|---|---|---|
| Registry rows — `COMMAND_HEADS` (`crates/epsilon-contracts/src/command.rs:371`) | **37** | **3** | 8.1 % |
| `Command` variants excluding `Unknown` (`command.rs:240-358`) | **38** | **3** | 7.9 % |

`3` is exact: `SUPPORTED` is `["compact", "goal", "sessions"]`
(`crates/epsilon-desktop/src/commands.rs:364`) and `try_command` has exactly three run arms —
`Command::Compact` (`commands.rs:117`), `Command::Goal` (`commands.rs:121`), `Command::Sessions`
(`commands.rs:124`).

**Why 37 and not 38.** `enum Command` has 39 variants; `Unknown` is one of them; and `Models`
(bare `/model`) and `Model(String)` (`/model gpt`) are **two variants for the single head
`model`**. So 37 heads map onto 38 non-`Unknown` variants, and `COMMAND_HEADS` holds 37 entries —
counted by matching `^    "[a-z?_]+",$` inside `:371-432`, which returned 37 lines (`:372`–`:431`).
The "38" the plan uses is the variant count, which is what the deleted 38-arm `head_of` match
counted (`commands.rs:31`, `commands.rs:278`). Both denominators give the same answer; **the
registry denominator is the honest one**, because `refused_heads()` subtracts from
`COMMAND_HEADS` (`commands.rs:334-342`).

---

## 1. The shared registry in `epsilon-contracts`

`crates/epsilon-contracts/src/command.rs` — declared at `:47`, `:240`, `:371`, `:446`, `:464`.

| Item | Line | Rows | Shape |
|---|---|---|---|
| `SlashItem` | `:37-43` | — | `value: &'static str`, `description: &'static str` |
| `SLASH_ITEMS` | `:47-236` | **43** | what `/help` prints and the `/` autocomplete suggests |
| `Command` | `:240-358` | **39** variants | parsed command; `Unknown(String)` is the 39th |
| `COMMAND_HEADS` | `:371-432` | **37** | canonical head per command, aliases deliberately absent |
| `ALIAS_HEADS` | `:446-457` | **8** | `models`→`model`, `timeline`→`branch`, `auth`/`connect`→`login`, `quit`/`q`→`exit`, `scoped-models`→`scoped_models`, `?`→`help` |
| `parse_command` | `:464-567` | — | hand-written `match`, kept verbatim by decision (`command.rs:26-33`, ADR-024 `:2040-2044`) |

**There are no capability flags.** No row anywhere in this file says which front end runs it.
`SlashItem` is two strings; `COMMAND_HEADS`/`ALIAS_HEADS` are `&[&str]`; per-front-end capability
exists only as each frontend's own list. That absence is the whole of plan §5's "Done when".

**What *is* checked, and where.** Three tests, all beside the parser, all reading the parser's own
text rather than a list of their own:

- `every_head_in_command_heads_is_parsed` — `command.rs:574`
- `every_offered_command_is_reachable_from_slash_items` — `command.rs:594` (head → `SLASH_ITEMS`)
- `the_parser_does_not_know_a_command_the_list_does_not` — `command.rs:620`; it reads its own
  source with `include_str!` (`:632`) and scans the quoted tokens of `parse_command`

The third is the only reverse-drift check in the tree, and it is honest about being source-shaped
(`:615-618`). **It checks heads, not arguments** — which is how the finding in §7.1 survives.

---

## 2. The desktop side

### 2.1 The dispatcher

**`IdeApp::try_command` — `crates/epsilon-desktop/src/commands.rs:111-141`.**

```
:112  let Some(command) = parse_command(text) else { return Outcome::Send(text.to_string()) };
:117  Command::Compact  => { self.run_compact(); Outcome::Ran("compacting context…") }
:121  Command::Goal(a)  => self.run_goal_argument(a)
:124  Command::Sessions => { self.set_sidebar_view(SidebarView::Sessions); Outcome::Ran(..) }
:139  other              => Outcome::Refused(refusal_for(&other, typed_head(text)))
```

The catch-all at `:139` is the refusal path. It takes **the reader's own spelling**
(`typed_head`, `:269-273`) and never the canonical head, which is why `/timeline` is refused as
`/timeline` rather than as `/branch` (`:257-268`, test `:550`).

**Production caller: exactly one** — `IdeApp::send_draft`, `crates/epsilon-desktop/src/app.rs:4832`,
calling `try_command` at `app.rs:4860`. Every other call site is in
`crates/epsilon-desktop/tests/window.rs` (`:1515, 1568, 1586, 1615, 1696, 1714, 1743, 1747, 1784`).
**No unit test in `commands.rs` calls `try_command` at all** — those tests call the helpers
directly (`window_runs` at `:395, :437`; `head_of` at `:442, :506, :525`; `refusal_for` at
`:398, :557, :595, :645`). `send_draft`'s three-way branch on `Outcome` is `app.rs:4860-4889`.

### 2.2 What `window_runs` actually does — the plan's claim is confirmed

**`window_runs` — `commands.rs:317-319`:**

```rust
fn window_runs(command: &Command) -> bool {
    head_of(command).is_some_and(|head| SUPPORTED.contains(&head))
}
```

**It is not on the production path.** Verified by grep over the whole crate for every identifier in
this chain:

| Symbol | Defined | Production callers | Non-production callers |
|---|---|---|---|
| `try_command` | `commands.rs:111` | `app.rs:4860` | tests only |
| `window_runs` | `commands.rs:317` | **none** | `commands.rs:339` (inside `refused_heads`), `commands.rs:395`, `:437` |
| `refused_heads` | `commands.rs:334` | **none** | `tests/window.rs:1668` |
| `SUPPORTED` | `commands.rs:364` | **none** | `commands.rs:318`, `:428`, `:463`, `tests/window.rs:1675, 1679, 1712` |

`send_draft` → `try_command` reads `parse_command`, `refusal_for` and `typed_head`, and **no
declared list at all**. So a name added to `SUPPORTED` with no arm behind it changes nothing a
reader can see — it changes `refused_heads()`, which only a test reads.

The file says this itself, in the module doc and on the function: *"the window's own run/refuse
split is the three arms in `IdeApp::try_command`, not this predicate… the declaration and the
implementation are two statements of one fact and one test says so"* (`commands.rs:309-316`).

**One correction to the plan, in the plan's favour.** The plan says the two statements are joined
by **one** test (plan `:148-152`). There are **two**, and the second is stronger than the plan
credits:

1. `the_window_runs_exactly_the_heads_it_declares_supported` — `commands.rs:614`. Compares a
   **hand-written** list `[Command::Compact, Command::Goal(None), Command::Sessions]` (`:616`)
   against `SUPPORTED`.
2. `every_command_head_the_window_lacks_is_refused_by_name` — `tests/window.rs:1667`. Drives
   **every** refused head through the real `try_command` (`:1696`) and **every** `SUPPORTED` head
   back through it (`:1714`), requiring `Outcome::Ran`. This is the recorded negative control for
   the list-comparison half: adding `"tree"` to `SUPPORTED` with nothing behind it left the *first*
   test's list comparison green (`tests/window.rs:1646-1650`), and the second half exists because
   of it.

So the join is real and it is tested against production dispatch — **but it is a test, not a
construction.** Adding a fifth arm to `try_command` without touching `SUPPORTED` produces a
working verb the window still refuses by name, and both tests stay green, because test 1 compares
a list the author also wrote and test 2 only drives the names that list already names. That gap is
the real work in plan §5, and it is the gap the plan names.

### 2.3 A seventh capability table nobody reconciles

`crates/epsilon-desktop/src/app.rs:743-775` declares `fn commands() -> Vec<Command>` — **8 rows**:
three sidebar views (`:761-763`), `tree.reload` (`:764-769`), and four goal verbs (`:770-773`).
It is production: `palette_entries` (`:783`) reads it, `palette_keys` (`:816`) drives it from
`Ctrl+P`, and `run_palette_action` (`:899`) runs it.

Nothing joins it to `SUPPORTED`. Grep for `SUPPORTED` across `crates/epsilon-desktop` returns only
`commands.rs` and `tests/window.rs` — never `app.rs`. Two facts fall out:

- `view.sessions` is `/sessions` and `goal.*` is `/goal …`, so **5 of the 8 palette rows are
  capability the window already declares in `SUPPORTED` under a different list.**
- `tree.reload` has **no** slash head, and the terminal's `/reload` is a different verb entirely —
  it reloads skills, prompts and config (`command.rs:321-322`; executor
  `crates/epsilon-tui/src/app/commands.rs:1386`). Mapping `/reload` onto `ReloadTree` would be a
  window that advertises a name and does something else: `AGENTS.md` §2.7.

ADR-024 names six such tables (`docs/design/epsilon-decisions.md:1973-1976`: `enum Command`,
`COMMAND_HEADS` + `ALIAS_HEADS`, `SLASH_ITEMS`, `palette_entries()`, `keybindings::DEFAULTS`, and
"a sixth in the desktop — its own `SUPPORTED` list and a 38-arm `head_of`"). The desktop's palette
table post-dates that paragraph, is **not** in it, and is the seventh.

---

## 3. The terminal side

**Yes — the terminal and the desktop genuinely read one registry today**, for the *vocabulary*.
Since M290 the parser and the three tables live in the crate below both front ends
(`crates/epsilon-contracts/src/command.rs:1-33`), and:

- the terminal calls it through a re-export: `crates/epsilon-tui/src/picker.rs:31` re-exports
  `ALIAS_HEADS, COMMAND_HEADS, Command, SLASH_ITEMS, SlashItem, parse_command`; the submit path
  is `crates/epsilon-tui/src/app.rs:856-857`;
- the desktop imports it directly: `crates/epsilon-desktop/src/commands.rs:90`.

**But the terminal does not maintain a "supported commands" list, and that is the key asymmetry.**

- **Dispatch:** `App::run_command` — `crates/epsilon-tui/src/app/commands.rs:471-519`. A five-way
  fan-out over `Command` variants into `run_provider_command` (`:522`), `run_session_command`
  (`:663`ff), `run_view_command` (`:1828`ff), `run_transfer_command` (`:2091`ff) and
  `run_workspace_command` (`:1490`ff), with `_ => run_workspace_command(...)` at `:517` as the
  catch-all. **A new `Command` variant is a compile error there** until some arm claims it. The
  terminal's capability is therefore *exhaustive by construction*.
- **What it does maintain instead:** `keybindings::DEFAULTS` (`crates/epsilon-tui/src/keybindings.rs:27`,
  closing `]` at `:150`) — **24 rows**, counted by `^    \("[a-z]` **or** `^        "[a-z]`; the
  two-pattern count matters, because `app.transcript.nextPrompt` (`:60`) is written across three
  lines and a single-indent pattern reads 23. And the command palette `picker::palette_entries()`
  (`crates/epsilon-tui/src/picker.rs:541`, `vec![` `:542` closing `:594`) — **7 rows**, `:543`-`:593`.
  Both tables are hand-written and neither is checked against `COMMAND_HEADS`.

**So `AGENTS.md` §2.4 holds on the axis that matters (what a `/word` *means*) and fails on the axis
that is being asked about (which front end runs it).** The desktop's answer is a three-name const
plus three arms; the terminal's answer is the type system. Those are not the same kind of answer,
which is why "3 of 37" is a fact about a declaration rather than about a product.

**A note on ADR-025.** The desktop's own keys are literal `consume_key` calls — `palette_keys`
(`app.rs:817`, `:833-840`), `pane_keys` (`app.rs:1691`), `layout_keys` (`app.rs:1622`). There is
no keybinding table in the desktop crate at all, and ADR-025 records the consequence: *"The new
bindings are read in `app.rs` alone: they are not in the command palette and not in `/hotkeys`,
because both need a `Command` variant … which the shared `REGISTRY` does not yet exist to host"*
(`docs/design/epsilon-decisions.md:2105-2109`).

---

## 4. The honest count, by name

45 registered spellings = 37 canonical heads (`COMMAND_HEADS`) + 8 aliases (`ALIAS_HEADS`).

### 4.1 Run by the desktop — 3

| Spelling | Arm | What it does |
|---|---|---|
| `/compact` | `commands.rs:117-120` | `self.run_compact()` (`app.rs:2175`) |
| `/goal …` | `commands.rs:121` | `run_goal_argument` (`:152-215`) → `epsilon_runtime::goal_command` (`crates/epsilon-runtime/src/lib.rs:4206`) → `agent.goal_apply` |
| `/sessions` | `commands.rs:124-127` | `set_sidebar_view(SidebarView::Sessions)` (`app.rs:1879`) |

### 4.2 Refused by name — 42 (34 canonical + 8 aliases)

Canonical heads, refused with `"/<name> — not in this window; it works in the terminal"`
(`commands.rs:250-255`):

`model`, `think`, `theme`, `branch`, `tree`, `fork`, `clone`, `new`, `resume`, `editor`,
`session`, `trust`, `reload`, `export`, `import`, `restore`, `share`, `copy`, `name`,
`changelog`, `tasks`, `hotkeys`, `settings`, `scoped_models`, `clear`, `expand`, `skills`,
`skill`, `templates`, `search`, `logout`, `exit`, `help`, `login`

Aliases, refused under the spelling the reader typed (same sentence, different word):

`models`, `timeline`, `auth`, `connect`, `quit`, `q`, `scoped-models`, `?`

**34 + 8 = 42, and 42 + 3 = 45.** The tree asserts the canonical half of that arithmetic itself —
`refused.len() + SUPPORTED.len() == COMMAND_HEADS.len()`, i.e. 34 + 3 = 37 — at `commands.rs:473-477`.
The 8 aliases reach the same catch-all and are driven separately by the alias test at
`commands.rs:550`.

### 4.3 Neither — 0 of 45, and that is worth stating plainly

**No registered spelling is neither run nor refused.** `try_command`'s catch-all (`commands.rs:139`)
means every parsed command reaches one of the two outcomes, and `the_refusal_set_is_the_parser_heads_
the_window_lacks` (`commands.rs:459`) asserts `refused.len() + SUPPORTED.len() == COMMAND_HEADS.len()`.
Any other slash word reaches `Command::Unknown` and is refused with a **third** wording —
`unknown command '/<word>' — nothing here runs it` (`commands.rs:252`), never pointed at the
terminal, because the terminal does not have it either (`:248-249`; test `:636`).

**The real "neither" is on the other side of the vocabulary.** Eight window capabilities have **no
slash head at all** and are therefore neither runnable nor refusable: `view.files`,
`view.subagents`, `view.sessions` (≈ `/sessions`), `tree.reload`, `goal.create`, `goal.pause`,
`goal.resume`, `goal.clear` (⊂ `/goal …`) — `app.rs:761-773`. A reader who knows the terminal
cannot type any of them; a reader of the desktop palette cannot type any of them either.

---

## 5. Cheap to wire vs. real UI work

**Cheap = the action already exists in this crate; only the arm, the `SUPPORTED` name and a test are
missing.**

| # | Head | Action that already exists | Cost | Value / risk |
|---|---|---|---|---|
| 1 | **`tasks`** | `SidebarView::Tasks` + `set_sidebar_view` (`app.rs:1879`); `tasks_pane` (`app.rs:3142-3212`) already draws task state, count and per-task lines via `task_header` (`:3217`) and `task_lines` (`:3238`) | 1 arm + 1 name + 1 test | **Highest.** Subagents are a shipped feature the window cannot reach by name. **Risk:** the terminal's `/tasks` is "state, turns, ids, cost" (`command.rs:69-71`); the pane must actually draw those or the notice must say what it does draw (`AGENTS.md` §2.7) |
| 2 | **`session`** | `model_suffix` (`app.rs:3763`), `run_state` (`:3783`), `figures` (`:2136`), `status_line` (`:3796`), and `IdeSession::session_path` (`session.rs:800`) | 1 arm + 1 name + 1 test | **High, low risk.** The answer already exists as strings the window prints every frame. The only discipline: reuse those strings, do not compute new numbers |
| 3 | **`clear`** | None — `Transcript` has `rows` (`session.rs:378`) and `dropped` (`:383`) but **no** `clear`, and `IdeSession`'s public surface (`session.rs:202-1043`) has none either | ~6 lines + 1 arm + 1 name | **Medium.** "clear transcript (totals kept)" (`command.rs:97-100`). Totals live in `refresh_metrics` (`app.rs:1948`), which reads the agent, not the transcript — so clearing rows does not touch cost. **Check that before believing it** |
| 4 | **`theme`** | `IdeTheme::resolve` (`theme.rs:334`), `ThemeMode::flipped` (`crates/epsilon-tui/src/ui/theme.rs:148`), `IdeApp.theme` copied per frame (`app.rs:3149`), `persist_prefs` (`app.rs:1661`) | ~20 lines, touches the frame | **Medium-high value** — high-contrast is an `AGENTS.md` §8 requirement. **Two risks:** see §7.1 (the description names a palette that does not exist), and `theme.rs:143` `palette` plus `visuals()` (`:349`) are read every frame, so the swap must happen before paint and force a repaint |
| 5 | **`resume`** | `IdeApp::switch_session` (`app.rs:3401`) and `IdeSession::sessions` (`session.rs:795`) both exist and are already wired to the sessions pane | small for the bare form; **medium for the path form** | **Medium.** Bare `/resume` → open the sessions pane, the same honest answer `/sessions` already gives (`commands.rs:126`). Typed path → `switch_session`, which **replaces the live conversation**: `AGENTS.md` §6 requires a confirmation, and `Command::Resume`'s own doc says the live switch is deferred to restart (`command.rs:297`). Take the bare form first |

**Needs real UI work — do not count these as cheap.**

| Head | Why |
|---|---|
| `model` / `models` | No model picker exists, and `IdeSession` has no model-switch method at all (`session.rs:202-1043`); the model is fixed at launch (`lib.rs:84`, `main.rs:47-48`) |
| `expand` | The transcript has no per-row disclosure state. A grep for `expand` over the whole `crates/epsilon-desktop/src` tree returns matches **only** in `workspace.rs` (12 of them: `:50, 62, 86, 127, 133, 141, 144, 154, 163, 192, 200, 328` — every one of them a file-tree folder) and **nothing at all** in `session.rs`, where `Transcript` lives. There is nothing to expand and no redraw path for it |
| `branch` / `tree` / `timeline` | These are **session-branch** navigation. The desktop's tree is a **file** tree (`workspace.rs`, `open_tree_cursor` at `app.rs:1801`). Mapping `/tree` onto it would name the wrong tree |
| `search` | `FileTree::visible` (`workspace.rs:112`) filters the palette's file rows; there is no session-history search anywhere in the crate |
| `new`, `fork`, `clone`, `restore`, `import` | All need session-file or filesystem actions the window does not have. `SUPPORTED`'s doc already calls `/new` out as deliberately unmapped (`commands.rs:361-363`) |
| `help`, `hotkeys` | Would need a reference the window does not build; `/hotkeys` in particular needs the desktop's own key list, which is literals in `app.rs` (ADR-025 `:2105-2109`) |
| `login`, `logout`, `auth`, `connect`, `share`, `export`, `trust`, `editor`, `settings`, `skills`, `skill`, `templates`, `changelog`, `copy`, `think`, `name`, `reload` | No corresponding action in this crate |

---

## 6. Can the declaration be derived from the implementation?

**Yes, and for this crate it is a ~35-line change that deletes more than it adds.**

### The constraint that picks the mechanism

A `macro_rules!` invocation **cannot expand in a match's arm-list** — arms are not a macro
position. So the macro cannot be invoked inside `try_command`'s body. It can be invoked in **item
position**, and it can write the *entire function*, arms included:

```rust
macro_rules! desktop_commands {
    ($( $head:literal => $pat:pat => $body:block ; )*) => {
        impl IdeApp {
            /// Heads this window runs — **derived from the arms below**, not written beside them.
            pub const SUPPORTED: &[&str] = &[ $($head),* ];

            /// Handle `text` if it is a command this window knows, and say what happened.
            pub fn try_command(&mut self, text: &str) -> Outcome {
                let Some(command) = parse_command(text) else {
                    return Outcome::Send(text.to_string());
                };
                match command {
                    $( $pat => $body, )*
                    other => Outcome::Refused(refusal_for(&other, typed_head(text))),
                }
            }
        }
    };
}
desktop_commands! {
    "compact"  => Command::Compact  => { self.run_compact(); Outcome::Ran("compacting context…".into()) };
    "goal"     => Command::Goal(a)  => self.run_goal_argument(a);
    "sessions" => Command::Sessions => { self.set_sidebar_view(crate::app::SidebarView::Sessions);
                                        Outcome::Ran("sessions — click one to resume it".into()) };
}
```

**What it buys, by construction rather than by test:** a name cannot be declared without an arm,
and an arm cannot exist without a name, because they are the same tokens. Adding a fifth verb is
one line and the declaration follows. The `other` arm stays the honest refusal default, so the
"refuse by name" property — the thing this file is actually for — is untouched.

**What it costs:** ~35 lines of macro in `commands.rs`; arm doc comments move inside the macro
(legal); `#![deny(missing_docs)]` (`lib.rs:33`) means the generated `pub const` must carry a doc
comment inside the macro body; and `run_goal_argument` stays a private method, so `try_command`
loses a `pub` it currently has and `tests/window.rs` needs no change (it calls `try_command`,
`app.try_command`, not the helper).

**What it deletes:** `window_runs` (`:317-319`) becomes unused and goes; `SUPPORTED`'s 20-line doc
block (`:344-363`) shrinks to one sentence; and `the_window_runs_exactly_the_heads_it_declares_supported`
(`:614`) has no reason to exist. `refused_heads()` (`:334`) is **already** derived by subtraction
and keeps working unchanged — it becomes, for the first time, a subtraction from the
implementation rather than from a declaration.

### The three alternatives, and why each is worse

| Option | Why not |
|---|---|
| **Move the test's list into production** — `window_runs` becomes the run/refuse split and `try_command`'s catch-all consults it | Derives the *refusal set* from the *list*. The arms and the list are still two hand-written things; a fifth arm with no name is still silently refused. It moves test 1 into the binary and buys nothing |
| **`const RUNNABLE: &[Command]`** | Not const-constructible: `Model(String)`, `Goal(Option<String>)`, `Skill(String)` all carry heap data (`command.rs:244, 316, 281`) |
| **A trait method per command** | Puts the name in a third place (trait, arm, list) unless a blanket object dispatch carries its own name — which is the macro again, with more indirection. Rejected by `AGENTS.md` §5 (minimum) |

### Scope discipline

This closes plan §5's residual **for the desktop's capability axis only**. It does not touch
`parse_command` (kept verbatim by ADR-024 `:2040-2044`), the RPC `get_commands` reply
(`crates/epsilon-cli/src/rpc_mode.rs`, a wire type under `AGENTS.md` §2.8 — ADR-024 `:1988-1994`
records the last widening), or the terminal's `DEFAULTS` and `palette_entries()`. It is strictly
smaller than the flat `REGISTRY` design in `docs/STATUS.md:1428-1435`, and it is the slice that
actually removes the "two statements joined by a test" the plan names.

---

## 7. Defects found while counting

### 7.1 `SLASH_ITEMS` advertises a palette that does not exist

`crates/epsilon-contracts/src/command.rs:213-215` offers `/theme ` with the description
**"switch palette: dark, light, high-contrast"**. `ThemeMode` has **two** variants —
`Dark`, `Light` (`crates/epsilon-tui/src/ui/theme.rs:110-117`) — and `from_name` accepts exactly
those two (`:131-137`). `/theme high-contrast` therefore reaches the refusal at
`crates/epsilon-tui/src/app/commands.rs:1536-1539` and prints *"unknown palette 'high-contrast' —
use `light` or `dark`"*.

A workspace-wide grep for `high-contrast|high_contrast|HighContrast` over `crates/` returns
**exactly one line**: the description string at `command.rs:214`. There is no implementation
anywhere.

This is ADR-024's own failure class in reverse (`docs/design/epsilon-decisions.md:1980-1986`
closed six heads that parsed but were never offered; this is an offered *value* that cannot be
typed). It survives `every_offered_command_is_reachable_from_slash_items` (`command.rs:594`)
because that test compares **heads**, never argument values.

### 7.2 The desktop declares its own goal verbs, in two places

`crates/epsilon-desktop/src/app.rs:570-579` declares
`pub enum GoalCommand { Create, Pause, Resume, Clear }` — a **second** goal-verb table in the tree,
beside `epsilon_runtime::GoalCommand { Show, Help, Apply(GoalOp), Invalid }`
(`crates/epsilon-runtime/src/lib.rs:4172-4181`), whose `GoalOp` already carries all seven verbs
(`command.rs:224-232`). The palette's four are a strict subset of the seven `/goal` reaches.

Two consequences, both inside one crate, ~3,700 lines apart:

- `run_goal_command` (`app.rs:916-969`) and `run_goal_argument` (`commands.rs:152-215`) both call
  `agent.goal_apply` and `refresh_goal` with their own notice wording — `said_for`
  (`app.rs:973-980`) and `describe` (`commands.rs:222-233`) are two past-tense tables for the same
  verbs.
- `epsilon-runtime`'s own doc for `goal_command` states the rule this breaks: *"rules that live in
  a frontend are rules one frontend has"* (`crates/epsilon-runtime/src/lib.rs:4200-4202`).

This is `AGENTS.md` §5 "Reuse before you add", and it is the cheapest real duplication left in the
desktop: one verb list, called from both routes.

### 7.3 Two ADR-024 citations in code point at the wrong lines

Both are the failure mode `AGENTS.md` §5 names — *cite a real `file:line`, not a memory of it*:

| Cited at | Says | Actually there |
|---|---|---|
| `crates/epsilon-contracts/src/command.rs:19-24` | ADR-024 is at `epsilon-decisions.md:1919-1924` | ADR-024's heading is at **`docs/design/epsilon-decisions.md:1969`**; `:1919-1924` is inside ADR-023 |
| `crates/epsilon-desktop/src/commands.rs:74` | ADR-024 is at `epsilon-decisions.md:1874` | `:1874` is inside ADR-022's role list |

`docs/STATUS.md:1449-1452` has the same class of drift: it cites `picker.rs:1061` for the `"/login"`
key-column literal, which is now `crates/epsilon-tui/src/picker.rs:552-556` (the pre-M290 line
number, from before the move).

### 7.4 The plan's "one test" — corrected in §2.2 above

Two tests join the declaration to the implementation, and the second drives production dispatch.
The conclusion is unchanged; the credit is understated.

---

## 8. Summary

- **3 of 37** registry heads (3 of 38 `Command` variants) — the numerator is exact, the
  denominator should be `COMMAND_HEADS`.
- **45 registered spellings: 3 run, 42 refused by name, 0 neither.** The refusal is derived and
  honest; the coverage is not.
- **The declaration is duplicated, not derived.** `SUPPORTED` (`commands.rs:364`) and the three
  arms of `try_command` (`:117`, `:121`, `:124`) are two statements of one fact, joined by two
  tests (`commands.rs:614`, `tests/window.rs:1667`) and by nothing else. `window_runs` steers only
  the exported answer — it has **no production caller**, verified in §2.2.
- **The vocabulary is genuinely shared**; the capability is not. The terminal's dispatch is
  exhaustive by construction (`app/commands.rs:471-519`, catch-all `:517`); the desktop's is a
  three-name const.
- **A seventh table exists and nobody reconciles it** — the desktop palette's 8 rows
  (`app.rs:743-775`), two thirds of which are already in `SUPPORTED` under a different name.
- **Cheapest real wins, in order:** `/tasks`, `/session`, `/clear`, `/theme`, `/resume` (bare form).
- **The mechanism:** a `macro_rules!` in item position that emits `SUPPORTED` and the whole of
  `try_command` from one list. ~35 lines, deletes `window_runs` and the drift test, leaves
  `refused_heads()` derived by subtraction from the implementation. Smaller than ADR-024's
  `REGISTRY` and it closes the exact residual plan §5 names.