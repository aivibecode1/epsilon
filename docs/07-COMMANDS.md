# Command reference

Every slash command Epsilon offers, with the spelling, the argument shape, and what
actually happens. This is the detail behind the one-line rows in the `/` menu and the
command palette (`Ctrl+P`): the menu tells you a command exists, this page tells you
what it will do.

The command list is defined once in
`crates/epsilon-contracts/src/command.rs` (`COMMAND_HEADS` + `SLASH_ITEMS`). `/help`
prints the grouped index and key hints; the popup and palette draw the same rows with
the descriptions below.

## How a command is read

- A line beginning with `/` is always a command attempt. The head is everything before
  the first space, matched case-insensitively; the rest is the tail, trimmed.
- An unknown head is refused by name. It is never sent to the model as literal text.
- A no-argument command ignores trailing words: `/clear now` still clears.
- Prompt templates and custom `commands/` files are expanded before parsing. A template
  cannot shadow a builtin; the builtin wins.
- Commands run in the **terminal IDE**. Headless `-p`, HTTP `POST /prompt` and the RPC
  `prompt` op send their text to the model verbatim. RPC advertises the list through
  `get_commands`, but it does not execute a slash command.

Two commands ask before acting (`/import`, `/plan off`), two use a typed `yes`
(`/restore`, `/undo-turn`), and `danger-full-access` asks once. The table says which.

## Start, navigate, and end

| Command | Usage | Aliases | What it does |
|---|---|---|---|
| `/help` | `/help` | `?` | Print the grouped command index, key hints, and custom commands. |
| `/hotkeys` | `/hotkeys` | — | Print the main shortcuts and every effective keybinding, including unbound ids. |
| `/exit` | `/exit` | `quit`, `q` | End the session and return to the shell. Refused while a run is in flight; `Esc` aborts first. `Ctrl+Q` does the same. |
| `/clear` | `/clear` | — | Clear the transcript pane. The session file and token/cost totals are kept; `Ctrl+C` clears the draft. |
| `/new` | `/new` | — | Start a new session file and switch to it. The previous file is kept. Refused while a run is in flight. |
| `/resume` | `/resume [path]` | — | Bare opens the session browser; a path live-switches to that session file. |
| `/sessions` | `/sessions` | — | Browse sessions for this workspace, newest first. `Enter` resumes, `Tab` switches workspace/all, `Ctrl+N` starts a new one, `Ctrl+D` then `Enter` deletes (asks first; the active session cannot be deleted). |
| `/import` | `/import <path>` | — | Ask `yes`/`no`, then live-switch to an existing Epsilon session file. This does **not** convert a pi session — use the CLI `epsilon --import <pi.jsonl>` for that. |
| `/branch` | `/branch` | `timeline`, `tree` | Open the session tree. `Enter` moves the leaf and reprojects the transcript; `k` forks, `L` labels, `/` searches, `f` filters. A jump changes what the next turn sees. |
| `/fork` | `/fork [text]` | — | Open a picker over user messages and fork at the chosen one into a new file. The optional `text` is put back in the composer afterwards. |
| `/clone` | `/clone` | — | Copy the active branch from the current leaf into a new session file. Abandoned branches are not copied. |
| `/name` | `/name [text]` | — | Bare shows the current name; text sets it and writes a `session_name` entry. |
| `/session` | `/session` | — | Show the file, leaf id, entry count, token totals, model, cache attribution, and repair lines. |
| `/changelog` | `/changelog` | — | Print the first 120 lines of the workspace `CHANGELOG.md`, or `docs/CHANGELOG.md`. |
| `/settings` | `/settings` | — | Open the six-row settings panel: model, thinking, theme, thinking blocks, permission preset, plan. |
| `/reload` | `/reload` | — | Re-check project trust, re-discover skills, prompt templates, custom commands and agent definitions, and re-read keybindings. It does **not** re-read model, theme or permission values. |
| `/trust` | `/trust [yes\|no]` | — | Bare reports whether the project config hash is trusted; `yes` accepts it, `no` leaves it untrusted. |

Examples: `/resume .epsilon/sessions/--my-project--/2026-10-18T09-00-00-000Z_ses_ab12.jsonl`,
`/fork try the other parser`, `/name parser-cleanup`.

## Models, context, and the screen

