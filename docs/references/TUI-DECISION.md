# TUI stack decision

**Decision: keep Ratatui. Do not adopt OpenTUI.**

Date: 2026-10-03. Status: decided. Supersedes nothing; the TUI has been Ratatui-based
throughout, so this is a decision to *keep* one option rather than a migration.

## The three options

| | Language | Runtime for the user | Native core | Licence |
|---|---|---|---|---|
| **Ratatui 0.30.2 + crossterm 0.29** (current) | Rust | none — one binary | no | MIT |
| **OpenTUI 0.5.14** | TypeScript over a Zig core | Bun ≥ 1.3 **or** Node ≥ 26.4 | yes, over FFI | MIT |
| codewhale-ratatui | Rust | none — a library, not a runtime | no | MIT |

Note that codewhale-ratatui is not a competing *stack*: it is a component kit **on** Ratatui.
Choosing Ratatui keeps it available. What it cannot be is the thing that decides this.

## Why: the evidence

**1. OpenTUI requires a JavaScript runtime, and Epsilon's product promise is that it does not.**
`references/opentui-0.5.14/packages/core/package.json` declares
`"engines": {"bun": ">=1.3.0", "node": ">=26.4.0"}`. `README.md` states the same and adds
"Use Bun 1.4.0 or later on native Windows arm64."

Epsilon's `README.md` says, in one line: *"It is a single binary with no required Node or
Python runtime."* Adopting OpenTUI would delete that sentence or make it false. That is a
product decision, not a technical preference, and it is the one that settles the question.

**2. Shipping its native Zig core means a second install channel for the CLI.**
`references/opentui-0.5.14/packages/core/README.md`: "TypeScript packages call native Zig
through an internal foreign function interface (FFI) boundary." The distribution is eight
prebuilt platform packages, named in its `package.json` as optional dependencies:
`@opentui/core-darwin-x64`, `-darwin-arm64`, `-linux-x64`, `-linux-arm64`, `-win32-x64`,
`-win32-arm64`, `-linux-x64-musl`, `-linux-arm64-musl`.

Epsilon ships by `cargo install`. A CLI whose renderer is eight platform-specific native
packages resolved by a JavaScript package manager has two install stories, two failure
modes, and a support matrix nobody on this project is paid to maintain.

**3. It would add a second language toolchain for widgets.**
OpenTUI's `README.md`: "Development requires Bun 1.4.1 or later and Zig 0.16.0."
`package.json` declares `"packageManager": "bun@1.4.1"`. Contributing to it, patching it, or
debugging its renderer means Bun **and** Zig in a workspace that is otherwise one Rust
toolchain.

**4. It would break the invariant this project is built on.**
The whole architecture is one command registry read by both front ends. Today that is a
direct Rust-to-Rust value: `crates/epsilon-desktop/src/commands.rs:67` calls
`epsilon_tui::picker::parse_command`, and the desktop matches `picker::Command` exhaustively
so a new terminal command breaks the desktop build. Across a Rust/TypeScript boundary the
same sharing needs a serialization seam and a second, driftable copy of the command list.

**5. The switching cost is the largest of the three, by an order of magnitude.**
`crates/epsilon-tui` is 38,067 lines across 49 files with 509 tests, and its golden-frame
suite drives ratatui's `TestBackend` against real `Frame`/`Buffer` values
(`crates/epsilon-tui/src/app/golden.rs`). The theme resolves to ratatui `Color` and
`Modifier`; the markdown renderer emits `Line`/`Span` over `ratatui::style::Style`. A
migration replaces the types under every one of those tests.

## What the licence does and does not decide

OpenTUI is MIT (`references/opentui-0.5.14/LICENSE`, "Copyright (c) 2025 opentui"), and so
is Epsilon. **Licence compatibility is therefore not an obstacle and was not the deciding
factor.** Anyone arguing against OpenTUI on licence grounds is arguing wrongly. It is
excluded on runtime, packaging, toolchain and architecture.

Equally: OpenTUI is genuinely good and genuinely used — its own README says "OpenCode uses
OpenTUI in production for millions of users." That is a real strength and is recorded as one
in [`opentui.md`](opentui.md).

## What we take from OpenTUI anyway

The task in front of this project is not "pick a winner"; it is "make one coherent system".
Three ideas from OpenTUI are worth having regardless of who renders the pixels:

1. **A declarative, constraint-shaped layout instead of hand-computed rectangles.**
   OpenTUI lays out with flexbox; Epsilon computes rects by hand in
   `crates/epsilon-tui/src/ui/layout.rs` and `crates/epsilon-desktop/src/layout.rs`, and
   Epsilon's own desktop investigator found pane resizing and collapsibility simply absent.
   The flexbox *idea* — declare regions and their relationships, let one place resolve them —
   is the fix, and it can be implemented in Rust without adopting anything.
2. **Key bindings and command routing as a separate concern.**
   `@opentui/keymap` is described as a "Key binding and command routing library". Epsilon has
   the same shape of gap from the other side: `KeybindingsManager::matches` resolves a chord to
   a *string id* that nothing then maps to a `Command`
   (`crates/epsilon-tui/src/keybindings.rs:359`), and the palette's `binding` field is a
   hand-typed string rather than a table lookup (`crates/epsilon-tui/src/picker.rs:1029`). One
   registry that produces both the chord and the action closes it.
3. **Its agent-facing conventions.** `references/opentui-0.5.14/AGENTS.md` is a compact,
   useful example of an agent contract — notably its rule that delegated briefs must name
   owned files and required checks, and that an independent review and a separate
   reproduction check are requested for actionable findings. We learned the shape of that;
   we did not copy the file.

## What we take from codewhale-ratatui

The idea of a fixed layout order that **yields** information as the window narrows rather than
clipping it, the approval and diff views, the terminal-profile fallbacks we actually have
(`NO_COLOR`, ASCII-only glyphs), and the discipline of pinning a profile per test so a frame is
compared against a known rendering environment.

Two things in their design we did **not** take, and are recorded here so nobody assumes we did:
Epsilon has **no workbar** — its work facts live in the header metrics row and its overlays
live in `crates/epsilon-tui/src/ui/overlay.rs` — and Epsilon has **no reduced-motion path at
all**, which is an open gap against `AGENTS.md` §8 rather than parity. These are design ideas
taken in Rust, with attribution. See [`codewhale-ratatui.md`](codewhale-ratatui.md).

## Consequences, stated so they are not assumed

- **One TUI stack ships.** Ratatui only. No second terminal toolkit, ever, for widgets.
- The command registry is built in Rust, shared by both front ends by construction.
- Accessibility work in the desktop cannot be deferred to a toolkit that already had it.
  AccessKit reaches Epsilon through egui's default features, and that is the path.
