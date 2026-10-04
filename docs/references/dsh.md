# DeepSeek Harness

Epsilon is not a port of DeepSeek Harness, and it is not a fork of it. DSH is a
different project solving a different problem, and this page records the four things we
took from it, the way we changed them, and the parts we deliberately left.

The vendored copy is under [`../../references/`](../../references/) at
`references/deepseek-harness-dsh-v0.2.1-alpha.1/`. It is read-only input: not part of
the build, not committed. Paths that start `docs/`, `packages/` or `apps/` are relative
to that directory.

## What it is

DeepSeek Harness — `dsh` — is "an open-source agent harness developed by DeepSeek AI"
(`references/deepseek-harness-dsh-v0.2.1-alpha.1/README.md:5`). It is built on an
**everything-is-a-plugin** architecture powered by Cordis (same file, line 7).

**Version as vendored here: 0.2.1-alpha.1**, read from
`references/deepseek-harness-dsh-v0.2.1-alpha.1/package.json:2-4` — the root workspace
package `@deepseek-ai/dsh-root`. It is a pre-release: the README says so in bold and
adds "THERE WILL BE COMPATIBILITY-BREAKING CHANGES" (same file, line 13).

What ships:

- Five launch profiles — `web`, `headless`, `sdk`, `sdk-minimal`, `acp`
  (`references/deepseek-harness-dsh-v0.2.1-alpha.1/docs/architecture.md:19,45`).
- A browser front end and a separate Electron desktop application
  (`docs/architecture.md:51-55`, same directory).
- 54 groups under `packages/`, holding 256 packages one level below them, each owning
  one concern — session, tools, llm, sandbox, spill, subprocess, goal, workflow — plus
  63 packages under `packages/client/` for the browser front end. The layout is
  documented at `references/deepseek-harness-dsh-v0.2.1-alpha.1/AGENTS.md`.
- A Python SDK and a TypeScript SDK that both project the same loop
  (`references/deepseek-harness-dsh-v0.2.1-alpha.1/AGENTS.md`, "Conventions").

DSH is a large, actively developed system. Epsilon is a single Rust binary with no
runtime dependencies. Every item below is a decision taken *from* it, never a
reimplementation of it.

## Who made it, and under what licence

- **Author.** DeepSeek AI (`references/deepseek-harness-dsh-v0.2.1-alpha.1/README.md:5`).
- **Copyright.** `references/deepseek-harness-dsh-v0.2.1-alpha.1/LICENSE:3` reads
  "Copyright (c) 2026 DeepSeek".
- **Repository.** <https://github.com/deepseek-ai/deepseek-harness>
  (`README.md:34`, and again at `README.md:45`).
- **Licence.** MIT — `README.md:75` and `package.json:4`; full text at `LICENSE:1-21`.
  Third-party dependencies are disclosed separately in
  `THIRD_PARTY_NOTICES.md` (`README.md:77`).
- **Documentation site.**
  <https://deepseek-harness.github.io/deepseek-harness/> (`README.md:9`).
- **Not stated in the vendored copy:** individual contributor names, and a legal entity
  behind the `DeepSeek AI` name.

## Its strengths, in its own terms

**Nothing is privileged, so nothing has to be patched.** `docs/architecture.md:11-13`:
"Every part of the product is a plugin, including the model adapter, the tool registry,
the session log, and the agent loop itself, so each is replaceable from configuration.
There is no privileged core to patch: you extend dsh by mounting a plugin beside the
others, and registrations are effects that unwind when their plugin unloads."

**Composition is data you can read and patch.** A **profile** is a named composition
stored in the Harness home; a **bundle** is a distribution format for configuration
rows and the code they mount; layers apply to an empty list in a fixed order
(`docs/architecture.md:17-29`). You can print the tree your machine boots:
`dsh --profile web --dump-config` — "Any row it prints can be replaced by a patch of
your own" (same file, line 39).

