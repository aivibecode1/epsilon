# Working note — the desktop IDE, and the shape it forced

**Status: M233 landed the shell; the wiring slice is next.** ADR-020 (toolkit) and ADR-021
(composition crate) are the two decisions everything else hangs off.

## What exists now

| Piece | Where | State |
|---|---|---|
| Native window | `crates/epsilon-desktop` | opens, renders, closes — smoke-tested |
| Geometry | `src/layout.rs` | pure fn; overlap + degradation tests |
| Palette | `src/theme.rs` | adapted from the terminal's roles, with contrast floors |
| Frame | `src/app.rs` | chrome + four panes; **no agent** |
| Composition | `crates/epsilon-runtime` | `runtime.rs` + `build_runtime`, public |

## The toolkit decision, in one paragraph

egui/eframe 0.36.2, measured against five. Two findings decided it and both came from
reading manifests rather than changelogs: **egui shapes text** (`harfrust`, a HarfBuzz port,
landed in 0.35 — the "no shaping" objection is stale), and **egui's tree has no
`bindgen`/`clang-sys`/`embed-resource`**, which on this machine is not a footnote.
Traps, for the next person who re-runs the search: Dioxus desktop resolves to `wry` +
`webview2-com` (WebView2 — it would have violated the "not a web app" rule quietly);
Slint pulls `skia-safe` + `bindgen` in *every* desktop config and is GPL-3.0-or-royalty;
iced's last release is ten months old. gpui is the better product and the worse
dependency — stale published crate, clang build deps, hard `cocoa =` pin, no a11y.

**Two costs accepted on purpose:** MSRV rises to 1.95 for this crate alone (declared in its
own manifest, CI's 1.88 job excludes it), and egui has **no bidi**.

## The rule to keep

> **This crate owns pixels. It does not own agent behaviour.**

If the desktop needs behaviour the composition does not offer, it changes
`epsilon-runtime` — it does not write its own copy. That is what ADR-021 is for, and it is
the same lesson as ADR-012's `global_dir`.

## Next slice, in order

1. **Wire the agent.** `epsilon_runtime::build_runtime` with a `ChannelApprovalUi`, exactly
   as `tui_mode` does. The one design question to settle first: `AgentEvent` is a
   `tokio::broadcast`, and the egui frame is not async. Forward on a runtime task into an
   `mpsc::Sender`, hold a cloned `egui::Context`, and call `request_repaint()` from the
   forwarder. Never block the render thread on the broadcast.
2. **Transcript + composer**, from the same `FrameSummary` discipline: assert on what the
   frame *drew*, not on state that ought to produce a frame.
3. **Approvals** — composer-adjacent, two outcomes, Escape rejects (DSH's placement).
4. **File tree**, virtualized from the first version. Then the editor.

## Traps paid for

- **`Context::run_ui` returns a `FullOutput` whose `TexturesDelta` panics in `Drop`** if
  dropped unapplied. A test harness that throws the frame away is not measuring the frame;
  six failures had that message.
- **`egui::Context::painter()` is gone** — paint through a `&Ui`. `ctx.content_rect()`,
  not `screen_rect()`, is the area safe for content.
- **`ratio(bg, WHITE) > 1.0` does not mean "dark background."** `#FAFAF9` scores ~1.005,
  so it is called dark. Use `luminance`.
- **Write files with `[System.IO.File]::WriteAllText` + explicit `\n`.** `WriteAllLines`
  writes CRLF on Windows and `check-line-endings` will name the file — it did, on the M232
  author, one command after they ran it.
- **`mix` rounds per step**, so a 1% step over a 26-unit channel never leaves 224. A test
  fixture built by repeated 1% mixing silently never reached its own precondition.

## Measurement still owed

Nothing here has been measured through wgpu. No screenshot, no text-throughput number
under `vello_cpu` (egui rasterizes text on the CPU since 0.34). ADR-020 names that as the
open risk and it is still open.

## Open: a flake in the terminal integration test

`m233` attempt 1 failed `typing_still_works_while_a_turn_is_streaming`; attempt 2 passed the
same tree (`m233-r2`, 1,683 tests / 71 binaries). The captured screen showed the typed text
**arriving and truncated** -- `typed while stre` for `typed while streaming` -- on a turn that
was in flight. Not caused by M233 (it touches the header label, not the composer), but
**not explained either**, and "it passed the retry" is not an answer.

Next move: run that single test ten times in isolation and count. If it flakes alone, the
TUI drops keystrokes under streaming load -- i.e. the reported freeze, reproduced, and worth
more than the desktop IDE. `type_text` sends a char every 30 ms while deltas also arrive every
30 ms; a full input queue drops from the end, which is the shape observed.