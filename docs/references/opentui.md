# OpenTUI

## What it is

OpenTUI is a library for building terminal user interfaces. In its own words, from
`references/opentui-0.5.14/README.md`:

> - It is written in Zig.
> - You write TypeScript directly or through React and Solid.
> - You arrange boxes and text with flexbox.
> - You add selects, inputs, and scroll boxes with keyboard and mouse controls.

Version vendored here: **0.5.14** (`references/opentui-0.5.14/`).

It is a set of packages under one repository:

| Package | What it is |
|---|---|
| `@opentui/core` | the native terminal UI library with TypeScript bindings |
| `@opentui/react` | React renderer over the core |
| `@opentui/solid` | Solid renderer over the core |
| `@opentui/keymap` | key binding and command routing, for terminal and browser |
| `@opentui/qrcode`, `@opentui/three`, `@opentui/ssh` | QR encoding, WebGPU 3D, SSH server integration |
| `@opentui/native`, `@opentui/examples`, `@opentui/web` | private workspaces: the Zig implementation, examples, and the documentation site |

## Who made it, and under what licence

- **Repository:** https://github.com/anomalyco/opentui
- **Licence:** **MIT** — `references/opentui-0.5.14/LICENSE` reads "Copyright (c) 2025 opentui"
- **Site:** https://opentui.com · **Docs:** https://opentui.com/docs
- **Package metadata:** `packages/core/package.json` declares `"name": "@opentui/core"`,
  `"version": "0.5.14"`. Its top-level `license` field is empty, so the `LICENSE` file is the
  authority.

There is no individual author named in the vendored copy beyond the organisation name in the
copyright line. We have not guessed one.

## Its strengths, in its own terms

**It is proven at scale, by someone else's users.** The README says: "OpenCode uses OpenTUI
in production for millions of users." That is the single most valuable thing to know about it,
and it is the reason this page exists rather than a dismissal.

**Flexbox layout for terminals.** Arranging boxes and text with flexbox rather than computed
rectangles is a real idea, not a convenience. Terminal layout is where most TUI code becomes a
pile of magic numbers; declaring the shape and letting one place resolve it is better.

**Keyboard and mouse as first-class input.** Selects, inputs and scroll boxes ship with both,
so a component author is not re-deriving focus and hit-testing for every widget.

**Key bindings and command routing are separated out.** `@opentui/keymap` is a distinct
package for "key binding and command routing" — the seam between *what key was pressed* and
*what the app should do*. Most TUI libraries give you the first half only.

**It publishes its own agent skill and its own agent guide.** `README.md` documents installing
the documentation as a skill, and the repository ships an `AGENTS.md`. For a project built by
and with coding agents, that is a considered choice rather than an accident.

## What we adopted

We adopted **none of its code**, and we took three ideas.

**1. Layout declared as relationships, resolved in one place.** Written up as decision input in
[`TUI-DECISION.md`](TUI-DECISION.md).

**2. Key binding separated from command routing.** Epsilon has the same gap from the other
side and is fixing it: `crates/epsilon-tui/src/keybindings.rs:359` resolves a chord to a string
id, and nothing maps that id to a `Command`. The command registry work in Epsilon is the same
seam OpenTUI ships as a package.

**3. Its `AGENTS.md` as an example of agent-facing conventions.** Two rules from it shaped how
this project briefs subagents: a delegated brief must **name the owned files and the required
checks**, and an actionable finding gets **an independent correctness review plus a separate
reproduction check**. We learned the shape of that file. We did not copy it.

## What we adapted

Nothing. No OpenTUI source was copied into this repository, so there is no adapted file to
mark and nothing to carry in `THIRD_PARTY_NOTICES`.

## What we skipped, and why

| Skipped | Why |
|---|---|
| **The library itself** | Decided against adoption, with the reasoning and the evidence in [`TUI-DECISION.md`](TUI-DECISION.md). Briefly: it needs Bun ≥ 1.3 or Node ≥ 26.4 at runtime, ships eight prebuilt native FFI packages, and would add Bun and Zig to a Rust-only workspace — against a product promise of one binary with no Node or Python runtime. |
| **`@opentui/react`, `@opentui/solid`** | Epsilon has no JavaScript. |
| **`@opentui/three` (WebGPU 3D), `@opentui/qrcode`, `@opentui/ssh`** | A coding agent's terminal does not need 3D graphics, QR encoding or an SSH server. They are not "rejected"; they solve problems this product does not have. |
| **Sound and image features** | Noted in the README, not needed here. |
| **The documentation agent skill** | Not installed. It is a large documentation corpus for a library we do not depend on, and installing it would put instructions about an unused toolchain into every agent's context. |

## Licence compatibility

MIT into an MIT project. Compatible. Because **no code was copied**, there is nothing to
attribute at the file level and nothing in `THIRD_PARTY_NOTICES`. The ideas above are ideas,
and ideas are credited in [`../../CREDITS.md`](../../CREDITS.md).