**The loop is written down, not only implemented.** `docs/architecture.md:86` defines
its two units — "A **step** is one model request plus the tools it calls. A **turn** is
zero or more steps" — and lines 88-107 give the turn flow as text, marking which frames
are durable session events and which are live extension points.

**Anything the model sees is reconstructable from the log.** The rule is stated as
standing policy: "Model-visible ⟺ logged: anything that reaches a model request must
be reconstructable from the session log; a new model-visible input requires a session
event" (`references/deepseek-harness-dsh-v0.2.1-alpha.1/AGENTS.md`, "Conventions").

**Oversized output leaves the context and comes back as a locator.** The `spill/` group
"stores full text outside the model's context and returns a locator with retrieval
guidance" (`references/deepseek-harness-dsh-v0.2.1-alpha.1/packages/spill/README.md:12`).
The failure case is handled too: spilling "keeps the original result on storage
failure", so a locator is never a promise about a file that does not exist.

**One child-process service, and a scrubbed environment for every child.** "Every child
process and terminal session the harness runs — bash commands, language servers,
persistent shells, and out-of-process subagent backends — is started, observed, and
terminated through one shared service (`ctx.subprocess`)… The group provides executable
lookup, bounded output capture with spill recovery, provider-managed process ranges
with disclosed weaker fallbacks, and a scrubbed starting environment for every child"
(`references/deepseek-harness-dsh-v0.2.1-alpha.1/packages/subprocess/README.md:12`).
Note the restraint in the same paragraph: "It is not a standalone product feature: the
consuming capability seams decide what each process means."

**Styling has exactly one owner.** `ui-theme` owns the `--dsw-*` static scale, the
semantic aliases, typography, motion, shadows and the light/dark preference; feature
packages "consume semantic aliases and do not define another global theme"
(`references/deepseek-harness-dsh-v0.2.1-alpha.1/docs/web-styling.md:9`). The sheet is
two-tier and it is really two tiers: raw `--dsw-static-*` primitives from
`packages/client/ui-theme/src/styles/design-platform.css:5`, then `--dsw-alias-*`
semantic names that refer to them from `:167`. The rule for everyone else is one
sentence: "Use `--dsw-alias-*` semantic tokens in feature components. Do not copy static
palette values or write literal colors there" (`docs/web-styling.md:18`).

**It is honest about its own safety limits.** "It has not undergone a security audit and
must not be treated as secure or production-ready"
(`references/deepseek-harness-dsh-v0.2.1-alpha.1/SAFETY.md:7`).

## What we adopted

Taken as behaviour and discipline.

- **Bounded capture with a spill locator.** The rule is that a tool result is capped
  and the full output is written somewhere the model can `read`:
  `crates/epsilon-tools/src/util.rs:837-848`, with the spill written *before* the marker
  is composed (`:862`) so a locator in the text always names a file that exists. The
  contract the tool is handed is `crates/epsilon-contracts/src/tool.rs:344`; the
  crate-level promise is `crates/epsilon-tools/src/lib.rs:7`.
- **Subprocess environment hygiene.** Before spawning a child, drop every environment
  variable whose name looks like a credential —
  `crates/epsilon-safety/src/env.rs:1-14`, with the markers named as DSH's at `:21-32`.
  Applied at the two spawn sites Epsilon owns:
  `crates/epsilon-tools/src/shell.rs:525-528` and
  `crates/epsilon-lsp/src/client.rs:102`.
- **Fail-closed confinement, and a denial that reads differently from a failure.**
  `crates/epsilon-safety/src/sandbox.rs:3-7` ports DSH's sandbox provider, including its
  refusal to run unconfined when a backend was demanded and its typed denial signatures.
  The Windows Job Object backend is `crates/epsilon-safety/src/sandbox/jobobject.rs:11`,
  and the process-range discipline is at `:206`.
- **Results commit in model order regardless of completion order.**
  `crates/epsilon-core/src/exec.rs:9-10` names it a DSH rule and gives the reason:
  replay-stable transcripts.
