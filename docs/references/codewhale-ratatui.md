# codewhale-ratatui

## What it is

**codewhale-ratatui** is a reusable [Ratatui](https://ratatui.rs) component kit. Its own
`Cargo.toml` (`references/codewhale-ratatui-main/Cargo.toml`) describes it as:

> Reusable Ratatui components from Codewhale: native workbar, composer, views, themes, ocean
> and motion.

It is **v0.1.0**, edition 2024, `rust-version = "1.89"`, vendored read-only at
`references/codewhale-ratatui-main/`. It has no binary and no event loop; the application-shaped
code lives under `examples/`.

A *component kit* is a library of drawing widgets that the host application assembles. The kit
holds no agent, no session, no permission engine. It receives plain values — a draft string, a
list of rows, a status — and paints them into a Ratatui buffer.

### The host / presentation boundary

This boundary is the single most useful thing the kit states, and it states it in its own terms.
`DESIGN.md:81-86`:

> Components own paint and local presentation geometry. The host owns Engine events, permission
> decisions, queue mutation, Markdown parsing, clipboard, link navigation, persistence and the
> event loop.

`README.md:767-769` says the same thing as a negative list — the kit does not calculate a diff,
parse Markdown, validate credentials, authorise a command, estimate cost, or run an agent.
`VIEWS.md:157-159` repeats it for the work dock: host input selects the row and dispatches its
action; the library does not run tools, inspect Git, read files, or infer prices.

**Engine adoption is separate work.** `DESIGN.md:88-93` closes with it: its own checks
"establish library behavior. Engine adoption is separate work." That sentence is the honest
boundary of the project — the kit is extractable presentation, and the surrounding application
is not part of the deliverable.

### Terms used below

- **Terminal profile** — what a terminal can actually draw. Colour depth (24-bit, 256, 16, or
  none), whether a background was measured or is unknown, and whether the font can render
  box-drawing characters. Nine profiles are defined (`README.md:894-895`): `dark-truecolor`,
  `dark-graphite`, `light-truecolor`, `dark-256`, `light-256`, `ansi-16`, `unknown-ground`,
  `no-color`, `ascii`.
- **`NO_COLOR`** — the [no-color.org](https://no-color.org) convention. A non-empty `NO_COLOR`
  in the environment means the reader has asked for no colour; every state must be carried by
  something else (weight, a mark, a word).
- **Workbar** — a tabbed dock of working facts. `VIEWS.md:146-155` names its eight panels:
  Tasks, Fleet, Jobs, Files, Notes, Context, Git, Cost, placeable at the bottom, top, left or
  right.
- **Role** — a semantic colour name such as `Foreground` or `Danger`. Components name roles;
  the theme resolves a role for the terminal in use (`references/codewhale-ratatui-main/src/lib.rs:56-79`).

## Who made it, and under what licence

- **Repository:** https://github.com/Hmbown/codewhale-ratatui — read from the `repository` field
  of `references/codewhale-ratatui-main/Cargo.toml`.
- **Licence:** **MIT**. `references/codewhale-ratatui-main/LICENSE` carries **both** copyright
  lines in full:

  > Copyright (c) 2026 Codewhale Contributors
  > Portions adapted from Codewhale: Copyright (c) 2024-2025 DeepSeek-TUI Contributors

  The second line is not boilerplate. It records that parts of the work descend from
  **DeepSeek-TUI Contributors** (2024–2025), not from Codewhale's own 2026 work. Any adaptation
  has to carry both.

`README.md:955-956` adds the practical note: use it as a Git dependency during development; the
crate has not been published to a package registry. That is why we studied a vendored copy
rather than adding a dependency.

## Its strengths, in its own terms

These are stated from `README.md`, `DESIGN.md`, `QUALITY.md`, `VIEWS.md` and `COMPONENTS.md`,
and each one is checkable by opening that folder.

**A layout order, not a layout suggestion.** `DESIGN.md:41-47` states the baseline: no permanent
top header, no permanent right sidebar. Conversation, pending input, the rounded composer,
posture, workflow progress, metrics and workbar form **one vertical layout**. Work has priority,
and narrow surfaces shed secondary information before essential input, permission and
model/context information (`DESIGN.md:49-52`).

**Every terminal profile is a first-class target.** `README.md:716-731` gives the fallbacks:
truecolor keeps exact source inks; 256 colors map to the nearest fixed-cube index; 16 colors or
an unknown ground use named terminal colours and visible marks; `NO_COLOR` means "words, weight
and marks carry every state"; `CODEWHALE_ASCII_SAFE=1` makes component chrome ASCII while
user-authored Unicode stays as text. `CODEWHALE_APPEARANCE=light|dark` exists for a ground that
cannot be measured.

**Every catalogue entry is exported in every profile.** `DESIGN.md:88-90`: "Every catalog entry is
exported in nine terminal profiles and included in the generated README's dark and light
boards." The README gallery is **195 entries** (`README.md:47-48`), rendered from actual Ratatui
buffers rather than screenshots. The generated boards can be re-checked with
`python3 tools/render-gallery.py … --check` (`README.md:839-840`).

**Contrast is a floor, applied per role.** `references/codewhale-ratatui-main/src/theme.rs:614-684`
audits roles at both truecolor and ANSI-256: body roles and state hues at **4.5:1**, secondary
roles and control edges at **3:0:1**. For *decorative* ink — the whale character — `Ombre`
keeps a `CONTROL_FLOOR` of 3.0 (`src/ombre.rs:90`) and `Scene::paint_with_contrast` accepts an
optional floor (`src/whale_motion/scene.rs:248-253`), mixing only insufficient inks toward the
theme foreground so native ink is otherwise retained exactly (`DESIGN.md:75-77`).

**Motion is capped, quiet, and the host's clock is never stolen.** `DESIGN.md:66-70`: one
session-scoped `Stage` per foreground session, advanced on the host's clock; terminal cadence
capped at **six paints per second in action and two at rest**; hiding a surface stops painting
and resuming discards missed motion; **reduced motion uses the authored poster**; the compact
view always keeps readable state words. `README.md:801-806` adds `FrameBudget`, which lets the
host claim one primary spinner per frame and — importantly — lets the host **wait for input
instead of painting identical frames** once transitions settle.

**Checks that would catch a consumer, not just the author.** `QUALITY.md:23-38`: a standalone
consumer crate that borrows widgets through `TestBackend` and selects no terminal backend;
`RUSTDOCFLAGS=-Dwarnings cargo doc`; `cargo package --locked` with CI running the packaged
library's tests; CI across Linux, macOS, Windows and the declared Rust minimum, rejecting
duplicate Ratatui/Crossterm versions; rendering tests over nonzero origins, empty rectangles,
narrow terminals, Unicode, **all nine capability profiles** and all native themes.

**Honest about what it does not claim.** `QUALITY.md:18-21` states plainly that its comparison
table to rat-widget, ratatui-textarea and tachyonfx "does not establish a market ranking or a
performance advantage over those libraries." `DESIGN.md:54-56` marks its optional desktop-style
compositions as *"not substitutes for the TUI baseline."* A library that names its own limits is
easier to depend on than one that does not.

**Every visual fact comes from a pinned source.** `COMPONENTS.md:8-29` records the exact engine
commit, the GPUI commit, the design-token version (1.1.2), the character-export commit and the
kit baseline, then maps each native reference file to the public kit family built from it. The
crosswalk is `Native reference → Public kit family → Kind → Gallery entries`, which is what
makes "where did this pixel come from" answerable.

## What we adopted

Epsilon took **ideas and mechanisms**, written in our own Rust. No codewhale source is in this
repository. Below, every `file:line` is in **our** tree unless the path starts
`references/`.

### 1. The semantic role palette as the single source of colour truth

`crates/epsilon-tui/src/ui/theme.rs` is a macro-generated palette table with **18 semantic
roles** in each of two palettes — `border`, `border_focus`, `text`, `text_dim`, `text_faint`,
`user`, `assistant`, `thinking`, `success`, `danger`, `warn`, `accent`, `code`, `code_block`,
`surface_user`, `surface_pending`, `surface_ok`, `surface_err`
(`crates/epsilon-tui/src/ui/theme.rs:686-728` dark, `:791-849` light). The rule is that a role
names a *meaning*, not an appearance, and that two roles sharing a colour usually means they
are one role (`theme.rs:22-26`).

What we took from codewhale is the **shape**: roles resolved at paint time so a theme change
reaches every widget on the next frame (`references/codewhale-ratatui-main/src/lib.rs:56-79`),
and the palette kept private so `NO_COLOR` is one decision rather than a hunt
(`theme.rs:921-931`). Our `Role::ALL` is generated from the table itself
(`theme.rs:662-669`), so a role nobody can name is a role nothing checks.

### 2. The colour-mode detection rule, and `NO_COLOR`

`crates/epsilon-tui/src/ui/theme.rs:69-98` defines `ColorMode { Truecolor, Ansi16, None }` and
detects it once, at startup: `NO_COLOR` first (`theme.rs:85`), then `COLORTERM`
containing `truecolor`/`24bit` (`theme.rs:89`), then a known-truecolor host — Windows Terminal or
VS Code (`theme.rs:93`) — and otherwise `Ansi16`. The comment states why the default is the
conservative one: claiming truecolor on a terminal that lacks it produces unreadable output.

Under `NO_COLOR`, `Style` collapses to attributes only, and emphasis is decided by
`Emphasis` (`theme.rs:942-948`, `:951-975`) — bold for a value the reader must catch,
dim + italic for background material. The rule that reasoning must never be indistinguishable
from the answer is why `Glyphs::assistant` and `Glyphs::thinking` exist as separate fields
(`theme.rs:177-189`).

### 3. UNICODE vs ASCII glyph tables, selected by `EPSILON_ASCII`

Two complete tables, `UNICODE` (`theme.rs:280-313`) and `ASCII` (`theme.rs:316-353`), chosen by
`Glyphs::detect` (`theme.rs:386-394`): `EPSILON_ASCII=1` forces ASCII, `EPSILON_ASCII=0` forces
Unicode, and a Windows host without a UTF-8-capable terminal gets ASCII by default. Both
directions exist because a default nobody can correct is a bug report waiting to happen
(`theme.rs:382-385`).

The check that keeps the table honest is `Glyphs::ALL` driving a frame-level assertion rather
than a table-level one: `an_ascii_frame_draws_no_character_a_legacy_console_cannot`
(`crates/epsilon-tui/src/app/golden.rs:95-155`) paints real frames with the ASCII table at three
sizes and four panels and reads the drawn rows against a CP437 drawability table
(`golden.rs:76-93`). A test over the table alone would have passed while a widget drew an emoji.

### 4. The desktop imports the terminal's roles rather than inventing its own

`crates/epsilon-desktop/src/theme.rs:173-229` constructs the terminal `Theme` in `Truecolor` mode
and converts each role to an `egui::Color32` through one total, one-directional conversion
(`theme.rs:26-53`). No desktop-specific colour exists in that file; a role the desktop needs
that the terminal lacks is a missing *role* (`theme.rs:12-16`).

The conversion is then pushed to contrast floors the desktop owns: 3.0:1 for text roles, 1.12:1
for surface roles (`theme.rs:55-67`, applied at `:252-276`). Body text holds WCAG AA 4.5:1 and is
a test, not a claim (`theme.rs:376-390`).

**The verification this brief asked about:** the 18-role palette, the `NO_COLOR` path, the
truecolor/ANSI-16 detection and the `EPSILON_ASCII` glyph switch are all present as described,
and the desktop import is real. One correction: `ColorMode` has **three** variants, not two —
`Truecolor`, `Ansi16` and `None` — and `None` is a first-class mode, not an absence.

### 5. The vertical layout order, decided in one place

`crates/epsilon-tui/src/ui/layout.rs` decides the whole frame from `frame.area()` every frame,
in a fixed order, from one `Regions::split` (`layout.rs:79-99`): header, transcript, composer,
one rule, footer. Two invariants keep it honest as the window narrows (`layout.rs:28-37`):
**chrome yields before content**, and below `MIN_WIDTH × MIN_HEIGHT` (`layout.rs:41-43`) the
frame says so instead of drawing a broken layout (`layout.rs:117-119`).

The *rule* is the part taken from the workbar's lesson. `DESIGN.md:44-46` puts the composer
under a single fold that means "everything below this line takes keystrokes"; `layout.rs:20-27`
records that Epsilon once had **two** such folds and cut one, because two say the same thing
twice.

We took the order and the yield rule. We did **not** take the workbar. Epsilon's terminal has
overlays (`crates/epsilon-tui/src/ui/overlay.rs`) rather than a permanent eight-panel dock, and
the facts a workbar would carry — tasks, jobs, files, cost — live in the header's metrics row
(`crates/epsilon-tui/src/ui/header.rs:62-75`) and in overlay panels. That is a difference in
what each UI is for: a dock suits a room with a workspace tree, and Epsilon's terminal is a
conversation.

### 6. Per-profile frame testing

`crates/epsilon-tui/src/app/golden.rs` drives the **production draw path** through
ratatui's `TestBackend` and asserts on the assembled screen, pinning the theme per test
(`golden.rs:33-35`) so a frame can name an exact colour. The deliberate difference from a
snapshot suite: these tests assert **structure, not pixel-exact text** (`golden.rs:12-18`), and
they sweep the colour modes — `None` at `golden.rs:506`, `602`, `1039`, `1645`, `1874`;
`Ansi16` at `833`, `885`, `935`, `1420`, `1644`; `Truecolor` at `386`, `454`, `627`, `701`.

### 7. States that must survive colour being taken away

- **Approval identity.** `crates/epsilon-tui/src/app/keys.rs:339-362` refuses a decision whose ask
  id is not the one the last frame actually painted. Fail-closed and identity-based, not
  "has this dialog been drawn once".
- **Selection carries a bar and a style.** `crates/epsilon-tui/src/ui/overlay.rs:208-220`: on a
  monochrome terminal or with `NO_COLOR`, a style-only selection is invisible and the list is
  unusable.
- **Bands are withheld, not approximated.** `theme.rs:1132-1143`: off truecolor, `surface_*`
  returns an empty style, because a wrong background collides with whatever theme the user runs.
- **Every drawn character is checked against the real code page.** `golden.rs:95-155`, described
  above.

### 8. Text safety, and the honest report about what is still open

`crates/epsilon-runtime/src/text_safety.rs:20-32` strips the bidi embedding/override family
(`U+202A`–`U+202E`), the isolates (`U+2066`–`U+2069`), the directional marks and every C0/C1
control except tab, newline and carriage return. It reports **how many** characters it removed,
because a silent strip is a lie (`text_safety.rs:36-41`).

The gap this project has not closed, stated because the reference's `COMPONENTS.md:85-92` makes
the same demand: grapheme-safe truncation. We use `.chars().take(n)` at several sites, and a
grapheme cluster cut mid-sequence is a rendering defect. Recorded, not fixed.

## What we adapted

Adapted means: we changed something about the mechanism while keeping it. It is **not** a copy,
and nothing here is vendored.

**The profile matrix, narrowed to what Epsilon actually has.** codewhale-ratatui defines nine
terminal profiles (`README.md:894-895`). Epsilon's terminal has three colour modes and two glyph
repertoires (`crates/epsilon-tui/src/ui/theme.rs:69-76`, `:280`, `:316`), so the palette rule is
the same and the matrix is smaller. Where codewhale-ratatui maps an RGB to the nearest fixed
256-cube index (`references/codewhale-ratatui-main/src/color.rs:107`), we resolve the 16 ANSI
slots by hand, one per role, and carry quietness with `Modifier::DIM` instead of a slot
(`theme.rs:613-627`) — because every ANSI slot that reads as "quiet grey" is invisible on one of
the two backgrounds.

**The light palette as a second table, not a transform.** `theme.rs:730-742`: light-mode "quiet
grey" is *darker* than light-mode body text, so the numbers are not a transform of each other and
the table is written twice.

**The host/presentation boundary, applied across two front ends.** codewhale-ratatui's is one
library and one host. Epsilon has two front ends, so the boundary became a rule about where a
decision lives: tokens in one place (`crates/epsilon-tui/src/ui/theme.rs`), converted at the
desktop boundary (`crates/epsilon-desktop/src/theme.rs:26-53`).

**A frame-level profile sweep, not a per-profile export.** codewhale-ratatui exports every
catalogue entry in nine profiles and regenerates README boards from it. Epsilon pins a colour
mode per test and drives the real draw path (`golden.rs:33-35`). The idea — *the profile is part
of the assertion* — is the same; the mechanism is a test rather than a generated gallery.

## What we skipped, and why

**The whale, the ocean and the ombré identity. This is deliberate, and it is theirs.**

Codewhale's visual identity — the C-shaped whale, `OceanColumn`'s three-stop water column
(`DESIGN.md:11-20`), `Ombre`'s spatial washes, the fish, jellyfish and bubbles, the `WhaleState`
vocabulary, and roughly 4 MB of SVG and GIF assets — is **the authors' work and brand**. It is not
ours to take, and Epsilon does not take it. This is the same rule the repository applies to every
reference: never take a name, logo, mascot or brand (`AGENTS.md` §10).

The *underlying techniques* are separable from the character, and two are worth naming honestly:

- **Continuous depth behind a layout.** One absolute vertical column that connects workspace,
  transcript and composer rather than restarting inside each widget (`DESIGN.md:21-28`) is a
  real idea. Epsilon has no such column; its transcript bands are per-message surfaces, which is
  a different and deliberate choice.
- **One foreground per Braille cell.** A terminal cannot colour individual dots, so a coloured
  braille renderer must sample visible ink per cell and choose the majority shape, resolving ties
  by paint order (`DESIGN.md:72-74`). Correct, and specific to a technique Epsilon does not use.

**Animation and motion systems.** `whale_motion::Stage`, `Director`, springs, authored clips,
`FrameBudget`, the six-paints-per-second cadence and the authored reduced-motion posters
(`DESIGN.md:58-70`, `README.md:801-818`) are a substantial subsystem. Epsilon's redraw is a
33 ms ceiling in one place (`crates/epsilon-tui/src/app.rs:91-94`) with no animation. The
*policy* is what matters and is worth remembering: cap the paints, and when reduced motion is
set, show a static poster rather than a degraded animation. Epsilon has no reduced-motion path
today — that is an open item against `AGENTS.md` §8, not a claim of parity.

**Palettes beyond what a coding agent needs.** codewhale-ratatui ships 16 fixed TUI palettes
(`README.md:243-259`) plus an ocean, lagoon, dusk, coral and graphite treatment. Epsilon ships
two: dark and light, both derived from one 18-role table. A developer reads a transcript and a
diff; a decorative palette set is a different product decision.

**Braille rendering and terminal-cell animation.** Spinners in Epsilon are a fixed frame table
in the glyph set (`theme.rs:312`, `:352`) — braille on Unicode, `|/-\` on ASCII — advanced by the
run loop (`crates/epsilon-tui/src/app.rs:268-270`). That is the whole of it.

**Its transcript re-wrapping.** `agents/memory/references-five.md:65-67` records why: it
re-wraps the whole document every frame and `height()` does it a second time, which is the exact
shape this project measured and fixed at M198 (144 µs → 276 ns). Epsilon's `List::place` is the
correct contrast.

**Scrollback and follow-tail design.** There is none in the reference: `Transcript.offset` is a
host-supplied field and there is no `follow_tail` anywhere. Nothing to take, and
`references-five.md:68-69` says so.

**Its benchmarks as a gate.** Hand-rolled, dependency-free, prints a median and a p95, and has no
CI threshold. It is a report, not a gate. Its one useful fact is that paint cost is flat between
100 and 10,000 rows — a claim about O(visible) — and Epsilon measures its own frame path
separately.

**Its `Keymap` as-is.** The *design* is the best idea in that repository — one table that is at
once the dispatch table and the rendered footer — but `references-five.md:57-64` records that
`Keymap::lookup` has no production caller there, verified by grep. The same defect class this
project spends milestones closing, found in a reference we are reading for ideas. Take the design;
wire it or not at all.

## Licence compatibility

**MIT into an MIT project. Compatible.**

- Epsilon is MIT (`LICENSE`, "Copyright © 2026 the Epsilon authors").
- codewhale-ratatui is MIT (`references/codewhale-ratatui-main/LICENSE`, `Cargo.toml`
  `license = "MIT"`).

MIT permits copying, modification and distribution provided the copyright notice and the
permission notice travel with the work. **No codewhale-ratatui source was copied into this
repository**, so there is nothing to attribute at the file level and no vendored body for a
`THIRD_PARTY_NOTICES` file. The ideas above are recorded as ideas in
[`../../CREDITS.md`](../../CREDITS.md).

**The obligation that survives anyway.** The MIT permission notice requires that "the above
copyright notice and this permission notice shall be included in all copies or substantial
portions of the Software." That licence carries **two** copyright lines:

> Copyright (c) 2026 Codewhale Contributors
> Portions adapted from Codewhale: Copyright (c) 2024-2025 DeepSeek-TUI Contributors

Any future adaptation of codewhale-ratatui code — as opposed to an idea — must reproduce **both**
lines, not just the first, and must add a header on the adapted file naming the source
(`AGENTS.md` §10). The second line names DeepSeek-TUI Contributors (2024–2025), so an adaptation
touches two prior copyright holders, not one. Epsilon's own `CREDITS.md` already records both.

## Where else this is written down

- [`README.md`](README.md) — the index of all six references and what was taken from each.
- [`TUI-DECISION.md`](TUI-DECISION.md) — why Ratatui stays, with codewhale-ratatui named as a
  component kit **on** Ratatui rather than a competing stack.
- [`../../CREDITS.md`](../../CREDITS.md) — the attribution table, with both copyright lines.
- [`../../agents/memory/references-five.md`](../../agents/memory/references-five.md) — the working
  note: what to take, what not to take, and which of its own mechanisms have no production caller.
