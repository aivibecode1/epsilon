# pi

Epsilon began as a port of **pi**. This page says what pi is, what pi says about
itself, and what Epsilon took from it. Every claim about Epsilon carries a
`file:line` in this repository, so a reader can open it.

The vendored copy is under [`../../references/`](../../references/) at
`references/pi-1.0.0/`. It is read-only input: not part of the build, not committed.
Paths that start `packages/` are relative to that directory.

## What it is

pi is the **Pi Agent Harness**, a TypeScript monorepo. Its README calls it "the home
of the Pi agent harness project including our self extensible coding agent"
(`references/pi-1.0.0/README.md:15`). The npm workspaces under `packages/` hold the
pieces: `@earendil-works/pi-ai` is a unified multi-provider LLM API,
`@earendil-works/pi-agent-core` is an "agent runtime with tool calling and state
management", `@earendil-works/pi-coding-agent` is the interactive CLI, and
`@earendil-works/pi-tui` is a terminal UI library with differential rendering
(`references/pi-1.0.0/README.md:30-36`).

**Version as vendored here: 1.0.0**, read from
`references/pi-1.0.0/packages/coding-agent/package.json:3`. That is the published CLI
package — the one `npx` installs and the one whose `bin` entry is `pi`
(`references/pi-1.0.0/packages/coding-agent/package.json:10`). The monorepo root is a
private workspace container and carries its own number,
`references/pi-1.0.0/package.json:2,69`, which is not the release number.

A **port** means reimplementing pi's behaviour in another language rather than
shipping its code. Epsilon is an *incomplete* Rust port: the loop, the queues, the
tools, session storage and prompt assembly are there; several of pi's interfaces are
not. "What we skipped" says which.

Website: <https://pi.dev>. Documentation: <https://pi.dev/docs/latest>
(`references/pi-1.0.0/README.md:23-24`).

## Who made it, and under what licence

- **Author.** `references/pi-1.0.0/LICENSE:3` reads "Copyright (c) 2025 Mario Zechner".
- **Organisation.** **earendil-works** — the npm scope on every published package, and
  the name `references/pi-1.0.0/CONTRIBUTING.md:101` uses when it describes how larger
  changes get discussed.
- **Repository.** <https://github.com/earendil-works/pi>, stated in
  `references/pi-1.0.0/packages/coding-agent/docs/session-format.md:34-35`.
- **Licence.** MIT. Stated at `references/pi-1.0.0/README.md:108-110`; the full text is
  `references/pi-1.0.0/LICENSE:1-21`.
- **Not stated in the vendored copy:** a legal entity behind the npm scope, a
  governance model, or a maintainer list. The README carries no author line, so
  "earendil-works" rests on the package scope and one CONTRIBUTING sentence rather than
  on a statement of ownership.

## Its strengths, in its own terms

**A core that is small on purpose.** `references/pi-1.0.0/CONTRIBUTING.md:7-11`: "First
things first: pi's core is minimal. If your feature does not belong in the core, it
should be an extension." Extensions are not a plugin bolt-on here — they are TypeScript
modules loaded into the pi process whose factory functions "register tools, commands,
shortcuts, providers, event handlers, renderers, and terminal UI"
(`references/pi-1.0.0/packages/coding-agent/docs/how-pi-works.md:43`). A tool provider
can change the core, not only extend it.

**One agent behind every interface.** Interactive mode, print mode, JSON mode, RPC mode
and the TypeScript SDK are five surfaces over the same machinery, and pi says so
directly: "All interfaces use the same agent and session mechanisms"
(`references/pi-1.0.0/packages/coding-agent/docs/how-pi-works.md:35-39`).

**The loop is described as two things, not one.** A turn is a streamed assistant
response plus the tool calls it made; another turn starts "if tool results or queued
messages require another model request"
(`references/pi-1.0.0/packages/coding-agent/docs/how-pi-works.md:11`). Queued input has
two kinds and two insertion points — "Steering messages enter after the current
assistant turn. Follow-up messages enter after the agent has finished its pending
work" (same file, line 13). **Steering** means "interrupt me mid-run"; a **follow-up**
means "run this after you would otherwise stop". `packages/agent/README.md:422` adds the
priority rule: follow-ups are checked only when there are no tool calls and no steering
left.

**A conversation is a tree, not a list.** "Messages and events in a session form a tree.
Each path through that tree is a branch. The branch ending at the current entry is the
active branch and supplies the history for the next model request"
(`references/pi-1.0.0/packages/coding-agent/docs/how-pi-works.md:5`). The storage is
JSONL with `id`/`parentId`, so branching happens inside one file rather than by copying
it (`references/pi-1.0.0/packages/coding-agent/docs/session-format.md:3`). The format is
documented entry by entry, including what a compaction boundary means
(same file, lines 123-137).

