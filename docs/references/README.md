# Reference harnesses and components

Seven projects were studied while building Epsilon: six agent harnesses and one
reverse-engineering component. All seven are vendored read-only under
[`../../references/`](../../references/), and all seven are **MIT-licensed**. Epsilon is
MIT, so code from any of them could legally have been adapted with attribution.

**In the event, no third-party source code was copied into this repository.** What Epsilon took
is behaviour, structure and design decisions. That is why there is no `THIRD_PARTY_NOTICES`
file carrying vendored bodies, and why every page below ends with a
licence-compatibility note rather than an attribution block. REA is invoked as an
external package over MCP; it is not vendored into the binary either.

> **One historical note.** These pages were written while the native desktop crate existed.
> ADR-039 removed `crates/epsilon-desktop`, so sections that cite that crate describe a
> removed experiment, not the current product. The Pi study and the terminal sections are
> current; the desktop comparisons are read as history.

Facts — authors, links, licences — were read from each project's own `LICENSE`, `README` and
package metadata in the vendored copy. Nothing here is from memory.

## The seven

| Reference | Who | Licence | What it is | What we took | Where that lives |
|---|---|---|---|---|---|
| **[pi](pi.md)** | Mario Zechner (`earendil-works`) | MIT | The TypeScript agent harness Epsilon is a port of | The agent loop: two-level provider/tool cycle, steering and follow-up queues, coding tools, sessions, prompt assembly | `crates/epsilon-core/src/lib.rs`, `crates/epsilon-session/`, `crates/epsilon-tools/` |
| **[DeepSeek Harness](dsh.md)** | DeepSeek | MIT | A full agent harness with a web front end | Bounded capture, subprocess lifecycle, design tokens, and the **IDE workflow** | `crates/epsilon-runtime/`, `crates/epsilon-safety/`, `crates/epsilon-tools/` |
| **[opencode](opencode.md)** | opencode (`anomalyco`) | MIT | A coding tool, whose TUI is built on OpenTUI | Permission rules, change provenance, language-server edit feedback | `crates/epsilon-safety/src/engine.rs`, `crates/epsilon-tools/src/mutation.rs`, `crates/epsilon-lsp/` |
| **[Hermes Agent](hermes.md)** | Nous Research | MIT | An agent with a flexible toolset system | Observer identity, verification evidence, recovery discipline | `crates/epsilon-tools/src/read_log.rs`, `crates/epsilon-core/` |
| **[OpenTUI](opentui.md)** | opentui (`anomalyco`) | MIT | A Zig-core terminal UI library with a TypeScript API | Three ideas: declared layout, key binding separated from command routing, agent-facing conventions | Ideas only — see [the decision](TUI-DECISION.md) for why the library was not adopted |
| **[codewhale-ratatui](codewhale-ratatui.md)** | Codewhale Contributors | MIT | A reusable Ratatui component kit — presentation only | The *idea* of a fixed layout order that yields information as the window narrows, the approval and diff views, and pinning a terminal profile per test | Ideas only. Epsilon has **no** workbar, and **no** reduced-motion path; see the page for what is adapted, borrowed, and still missing |
| **[REA](rea.md)** | morluto | MIT | A reverse-engineering MCP server for binaries, apps, .NET and the web | The optional `--rea` integration, the Windows launch wrapper, the per-server deadline and the evidence-first workflow skill | `crates/epsilon-runtime/src/integrations.rs`, `crates/epsilon-core/src/builtin_skills/` |

## The one decision that spans them

**[TUI stack decision](TUI-DECISION.md)** — Ratatui, OpenTUI, or codewhale-ratatui; the
evidence; what we borrow from the two we did not choose; and why licence was *not* the deciding
factor. Read this before proposing a terminal toolkit.

## A note on tone

These are seven good projects, and the pages above say what is good about each
one first. Where Epsilon differs, the difference is a difference in what each
project is for — a single Rust binary with no runtime dependency and one terminal
IDE over one core. That is not a judgement on opencode, DSH, OpenTUI or anyone
else's design.
