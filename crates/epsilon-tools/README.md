# epsilon-tools

## What this crate is

The built-in capabilities: read, write, edit, bash, grep, glob, find, ls, webfetch, todo,
task, the three goal tools and the skill loader. It owns argument validation, the
workspace-boundary refusal and output capping. It does **not** own the permission
decision — it asks `epsilon-safety` — and it does not own which tools a given launch
registers; that is `epsilon-runtime`.

## How it connects

- **Depends on:** `epsilon-contracts`, `epsilon-core`, `epsilon-safety`.
- **Depended on by:** `epsilon-runtime`, `epsilon-server`, `epsilon-cli`.
  `epsilon-tui` depends on it for tests only.

## Entry points

Registration is the surface. `Confinement` (`src/lib.rs:48`) chooses the OS sandbox, then
`register_builtins` (`:87`, the inert default), `register_builtins_with` (`:117`, adds a
diagnostics handle), `register_builtins_with_skills` (`:97`, also registers the `skill`
tool, and registers none when the list is empty), `register_builtins_and_gate_runner`
(`:140`, also hands back the goal-gate runner) and `builtin_registry_and_gate_runner`
(`:222`, the one most callers want).

The tools are re-exported by name: `ReadTool`, `WriteTool`, `EditTool` (`src/files.rs:59`,
`:259`, `:420`), `GrepTool`, `GlobTool`, `LsTool` (`src/search.rs:77`, `:341`, `:536`),
`FindTool` (`src/find.rs:34`), `BashTool` (`src/shell.rs:50`), `WebFetchTool`
(`src/webfetch.rs:36`), `TodoTool`, `TaskTool`, the goal tools, `SkillTool`. `util` holds
`resolve_or_refuse` (`src/util.rs:145`) and `truncate_output` (`:848`); `checkpoint`
stores the pre-edit contents of every changed file (`src/checkpoint.rs:92`).

## How to run and test it

No binary of its own. To use a tool for real, run the terminal front end, which registers
these built-ins through the runtime. Integration tests: `tests/builtin_tools.rs`,
`tests/goal_gates.rs`.

```sh
cargo check -p epsilon-tools
cargo test -p epsilon-tools
```

## Things worth knowing

- Every path-taking tool calls `resolve_or_refuse`, which normalises, checks for escape and
  names a symlink's real target in the refusal (`src/util.rs:145`).
- Output is capped, and oversized output spills to disk with a locator.
  `TOOL_OUTPUT_MAX_BYTES` is 50 KiB for a file listing (`src/util.rs:28`); a single file
  read is capped at 256 KiB (`MAX_READ_BYTES`, `:12`).
- The stale-read guard tracks *coverage*, not a flag: a page of a 5 MB file is not a
  licence to overwrite the whole thing, and a targeted `edit` carries the coverage forward
  rather than promoting it (`src/read_log.rs:24`). It closes the route through the file
  tools; a shell command can still overwrite anything (`:11`). The refusal itself is
  `read_log::verify` (`src/read_log.rs:384`).
- The goal-gate runner must be built from the **same** sandbox the shell tool got. Two
  resolutions would let a transcript name one confinement for a tool call and another for
  the gate that judged it (`src/lib.rs:129`).
- `diagnostics` is a handle, not a policy: whether to start a language server is the front
  end's decision, which is why this crate does not depend on `epsilon-lsp`
  (`src/lib.rs:111`). `Confinement::Auto` resolves to `best_available()`, which fails
  closed — with no backend, commands refuse rather than run unconfined (`:51`).
  `Confinement::Off` records `confinement: "off"` on every result (`:56`).