**Hooks are contracts with an order.** `packages/agent/README.md:124-167` writes down
`prepareRequest` and `finishTurn`, when each runs, what each may do, and the exact
per-turn lifecycle. That is unusually specific for an extension point, and it is what
made the seams portable: the port could follow the order instead of guessing it.

**Parallel tool execution with a stable persisted order.**
`packages/agent/README.md:114`: "In parallel mode, tool completion events follow tool
completion order, but persisted toolResult messages still follow assistant source
order." The UI can react the moment a tool finishes; the transcript stays replayable.

**Stopping is a batch decision, not a tool decision.** A tool may return
`terminate: true`, but "the loop only stops early when every finalized tool result in
that batch sets `terminate: true`. Mixed batches continue normally"
(`packages/agent/README.md:120`).

**Supply chain is treated as reviewed code.** `references/pi-1.0.0/README.md:78-90`:
direct dependencies pinned to exact versions, `.npmrc` sets `save-exact=true` and
`min-release-age=2`, `package-lock.json` is "the dependency ground truth", pre-commit
blocks accidental lockfile commits, and the published package carries a generated
shrinkwrap so transitive versions are pinned for npm users too.

**It says out loud where it does not protect you.** "Pi does not include a built-in
permission system for restricting filesystem, process, network, or credential access.
By default, it runs with the permissions of the user and process that launched it"
(`references/pi-1.0.0/README.md:42`) — followed by three containerisation patterns it
does support (same file, lines 44-48). Naming the gap is more useful than leaving it to
be discovered.

## What we adopted

Taken as behaviour. Nothing was copied as text.

- **The two-level loop.** Outer boundary drains follow-ups; the inner level drains
  steering after every turn. `crates/epsilon-core/src/lib.rs:2012` is the outer loop and
  `:2015` the inner one; `:2381` drains steering and `:2430` drains follow-ups. ADR-002
  commits to this shape at `docs/design/epsilon-decisions.md:44-48`.
- **Steering and follow-up queues, with a per-drain-point delivery mode.** The two
  queues and their modes are fields on the agent at
  `crates/epsilon-core/src/lib.rs:544-545`, constructed at `:682-683`, and fed by
  `steer` (`:1055`) and `follow_up` (`:1114`). `OneAtATime` is the default, matching the
  constructor pi documents; `All` remains available.
- **Events as the only output of the loop.** `crates/epsilon-core/src/lib.rs:7-8`:
  front ends are reducers over the broadcast event stream, and
  `crates/epsilon-contracts/src/agent_event.rs:1` is the taxonomy they reduce.
- **The built-in coding tools.** Read, write, edit, find, glob, grep, ls, bash and the
  rest are listed at `crates/epsilon-tools/src/lib.rs:31-44`, each with typed arguments
  and a schemars-derived schema.
- **Sessions as a JSONL tree.** `crates/epsilon-session/src/store.rs:1` states the
  shape; appending is `:1222`, navigating the active branch is `:1580`, branching with
  a summary is `:1609`, forking at a chosen entry is `:1723`, and the derived
  model-visible view is `:1873`.
- **Prompt assembly from an identity block plus discovered context files.**
  `crates/epsilon-core/src/prompt.rs:1-12` is Epsilon's version of pi's
  `core/system-prompt.ts`, cited by name at `crates/epsilon-core/src/prompt.rs:35,44`.
- **Project trust.** `crates/epsilon-runtime/src/lib.rs:118` names the trust file
  `.epsilon/trust.json`; `:382` names pi's layout as the origin.

## What we adapted

Same idea, different mechanism — recorded here so the difference is not mistaken for a
port.

- **Tool ordering.** pi preflights a batch sequentially and then runs the parallel-safe
  calls concurrently. Epsilon makes ordering *per resource*: three edits to three files
  run together, two edits to one file stay in call order, and a tool that declares no
  resource still makes the batch a barrier — `crates/epsilon-core/src/exec.rs:1-10` and
  `crates/epsilon-contracts/src/tool.rs:17`.
- **Reads page instead of loading the file.** pi reads the whole file and slices
  (`packages/coding-agent/src/core/tools/read.ts:275-323`, as recorded at
  `crates/epsilon-tools/src/util.rs:530-538`). Epsilon streams the page so resident
  memory is one line plus the window: `crates/epsilon-tools/src/util.rs:521-543`, and
  the function itself at `:544`.