- **Sticky typed turn endings.** `crates/epsilon-contracts/src/agent_event.rs:10` and
  `:30-31`: `MaxTokens` seen anywhere in a run is not erased by a later clean step,
  because the context is still saturated.
- **The goal domain.** A goal lives in the session log as entries, every mutation names
  the `{id, revision}` it believes is current, activation is process-local, and rounds
  are the only budget — `crates/epsilon-contracts/src/goal.rs:3,11-20`. The 256-round
  default is DSH's, kept at `crates/epsilon-contracts/src/goal.rs:30-33`, and the
  three-consecutive-rounds rule for "blocked" at `:37-41`.
- **The IDE workflow.** Activity bar, sidebar, editor, agent panel, status line — as
  **named constants with an arithmetic that says why**, in
  `crates/epsilon-desktop/src/layout.rs:44-81`.
- **Composition as one place, reachable by every front end.**
  `crates/epsilon-runtime/src/lib.rs:1-2`; the reasoning is ADR-021 at
  `docs/design/epsilon-decisions.md:969`.

## What we adapted

Same idea, different substrate.

- **The IDE workflow onto a native carrier.** DSH's desktop is an Electron shell. Epsilon
  took the workflow and wrote a different carrier, because Epsilon's desktop must be a
  real window. ADR-020 states the constraint and the choice at
  `docs/design/epsilon-decisions.md:893-900`; the window is egui/eframe over winit and
  wgpu, and `crates/epsilon-desktop/src/lib.rs:1-21` says what the crate may and may
  not own. This is a difference in goals — see "What we skipped" below.
- **The two-tier token idea onto something that is not CSS.** DSH's tiers are a
  stylesheet of custom properties. Epsilon's are the terminal front end's named
  *roles*, converted one way into egui colours with no desktop-specific values:
  `crates/epsilon-desktop/src/theme.rs:1-22` and the converter at `:28`. Where DSH's
  rule is "do not write literal colours in a feature component", Epsilon's is
  enforceable: `crates/epsilon-desktop/src/theme.rs:55-67` holds a contrast floor and a
  test that fails if a palette drops below it.
- **Bounded capture measured in bytes, not estimated tokens.** DSH's tool-result policy
  is opt-in through `maxInlineTokens` (`packages/spill/README.md:12`). Epsilon uses two
  constants a test can assert on — `crates/epsilon-tools/src/util.rs:16-18` — and keeps
  both ends of the output, with the omission marker naming exact line and byte counts
  (`crates/epsilon-tools/src/util.rs:837-848`, `:960-972`).
- **Composition as a crate graph rather than a plugin tree.** The same rule, enforced by
  the compiler: a front end that wants behaviour the composition does not offer changes
  `epsilon-runtime`, not its own copy — `crates/epsilon-runtime/src/lib.rs:29-33`.
- **The goal domain, narrowed.** No round driver, no completion evaluator, no parallel
  goals — `crates/epsilon-contracts/src/goal.rs:22-24`. The reasons are each its own;
  the narrowing is stated where it is made rather than discovered later.

## What we skipped, and why

- **The Electron carrier.** DSH's desktop "ships its exact dsh production runtime in
  signed resources", "Electron starts the private Desktop Host in Electron Node mode",
  and "The Host invokes the shared CLI profile runner and complete Web application"
  (`docs/architecture.md:53-55`). That is a coherent product: one web profile, two
  carriers, one bundle, no second implementation of the UI. Epsilon needs the opposite —
  a window drawn by the process that owns the agent — because "the desktop must stay a
  real desktop application" is a standing constraint here
  (`docs/design/epsilon-decisions.md:893-898`). **Different goal, not a defect.**
- **The Cordis plugin kernel and the plugin tree.** ADR-002 weighed it and declined:
  DSH's durable inbox and phase machine "is more capable but its cost is ~150 packages
  and a vendored DI kernel - complexity Epsilon cannot carry"
  (`docs/design/epsilon-decisions.md:78-82`). Epsilon's answer is a static crate graph,
  which cannot be reconfigured at boot and does not need to be.
