# opencode

opencode is one of the reference harnesses under `references/`. It is read-only input: this
document records what Epsilon took from it, what it changed, and what it left behind.

Paths into `references/` are written as inline code rather than as links. That directory is
gitignored (`.gitignore`), so a link into it would be dead in a fresh clone. Links in this
document point only at files this repository commits.

## What it is

opencode describes itself as "The open source AI coding agent" — the line under its logo in
`references/opencode-1.18.34/README.md:10`. Its `package.json` is blunter: `"description":
"AI-powered development tool"`.

**Version vendored: 1.18.34.** The root manifest is `"private": true` and carries no version
field; the workspace packages do, and `packages/tui/package.json` reads `"version": "1.18.34"`.
So 1.18.34 is the version of the workspace, not of one published artifact.

Site: <https://opencode.ai>.

**Shape.** A Bun/TypeScript monorepo — 32 directories under `packages/`, with `bun@1.3.14` as the
declared package manager (`package.json:7`). Runtime services are Effect services; durable state
is SQLite through Drizzle, with the schema in `packages/core/src/**/*.sql.ts` and a checked-in
`snapshot.json` per migration under each service's `migrations/` directory.

**The terminal UI is built on OpenTUI.** OpenTUI is a separate library, also vendored here:
"OpenTUI is a library to build terminal user interfaces. It is written in Zig. You write
TypeScript directly or through React and Solid. You arrange boxes and text with flexbox. You
add selects, inputs, and scroll boxes with keyboard and mouse controls."
(`references/opentui-0.5.14/README.md`).

opencode depends on three of its packages, pinned to `0.4.5` in the root dependency catalog
(`package.json:43-45`): `@opentui/core`, `@opentui/keymap` and `@opentui/solid`. How it uses
them is readable in a few places:

- The renderer is one call. `createCliRenderer({...})` from `@opentui/core` is made once at
  startup and released by a finalizer (`packages/tui/src/app.tsx:191-213`). The options are
  all in one object: 60 fps target, the Kitty keyboard protocol, `exitOnCtrlC: false`, mouse
  optional.
- Components are Solid JSX. Files that need the pragma declare
  `/** @jsxImportSource @opentui/solid */` at the top; the rest inherit it.
- The drawable objects — `BoxRenderable`, `ScrollBoxRenderable`, `TextareaRenderable`, `RGBA`,
  `TextAttributes` — come from `@opentui/core`, so layout is flexbox rather than a manual
  cell grid.
- Key binding is separate from command routing. The app builds a default keymap and then
  layers the project's on top: `createDefaultOpenTuiKeymap(renderer)` followed by
  `registerOpencodeKeymap(keymap, renderer, config)` (`packages/tui/src/app.tsx:215-217`). A
  binding is therefore a row in a table, not a branch in a chain.

So opencode's terminal front end is TypeScript driving a Zig renderer across an FFI boundary;
`references/opentui-0.5.14/AGENTS.md` is a document about keeping that boundary portable.
There is no second drawing path.

**Distribution.** npm `opencode-ai`, a `curl | bash` installer, Scoop, Chocolatey, Homebrew,
pacman, AUR, mise and Nix, plus a beta desktop app per platform (`README.md:46-83`).

## Who made it, and under what licence

- **Licence: MIT.** `LICENSE` reads "MIT License" and "Copyright (c) 2025 opencode".
  `package.json:124` agrees: `"license": "MIT"`.
- **Repository: <https://github.com/anomalyco/opencode>**, taken from the `repository.url`
  field in `package.json:120-123`. The README's build-status badge points at the same place.
- **Organisation:** the copyright line names `opencode`, and the repository owner segment is
  `anomalyco`. The vendored copy names **no individual authors** — not in `LICENSE`, not in
  `package.json`, not in `CONTRIBUTING.md`. Individual authorship is *not stated in the vendored
  copy*, and is not guessed here.
- **Project site:** <https://opencode.ai>.

## Its strengths, in its own terms