- **The turn seams are a typed, async, panic-contained trait.** pi's callbacks may
  return a Promise, so `TurnHooks` is `#[async_trait]` and a panicking hook is contained
  at the call site rather than unwinding the run —
  `crates/epsilon-core/src/lib.rs:2305-2374`. The recorded deltas from pi are written
  out at `docs/design/epsilon-decisions.md:69-72`.
- **Byte-stable prompt assembly in three tiers.** pi assembles a prompt; Epsilon makes
  the assembly deterministic and tiers it so provider caching has something stable to
  keep — `crates/epsilon-core/src/prompt.rs:1-12`. One divergence is deliberate and
  written down at `crates/epsilon-core/src/prompt.rs:38-41`: pi drops its tool list
  along with a custom system prompt, which for Epsilon would let a user's file make the
  model believe in tools that are not loaded.
- **Trust pins a content hash and blocks.** ADR-012 records this as "Project trust
  adapted from pi's trust-manager concept with hash pinning so silently changed project
  rules re-prompt" (`docs/design/epsilon-decisions.md:520`); the interactive half is
  `crates/epsilon-tui/src/trust_store.rs:5` and the blocking re-prompt is described at
  `docs/design/epsilon-decisions.md:507-514`.

## What we skipped, and why

- **In-process extensions.** pi loads extension code into its own process
  (`packages/coding-agent/docs/how-pi-works.md:43`), which is how a third party can
  replace part of the core. Epsilon's `epsilon-ext` runs extensions as separate
  processes speaking line-delimited JSON, with a per-call timeout, a crash budget and
  manifest-validated tools — "Third-party code never runs inside the harness process"
  (`crates/epsilon-ext/src/lib.rs:1-13`, ADR-009). **Different goal:** a single Rust
  binary with `unsafe_code = "forbid"` and a permission gate. Both choices are
  internally consistent; they answer different questions.
- **The terminal UI library.** `@earendil-works/pi-tui` is a TypeScript renderer with
  differential drawing; Epsilon's terminal front end is ratatui over crossterm.
  **Different runtime** — a Rust TUI cannot embed it. The comparison and the reasons
  are in [the TUI decision](TUI-DECISION.md).
- **Containerisation as the answer to permissions.** pi's README offers three patterns
  (Gondolin, Docker, OpenShell) as the way to get a boundary
  (`references/pi-1.0.0/README.md:44-48`). Epsilon ships OS confinement in the product
  and refuses to run a command unconfined when confinement was asked for:
  `crates/epsilon-safety/src/sandbox.rs:1-14`. **Different deployment:** requiring a
  micro-VM or a container to be safe is a reasonable answer for a Node CLI and the wrong
  one for a Windows-first binary that should be safe as installed.
- **The npm and standalone-binary release path.** pi builds a compiled executable with
  `bun build --compile` and ships a shrinkwrapped npm package
  (`references/pi-1.0.0/packages/coding-agent/package.json:44`). Epsilon ships cargo
  builds. **Different supply chain**, same supply-chain discipline.
- **`pi-codemode`** — model-written JavaScript that calls the agent's own tools
  (`packages/agent/README.md:506-508`). **Not needed yet.** It would mean embedding a
  JavaScript engine, and no user has asked for it.
- **Virtual models and the router** (`packages/coding-agent/docs/session-format.md:99`).
  **Not needed yet.** Epsilon resolves a concrete provider and model id; a routing layer
  has no consumer here.

## Licence compatibility

**Compatible.** pi is MIT and Epsilon is MIT, so adaptation is permitted provided the
copyright notice and licence text travel with it.

Concretely, that means:

- Every substantial piece of pi's MIT text, including the line "Copyright (c) 2025
  Mario Zechner", must accompany any copied or adapted body.
- Each adapted file carries a header naming pi and the file it came from.
- `THIRD_PARTY_NOTICES` gains an entry for pi if anything is ever copied.
- The credit line in [`../../CREDITS.md`](../../CREDITS.md) stays.

**What has actually happened here: no third-party source was copied into this
repository.** Epsilon took behaviour, structure and design decisions, and every place
where a decision was taken differently is named above or in the ADR log. That is why
there is no `THIRD_PARTY_NOTICES` file carrying vendored bodies, and why this page ends
with a compatibility statement rather than an attribution block. If that ever changes,
the three obligations above apply on the same day.

pi's name, logo and its Exy mascot belong to pi's authors and are not ours to take.