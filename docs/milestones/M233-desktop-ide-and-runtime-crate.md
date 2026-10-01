# M233 — a native desktop IDE, and the composition crate it needed first

The project stops being a port with a terminal frontend. Two ADRs ([ADR-020](../../docs/design/epsilon-decisions.md),
[ADR-021](../../docs/design/epsilon-decisions.md)), two new crates, and the first window that is not a
terminal.

## What this is

`crates/epsilon-desktop` — a real desktop application in an operating-system window, rendering
natively through winit + wgpu. Not Electron, not Tauri-with-HTML, not a local server and a browser
tab. The requirement was explicit and it ruled out the obvious candidates by measurement rather
than by taste:

| toolkit | native? | licence | accessibility | build-host deps | outcome |
|---|---|---|---|---|---|
| **egui 0.36.2** | winit + wgpu | MIT **or** Apache-2.0 | AccessKit in default features | **none** | chosen |
| gpui | yes (Blade/D3D12) | Apache-2.0 | none in the published crate | `bindgen`, `cbindgen`, `embed-resource` | second best |
| iced 0.14 | yes | MIT | none | none | 10-month release gap, ~96% one maintainer |
| Dioxus desktop | **no** — `wry` + `webview2-com` | Apache/MIT | — | — | violates the constraint outright |
| Slint 1.18 | yes | **GPL-3.0 / royalty** | present | **`skia-safe` + `bindgen` in every desktop config** | licence *and* build weight |

Two findings changed the decision. First, the objection that motivates choosing against egui —
"it shapes no text" — **stopped being true at 0.35**, when `harfrust` (a HarfBuzz port) replaced
`ab_glyph` for kerning and ligatures; it is verified from the manifest at each tag, not from the
changelog's word. Second, and decisive on this machine, **egui's resolved dependency tree contains
no `bindgen`, `clang-sys` or `embed-resource`** — every build failure this repository has ever
recorded is an MSVC toolchain problem, and a toolkit that needs clang would be a fifth way to hit
them. Slint needs clang *and* compiles Skia from source in every desktop configuration resolved.

DSH's own "desktop" is an Electron shell pointing a `BrowserWindow` at a Node child running the
same web profile (`references/deepseek-harness-dsh-v0.2.0-rc.1/apps/desktop/src/main.ts:408,421`).
That is a browser. Its *workflow* is what is worth taking here; its carrier is what was ruled out.

## What had to be built first: `epsilon-runtime`

The blocker was not graphical. Exactly one function in this repository knew how to turn a workspace
plus a configuration into a running agent: `build_runtime`, a private `async fn` in a **binary**,
beside 3,199 lines of helpers in `crates/epsilon-cli/src/runtime.rs`.

A second frontend in a second crate cannot reach it. Reimplementing it would mean copying skill
roots, trust gating, permission layers, resume ordering, the lease sweep, the migration and repair
passes and the extension event forwarder — two copies of a rule that cannot disagree visibly, which
is precisely the defect ADR-012's `global_dir` amendment exists to close.

So `runtime.rs` moved verbatim to `crates/epsilon-runtime/src/lib.rs` and `build_runtime` moved
with it as a public function. **Nothing about it changed but its visibility.** `epsilon-cli`'s
library re-exports it (`pub use epsilon_runtime as runtime;`), so all 35 existing
`epsilon_cli::runtime::…` call sites are unchanged and the move is one commit that *adds* a crate
rather than one that rewrites thirty call sites.

`Resume`, `session_path_for`, `resumed_model_to_use`, the lease-wait notice and their tests moved
with it, because they are part of the same rule.

## The layout, and the three bugs painting frames found

`layout.rs` computes the frame's geometry as a pure function of `(available rect, preferences)` —
so every degradation rule is a test rather than something found on a user's screen. Painting
frames at ten widths and six heights, and asserting that **no two drawn regions overlap**, found
three real defects that reading the arithmetic had not:

1. **The title bar overlapped the activity bar at every ordinary window size.** The body started
   `GUTTER` below the *window's* top edge whether or not a title was drawn there.
2. **A non-positive window rect produced a negative-width region.** A window can report zero (a
   monitor unplugged mid-drag on Windows) and some maximized configurations report negative.