- **The web front end and its ~60 client packages.** Epsilon's two front ends are a
  terminal and a native window. There is no browser target to serve.
- **The SQLite session store and its `SCHEMA_VERSION`.** Epsilon's session is an
  append-only JSONL tree with leases and repair —
  `crates/epsilon-session/src/store.rs:1`, `crates/epsilon-session/src/repair.rs:1`.
  **Different goal:** a session file a person can open in an editor, read, and diff.
- **Executable configuration.** DSH's loader allows `!!js` under a plugin's `config`
  (`references/deepseek-harness-dsh-v0.2.1-alpha.1/AGENTS.md`, "Secrets / .env").
  Epsilon refuses the category rather than restricting it —
  `docs/design/epsilon-decisions.md:516-518`.
- **The bilingual documentation pairs.** Every DSH document has a `.zh.md` counterpart
  under a stated i18n contract. **Cost:** this repository has no translation reviewer,
  and a half-maintained pair is worse than one accurate document.

### The citation ADR-020 carries, and what the vendored copy actually shows

ADR-020 cites
`references/deepseek-harness-dsh-v0.2.0-rc.1/apps/desktop/src/main.ts:408,421`.
**That version is not vendored here.** `references/` holds only
`deepseek-harness-dsh-v0.2.1-alpha.1`, so the line numbers cannot be opened and the
claim behind them cannot be checked against the cited file.

The claim itself holds, and the code that carries it in the copy we do have is:

- `references/deepseek-harness-dsh-v0.2.1-alpha.1/apps/desktop/src/host-process.ts:189`
  — the Electron shell spawns a Node child running the Desktop Host.
- `apps/desktop-host/src/index.ts:1` — "Launch the Desktop profile through the Web
  application and report its URL to Electron."
- `apps/desktop-host/src/index.ts:103-104` — that URL is
  `http://127.0.0.1:<port>`, sent to the shell over IPC.
- `apps/desktop/src/main.ts:450-452` — the shell keeps it as `hostUrl`.
- `apps/desktop/src/main.ts:399,431` — the window is pointed at `dsh-app://app/` and
  `window.loadURL` is what loads it.
- `apps/desktop/src/main.ts:662-677` — requests under that origin are forwarded over
  HTTP to the child's loopback URL; a short list of static documents is served from the
  bundle instead.

So the mechanism is slightly more precise than ADR-020's wording: the `BrowserWindow`
loads a privileged custom scheme, and the application's content is fetched from the
Node child's HTTP server. That is still a browser rendering a web profile. The ADR's
conclusion is unchanged; its citation names a tree a reader cannot open.

## Licence compatibility

**Compatible.** DeepSeek Harness is MIT and Epsilon is MIT, so adaptation is permitted
provided the copyright notice and licence text travel with it.

Concretely, that means:

- Every substantial piece of DSH's MIT text, including the line "Copyright (c) 2026
  DeepSeek", must accompany any copied or adapted body.
- Each adapted file carries a header naming DeepSeek Harness and the file it came from.
- `THIRD_PARTY_NOTICES` gains an entry for DeepSeek Harness if anything is ever copied.
  DSH keeps its own `THIRD_PARTY_NOTICES.md`, so the entry would name us as a
  downstream, not a redistributor of its dependency set.
- The credit line in [`../../CREDITS.md`](../../CREDITS.md) stays.

**What has actually happened here: no third-party source was copied into this
repository.** Epsilon took four ideas — bounded capture, subprocess discipline, a token
system with one owner, and the shape of an IDE workflow — and reimplemented each in
Rust. That is why there is no `THIRD_PARTY_NOTICES` file carrying vendored bodies, and
why this page ends with a compatibility statement rather than an attribution block. If
that ever changes, the three obligations above apply on the same day.

DSH's name, its DeepSeek brand marks and its own product identity belong to DeepSeek
and are not ours to take.