| Command | Usage | Aliases | What it does |
|---|---|---|---|
| `/models` | `/models [id]` | `model` | Bare opens the model picker. With an id, behaves exactly like `/model <id>`. |
| `/model` | `/model [id]` | `models` | Bare opens the picker; an id applies a unique match (exact, `provider/id`, or a catalog label), otherwise opens the picker pre-filled. The choice is persisted. |
| `/think` | `/think [level]` | — | Bare opens the level picker; a level sets it. Values: `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` (case-insensitive). Persisted project-first. |
| `/compact` | `/compact` | — | Summarize older context in a background task, exactly like the pressure path. Costs one provider call. Refuses while a turn or compaction is already running. |
| `/catalog` | `/catalog` | — | Print the bundled catalog snapshot: date, age, size, providers, and live-listing status. |
| `/scoped_models` | `/scoped_models` | `scoped-models` | Open the checklist that scopes `F2` / `Shift+F2` model cycling. `Space`/`Enter` toggles, `Ctrl+S` saves, `Esc` reverts. Staging is memory-only until saved. |
| `/theme` | `/theme [palette]` | — | Bare toggles dark/light; a name sets the palette for this run. Values: `dark`, `light`, `high-contrast` (also `contrast`, `hc`). Does not write config — the settings theme row persists. |
| `/expand` | `/expand` | — | Toggle global tool-output expansion (same as `Ctrl+X`). Not persisted. |
| `/search` | `/search <text>` | — | Search the **rendered transcript** and jump to the first match. This is not a search of the session file. `n`/`Enter`/`Down` next, `p`/`Up`/`PgUp` previous, `q`/`Esc` close. |
| `/copy` | `/copy` | — | Copy the most recent assistant transcript line to the clipboard (OSC 52 when no native clipboard is available). |
| `/share` | `/share` | — | Write an HTML export, then create a **secret** GitHub gist with `gh gist create --public=false`. Needs `gh` installed and authenticated; otherwise it prints the command. |
| `/export` | `/export [format] [path]` | — | Write the session as HTML (default), pretty JSON, or JSONL. `html`, `json`, `jsonl` are the format words; a path ending `.json`/`.jsonl` infers the format. Relative paths resolve inside the workspace. |
| `/tasks` | `/tasks` | — | List delegated subagent tasks: id, state, model, tokens, description, turns, output preview. |
| `/skills` | `/skills` | — | Open the skills browser: built-ins first, then discovered skills, with `[on]`/`[off]` state. `up`/`down` select; the detail block shows the selected skill's description, why it is useful, its scope, its credit where adapted, and the exact `/skill` command that changes it. `enter` puts that command on the prompt; `esc` closes. |
| `/skill` | `/skill <name> on\|off` | — | Switch one skill. Enabling a built-in writes its `SKILL.md` under the config directory; disabling removes a pristine copy (an edited copy is left alone and reported). A discovered skill is toggled through the global disable list. `/skill:<name> off` is the same command; `enable`/`disable` are accepted. Takes effect next turn. |
| `/subagent` | `/subagent [<name> on\|off]` | `subagents` | Bare opens the subagent browser for the four built-in roles (`explore`, `review`, `verify`, `implement`) with the same select/detail layout. `<name> on` installs that role's agent definition under the config directory; `off` removes a pristine copy. Enabled roles become `task` subagent types on the next turn. |
| `/templates` | `/templates` | — | List loaded prompt templates and custom commands with descriptions and argument hints. |

Examples: `/model deepseek-chat`, `/think high`, `/theme high-contrast`,
`/export json notes.json`, `/search TODO`.

`/export` creates parent directories and overwrites the target without a second
confirmation, so choose the path deliberately. A bare `/export` writes
`.epsilon/exports/session-<id>.<ext>`.

## Goals, plan mode, and permissions

| Command | Usage | Aliases | What it does |
|---|---|---|---|
| `/goal` | `/goal [objective]` or a verb | — | Bare shows the goal; a sentence creates/replaces it and starts a round when idle. Verbs: `edit <objective>`, `edit rounds <n>`, `edit gate <command>`, `edit gate clear`, `pause`, `resume`, `complete`, `blocked <code> <reason>`, `clear`, `help`. Creating or resuming can immediately spend tokens. |
| `/plan` | `/plan [on\|off]` | — | Bare reports the live gate. `on` denies writes, edits and shell immediately and saves. `off` asks `yes`/`y`, then re-allows them. Plan mode is a capability allowlist, not only four tool names, so an MCP or extension write is denied rather than offered. |
| `/preset` | `/preset [name]` | — | Bare lists the four presets with the current one marked. Values: `auto`, `workspace-write`, `read-only`, `danger-full-access`. `danger-full-access` asks `yes`/`y` and applies to this run only (never saved); the others persist project-first. |