These are opencode's claims about itself, quoted or closely paraphrased from its own README and
its own documentation under `packages/web/src/content/docs/`.

**A permission model that resolves to three words.** "OpenCode uses the `permission` config to
decide whether a given action should run automatically, prompt you, or be blocked"
(`permissions.mdx:6`). Each rule resolves to one of `"allow"`, `"ask"` or `"deny"`
(`permissions.mdx:14-18`) — and an action with no matching rule is an ask, not an allow
(`packages/core/src/permission.ts:76-86`).

**Last match wins, with the catch-all written first.** "Rules are evaluated by pattern match,
with the **last matching rule winning**. A common pattern is to put the catch-all `"*"` rule
first, and more specific rules after it" (`permissions.mdx:91`). The patterns are small:
`*` matches zero or more characters, `?` matches exactly one, everything else is literal
(`permissions.mdx:95-99`).

**The tool proposes the approval pattern, not the user.** When approval is needed, the UI
offers `once`, `always` and `reject`, and "the set of patterns that `always` would approve is
provided by the tool (for example bash approvals typically whitelist a safe command prefix like
`git status*`)" (`permissions.mdx:193-199`). The user is confirming a proposed rule rather than
writing one.

**Two agents, one keystroke.** "OpenCode includes two built-in agents you can switch between
with the `Tab` key" — `build`, the "Default, full-access agent for development work", and
`plan`, a "Read-only agent for analysis and code exploration" that "Denies file edits by default"
and "Asks permission before running bash commands" (`README.md:100-113`). A read-only mode is
part of the product, not a flag.

**A named brake for the loop that is going in circles.** `doom_loop` is a permission of its own,
"triggered when the same tool call repeats 3 times with identical input" (`permissions.mdx:164`).