3. **The editor pane never grew.** `clamp(AGENT_MIN, spare - CENTER_MIN)` is the agent panel's
   *maximum*, not its share — so widening the window widened the transcript and left the file view
   at a fixed 328 points. The test named for the property ("widening the window widens the file
   view") was the thing that found it.

A fourth came from the frame harness itself: egui panics in `Drop` when a `TexturesDelta` goes away
unapplied, which produced six failures whose message was about textures and whose content was
"the harness throws away what the frame produced".

## Contrast, measured rather than chosen

The palette is **adapted from the terminal's semantic roles**, not re-invented: two frontends with
two palettes look like two products. But the *floors* are the desktop's, because the rendering
differs, and adapting them naively failed two tests:

- the light theme's `surface_pending` is `#FFFFFF` on a `#FAFAF9` window — **1.04:1**, a band that
  is not a band;
- the dark theme's quiet role sat at **2.98:1**, just under the 3:1 floor.

`IdeTheme::separated_from_background` now moves every role away from the background until it clears
the right floor — `TEXT_FLOOR` 3.0 for anything carrying glyphs, `SURFACE_FLOOR` 1.12 for anything
that is a fill — in twenty linear steps toward white or black, so the hue survives. The first
version of that helper classified "is this background dark?" by asking whether its contrast ratio
against white exceeded 1.0, which calls `#FAFAF9` **dark** and pushes the light palette's bands
further from the page. Relative luminance is the primitive; the test now anchors the ratio function
against WCAG's own worked examples (black on white is 21:1; `#767676` on white is ≈4.54:1).

## What is deliberately not here

Slice 1 is chrome, geometry and a palette. It draws the workspace name and says **"no agent
attached"** rather than inventing a model, a cost or a context figure — M233-in-the-TUI (a header
naming `DeepSeek V3.1` over a faux turn) is the same class of lie, and a desktop app is judged by
how it looks, which is exactly where a plausible fake is most tempting. The wiring slice composes
the agent through `epsilon_runtime::build_runtime`, exactly as the terminal does.

## Verification

**Battery `m233-r2` — GATE PASS, 1,683 tests across 71 binaries, 0 failed.** Attempt 1
(`m233`) failed on one test and attempt 2 passed; both logs exist. See the note on that
below rather than reading the PASS as "it was fine the first time".

- `cargo clippy --workspace --all-targets -- -D warnings` — clean, 13 crates.
- `cargo test -p epsilon-desktop` — **26 tests**, covering the overlap property across ten widths
  and six heights, every degradation threshold, the palette floors in both modes, and the frame
  assertions above.
- check-links 566/566, check-line-endings 817 files 0 CRLF, lint tables 13 + 1, `cargo fmt --check`
  clean — all re-run against the final tree after the documentation edits.
- The built binary was launched against this repository and stayed up with empty stderr: the window
  opens and the renderer initialises on this machine. That is a smoke test, not a rendering
  measurement.

### The failure that is worth writing down

Attempt 1 failed `typing_still_works_while_a_turn_is_streaming` — the terminal integration
test that exists precisely to catch the reported freeze. The screen it captured showed the
typed text **arriving and truncated**: `❯ typed while stre` where the test typed
`typed while streaming`. Five characters were missing, from the end, on a turn that was
genuinely in flight. Attempt 2 passed the same tree.

So it is a flake, not a regression, and specifically **not** one this milestone caused: the
change in `view.rs` touches the header's model label and nothing in the composer's width or
the editor's insert path. But "it passed the second time" is exactly the reasoning
`AGENTS.md` warns against, so the honest reading is: **the mechanism is not yet understood,
and the test is not yet reliable.** It is recorded here as an open item rather than as a
resolved one.

The two candidate mechanisms, neither measured yet:

1. **Keystrokes lost under streaming load.** `type_text` sends one character every 30 ms
   while deltas also arrive every 30 ms. If the harness's input queue fills, characters go
   from the end — which is the shape that was observed.
2. **A cross-test race.** The eight terminal tests each open their own pseudo-console;
   `gate-until-pass` stopped specifically to rule this out, and the retry's fingerprint was
   the same test name, which is consistent with both.

The next thing to do is not another battery. It is to run that one test ten times in
isolation and count. If it flakes alone, it is (1) and the TUI's input path is dropping keys
under load — which would be the reported freeze, finally reproduced. **Until that is measured,
neither story is the answer.**

## What nothing exercises

- **The window's own chrome.** `eframe` owns the title bar and the frame; nothing here asserts on
  it.
- **Real GPU rendering.** Every frame in this milestone was painted into an off-screen context and
  asserted on as *text and geometry*. Nothing has been measured through wgpu — no screenshot, no
  text-throughput benchmark. The ADR names CPU text rasterization (`vello_cpu`) as the open risk
  and this milestone does not discharge it.
- **Input.** No key has been pressed in this window yet, so nothing exercises focus, caret or
  shortcuts.
- **The window on a second monitor**, or under fractional DPI, or maximised: egui's tracker has
  open issues for `egui_winit` on non-100% scale factors and for text across mixed-DPI monitors.
- **Any agent behaviour.** The window has no transcript and no turn.