A `--sandbox read-only` launch also forces the session permission preset to
`read-only`: the shell flag is not narrower than the tool gate, so `/plan` and the
preset agree about what the session may do.

## Files and history

| Command | Usage | Aliases | What it does |
|---|---|---|---|
| `/changes` | `/changes [all]` | — | Open the changed-files panel from checkpoints. Empty scope is this session; `all` is the workspace. `Enter` previews, `d`/`Tab` diffs, `r` describes, `y` restores the described file, `q`/`Esc` closes. |
| `/restore` | `/restore <path> [yes]` | — | Preview the checkpoint for one file. Only a trailing `yes` applies it; preview changes nothing. Restoring a file that the checkpoint says was created removes it, if it is still unchanged. |
| `/undo-turn` | `/undo-turn [N] [yes]` | — | Bare lists anchored turns; `N` previews; `N yes` restores every file that turn changed and moves the transcript leaf. There is no redo, and tokens, approvals, goals and shell history are not rewound. |

`/restore` and `/undo-turn` refuse while a run is in flight. A restore refuses when the
file changed since the checkpoint, so it cannot silently discard later edits.

## Providers and the editor

| Command | Usage | Aliases | What it does |
|---|---|---|---|
| `/login` | `/login [provider [key\|oauth]]` | `auth`, `connect` | Bare opens the provider picker. A provider alone opens a masked key prompt. `provider key` saves immediately. `provider oauth` (or `--oauth`) starts the subscription flow. |
| `/logout` | `/logout <provider>` | — | Remove the stored key for one provider. |
| `/editor` | `/editor` | — | Suspend the TUI, edit the current draft in `$EDITOR` (or `[editor] command`, then `$VISUAL`), and read it back. `Alt+E` does the same. |

Credentials are stored in `~/.config/epsilon/auth.json`; see
[authentication](user/auth.md) for the file's permissions and the environment-variable
alternative.

## Aliases and hidden spellings

Aliases are parser inputs, not extra menu rows. They work when typed and are listed
here so nothing is a secret.

| Alias | Canonical command |
|---|---|
| `/models` | `/model` (and it has its own offered row) |
| `/timeline`, `/tree` | `/branch` |
| `/auth`, `/connect` | `/login` |
| `/quit`, `/q` | `/exit` |
| `/scoped-models` | `/scoped_models` |
| `/?` | `/help` |
| `/skill:<name> [on\|off]` | `/skill <name> [on\|off]` |
| `/subagents` | `/subagent` |

`/tree` is the **session** tree (`/branch`). The workspace file tree has no command; it
is the `Files` palette row and the `F6` key.

## Keys that mirror a command

| Key | Action |
|---|---|
| `Ctrl+P` | Open the command palette |
| `F6` | Show/focus or hide the workspace file tree |
| `Ctrl+M` | Model picker (`/model` bare) |
| `Ctrl+T` | Cycle thinking level (`/think`) |
| `Ctrl+X` | Expand/collapse tool output (`/expand`) |
| `Alt+E` | External editor (`/editor`) |
| `F4` | Copy the last assistant message (`/copy`) |
| `Ctrl+N` | New session (`/new`) |
| `Ctrl+Q` | Quit (`/exit`) |
| `F2` / `Shift+F2` | Cycle scoped models forward/back |
| `Ctrl+G` | Refresh the live model listing (model picker) |

The palette has exactly one row without a slash command: **Files · workspace tree**,
which runs the same `F6` action. `/tree` is a different feature (the session tree);
offering it as "Files" would run the wrong thing.

## Where to go next

- [User guide](05-USER-GUIDE.md) — task map for install, auth, models and permissions.
- [Sessions](user/sessions.md) — files, resume, fork, export and repair.
- [Goals](user/goals.md) — rounds, gates and blocked states.
- [Permissions](user/permissions.md) — rules, presets, plan mode and sandboxing.