**Language servers as edit feedback, off by default.** "OpenCode can integrate with Language
Server Protocol (LSP) servers to use diagnostics as feedback for the agent"
(`lsp.mdx:6`). Thirty-four servers ship in the table (`lsp.mdx:12-49`), and "LSP is disabled by
default" (`lsp.mdx:51`). The block rides on the edit's own result text: after `Edit applied
successfully.`, opencode appends "LSP errors detected in this file, please fix:" plus a rendered
list (`packages/opencode/src/tool/edit.ts:196-201`). That renderer filters to severity 1 and
caps at twenty per file (`packages/opencode/src/lsp/diagnostic.ts:3,20-27`).

**And opencode argues against its own feature, in its own docs.** "LSP can help the agent find
and fix issues by providing diagnostics from language servers. This is useful in some projects,
but it is not always a net positive. Language servers can get out of sync, use significant
memory, vary by version or project, and slow down agent workflows" — the advice is to let the
agent run lint and typecheck directly and record those commands in `AGENTS.md`
(`lsp.mdx:70-72`). A project that documents the cost of its own headline feature is easier to
read than one that does not.

**Every change is reversible inside the session.** "OpenCode uses snapshots to track file changes
during agent operations, enabling you to undo and revert changes within a session. Snapshots are
enabled by default" (`config.mdx:631`) — captured as a content-addressed tree
(`packages/core/src/snapshot.ts:43-49`) over an internal git repository, with the cost stated
next to the feature: slow indexing and significant disk usage on large repositories
(`config.mdx:633`).

**A vocabulary kept under control.** `CONTEXT.md` opens with a glossary that defines terms and
then says what to avoid — "System Context … _Avoid_: System prompt", "Session History … _Avoid_:
Session Context", "Context Source" — and a relationships section that states what composes with
what. This is the single most useful document in the vendored copy for a reader trying to
understand what a code path means, because it settles arguments before they start.

## What we adopted

Taken as documented behaviour and reimplemented in Rust. Each item names where it lives in
**this** repository, confirmed against the source, and where the reference said it.

**The rule shape: capability × pattern → action.** `Rule` is a flat struct of `capability`,
`pattern` and `action`, matched against a capability name and the call's subject
(`crates/epsilon-safety/src/engine.rs:19-30,42-46`). `crates/epsilon-safety/src/lib.rs:3-6`
names the source: "Semantics ported from opencode: flat rules of `{capability, pattern,
action}`, evaluated **last-match-wins** across stacked layers".

**Three actions, and an unknown capability asks.** `Action::{Allow, Ask, Deny}`
(`crates/epsilon-safety/src/engine.rs:8-17`); `evaluate` returns `Action::Ask` when nothing
matched (`crates/epsilon-safety/src/engine.rs:97`), with a test pinning it
(`crates/epsilon-safety/src/engine.rs:179-185`). This is the same fallback opencode uses
(`packages/core/src/permission.ts:76-86`).

**The ask flow, with session memory.** `ApprovalUi` is the trait a front end implements
(`crates/epsilon-safety/src/gate.rs:12-17`); `RulesetGate` holds a list of promoted
`(capability, pattern)` pairs and documents itself as "the opencode-style flow: session memory →
engine evaluation → (`ask`) approval UI → optional memory promotion"
(`crates/epsilon-safety/src/gate.rs:46-50`). Wired into composition at
`crates/epsilon-runtime/src/lib.rs:4555`.

**Wildcard semantics.** `glob_match` is the matcher the engine calls
(`crates/epsilon-safety/src/engine.rs:5,44`), and `crates/epsilon-safety/src/wildcards.rs:95-103`
carries spot checks taken from opencode's `wildcard.ts` cases, including case sensitivity on
Windows.

**Plan mode as a read-only ruleset.** `crates/epsilon-safety/src/engine.rs:129-149` is
`plan_ruleset()`, documented as "opencode's `plan` agent pattern". The CLI flag is
`crates/epsilon-cli/src/main.rs:193`.

**The doom-loop brake at three.** `crates/epsilon-core/src/lib.rs:206` — "Consecutive identical
calls before the doom-loop gate asks (opencode: 3)" — with the streak state living on the
run-scoped `Agent` (`crates/epsilon-core/src/exec.rs:559`).

**Language-server diagnostics, riding on the edit's own output.** The bounded block lives in
`crates/epsilon-lsp/src/report.rs:1-7`, the twenty-per-file cap is
`crates/epsilon-lsp/src/report.rs:60`, and the errors-only filter is
`crates/epsilon-lsp/src/report.rs:66-70`. The edit and write tools append it to their own result
text through `diagnostics_note` (`crates/epsilon-tools/src/files.rs:18-38`, called at
`crates/epsilon-tools/src/files.rs:372,562,696`). The provider is constructed in composition and
handed to the two tools (`crates/epsilon-runtime/src/lib.rs:70-78`,
`crates/epsilon-tools/src/lib.rs:152-153`).

**One server per (project root, server), keyed by extension.** `crates/epsilon-lsp/src/service.rs:3-5`
names the source and the three behaviours taken: keying by extension and nearest project root,
deduplicating in-flight spawns, and a `broken` set so a dead server is not retried for ever.

**A diff without a new dependency.** `crates/epsilon-tools/src/util.rs:36` says so in the
heading — "M30: lightweight unified diff (no crate dependency — opencode does the same)" — and
`unified_diff` is at `crates/epsilon-tools/src/util.rs:44`. The edit tool returns it as part of
its result (`crates/epsilon-tools/src/files.rs:549,683`), so the transcript shows what changed.

**An `auth` command that reads as a credential flow.** `crates/epsilon-llm/src/auth.rs:1-8`
names the pattern, and `crates/epsilon-cli/src/main.rs:226,240,627` is
`epsilon auth list|login|logout`.

## What we adapted

Taken, then changed on purpose. The difference from the section above is that the semantics
moved.

**Deny is monotonic here; in opencode it is not.** opencode is last-match-wins throughout
(`permissions.mdx:91`), so a later `allow` re-allows an earlier `deny`. Epsilon returns
immediately on any deny, regardless of position: "**any deny wins immediately** regardless of
position. A project cannot accidentally re-allow something the global config denied"
(`crates/epsilon-safety/src/engine.rs:59-62`, implemented at
`crates/epsilon-safety/src/engine.rs:90-93`, tested at
`crates/epsilon-safety/src/engine.rs:163-177`). A project layer is exactly the kind of file a
user edits casually, so order-independence is the safer reading of the same idea.

**Reading a `.env` asks here; opencode denies it.** opencode's default is `"read": {"*": "allow",
"*.env": "deny", "*.env.*": "deny", "*.env.example": "allow"}` (`permissions.mdx:174-187`).
Epsilon's defaults use `Ask` for the same two patterns and keep `*.env.example` allowed
(`crates/epsilon-safety/src/engine.rs:113-117`). A denial is not reversible by the person at the
keyboard; an ask is. The trade is that Epsilon will prompt more often than opencode, which is
the side to err on for a rule that decides whether a file can be inspected at all.

**Plan mode denies the shell rather than asking for it.** opencode's `plan` agent "Asks
permission before running bash commands" (`README.md:107`). Epsilon refuses
(`crates/epsilon-safety/src/engine.rs:143-146`), and says why in its own test: "Even apparently
read-only commands are refused: there is no portable read-only shell boundary inferred from
command strings" (`crates/epsilon-safety/src/engine.rs:229-231`). Classification of a command
string is guesswork; a mode whose whole promise is "this run cannot touch the tree" should not
rest on a guess.

**The server table is user-installed only.** opencode carries per-server install commands and
auto-installs several of them (`lsp.mdx:12-49`). Epsilon does not: "a coding agent that fetches
and executes binaries is a supply chain, and the failure mode is silent for the user"
(`crates/epsilon-lsp/src/server.rs:3-6`). Relatedly, Epsilon answers one question — what the
compiler says about the file just written — rather than shipping opencode's nine navigation
operations (`crates/epsilon-lsp/src/server.rs:6-8`).

**Diagnostics degrade to nothing.** Epsilon makes the absence of a block a named, ordinary
outcome with three triggers — no provider, a file that cannot be re-read, or a provider that is
silent, broken or wrong — and the rule is that none of them may fail an edit that already
happened (`crates/epsilon-tools/src/files.rs:19-23`). The crate header states it as the design:
"Degradation is silent by design. A machine without `rust-analyzer` gets exactly today's
behaviour: the edit succeeds, and nothing is said" (`crates/epsilon-lsp/src/lib.rs:20-22`).

**A redo rail on the editor.** pi's undo stack plus an explicit redo rail from opencode's
`input_redo`, with Ctrl+Z popping undo and Ctrl+. re-applying
(`crates/epsilon-tui/src/undo.rs:1-3`, re-apply bound at
`crates/epsilon-tui/src/app/editing.rs:201`).

**The edit-match guard, with the thresholds kept and the ordering changed.** The span rule is
opencode's — "a span of `max(old+3, old*2)` lines or +500 characters"
(`crates/epsilon-tools/src/edit_match.rs:341`) — but Epsilon orders replacer candidates by rising
risk rather than by observed frequency, and says why in the ADR
(`crates/epsilon-tools/src/edit_match.rs:27`, `docs/design/epsilon-decisions.md:1545-1549`).

## What we skipped, and why

**OpenTUI as the renderer.** Epsilon's terminal front end is ratatui over crossterm, chosen in
ADR-011 ([`docs/design/epsilon-decisions.md`](../../docs/design/epsilon-decisions.md), "ADR-011:
Frontends — ratatui TUI default"). OpenTUI is Zig reached from TypeScript through
`node:ffi`/`bun:ffi`; a Rust workspace with `unsafe_code = "forbid"` and no Node runtime in the
product would have to add a foreign-function layer, a second language toolchain and a runtime
dependency to obtain a layout engine ratatui already provides. The *idea* was kept — key binding
separated from command routing — and lives in `crates/epsilon-tui/src/app/keys.rs:1-13` and the
one-command-registry invariant in [`AGENTS.md`](../../AGENTS.md). [`CREDITS.md`](../../CREDITS.md)
points the full reasoning at `docs/references/TUI-DECISION.md`; that file is **not in the tree
yet**, so this paragraph is the record until it is written.

**The Solid web app, the console and the stats site.** opencode ships `packages/app` (a Solid
front end), `packages/console`, `packages/web` and `packages/stats`. Epsilon's second front end
is a native egui/eframe window and its third is a small HTTP/SSE server
(`crates/epsilon-server/`). Same goal — one core behind several front ends — different carrier.

**The Electron desktop app.** `README.md:67-83` lists the `.dmg`, `.exe`, `.deb`, `.rpm` and
`.AppImage` builds. ADR-020 rules it out on purpose: "That rules out Electron, and rules out
Tauri-with-an-HTML-frontend" ([`docs/design/epsilon-decisions.md`](../../docs/design/epsilon-decisions.md),
"ADR-020"). That is a difference in what each project is for.

**SQLite and Drizzle for session storage.** opencode's schema is `packages/core/src/**/*.sql.ts`
with a migration per change. Epsilon's session store is append-only JSONL, one file per session,
which is ADR-006 and the property its resume and repair paths actually depend on: a session file
can be read, diffed, repaired by hand, and salvaged. A database gives a different set of
guarantees, and none of them is the one Epsilon's `--resume` is built on.

**The `lsp` navigation tool.** `packages/opencode/src/tool/lsp.txt:3-12` lists nine operations —
`goToDefinition`, `findReferences`, `hover`, `documentSymbol`, `workspaceSymbol`,
`goToImplementation`, and three call-hierarchy operations. No tool in this tree makes them
(`crates/epsilon-lsp/src/server.rs:6-8`). Answering "where is this symbol defined" needs a
workspace-wide index Epsilon does not keep, and a navigation surface nothing asks for is a
second thing to keep honest.

**Auto mode.** `--auto` approves anything not explicitly denied (`permissions.mdx:24-38`).
Epsilon's unattended path is `AutoDeny` (`crates/epsilon-safety/src/gate.rs:29-44`), which
returns a refusal carrying the reason: "unattended run: `{capability}` on `{pattern}` requires
an interactive session or `--yes`". A headless run that self-approves cannot be told apart from
one that was authorised.

**The legacy `tools` boolean permission config.** Deprecated in opencode as of v1.1.1 and merged
into `permission` (`permissions.mdx:8`). There is no reason to port a shape its own maintainers
are retiring.

## Licence compatibility

**Compatible. opencode is MIT, Epsilon is MIT, and adaptation with attribution is permitted.**

- opencode: MIT, "Copyright (c) 2025 opencode" (`LICENSE`; `package.json:124`).
- Epsilon: MIT, "Copyright (c) 2026 Mohamed" ([`LICENSE`](../../LICENSE)).
- The MIT permission notice requires one thing of anyone who copies: "The above copyright notice
  and this permission notice shall be included in all copies or substantial portions of the
  Software."

Concretely, that would mean two obligations in this repository: a `THIRD_PARTY_NOTICES` file
carrying opencode's notice, and a header on every adapted file naming its source — which is what
[`AGENTS.md`](../../AGENTS.md) §10 requires of an adaptation.

**What actually happened is different, and the difference is deliberate.** No third-party source
code was copied into this repository; every item above is a reimplementation from opencode's own
written behaviour. [`CREDITS.md`](../../CREDITS.md) says so in as many words — "**no third-party
source code was copied into this repository**: what was taken is behaviour, structure and design
decisions" — and that is why there is no `THIRD_PARTY_NOTICES` file. The cost of that choice is
that a citation has to be checked by hand, so every item above carries the reference file and
line where the behaviour is stated, and the port sites in `crates/` carry the same citation in
their own doc comments.

The name, logo and brand of opencode belong to its authors and are not used anywhere in this
repository.