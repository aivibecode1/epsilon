# Working note — the desktop IDE, and the shape it forced

**Status: M233 landed the shell; the wiring slice is next.** ADR-020 (toolkit) and ADR-021
(composition crate) are the two decisions everything else hangs off.

## What exists now

| Piece | Where | State |
|---|---|---|
| Native window | `crates/epsilon-desktop` | opens, renders, closes — smoke-tested |
| Geometry | `src/layout.rs` | pure fn; overlap + degradation tests |
| Palette | `src/theme.rs` | adapted from the terminal's roles, with contrast floors |
| Composition | `crates/epsilon-runtime` | `resolve_launch` + `build_runtime`, both public |
| The agent | `src/session.rs` | `IdeSession`, `Transcript`, the event bridge |
| The frame | `src/app.rs` | chrome, panes, status, transcript, approvals, composer |

**M235 wired the agent in.** An end-to-end test composes a real offline agent, sends a turn,
waits for a permission ask to reach the screen, answers it, and waits for the turn to finish.
It found four defects that no state-level test could have: `Arc::get_mut` silently stopping
the drain, a reducer that depended on a `MessageStart` the loop does not always emit, a
composer that never took the keyboard, and Enter consumed before the frame's characters were
ingested.

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

1. **The file tree and the editor.** They draw placeholders now. Both need virtualization
   from the first version — `ScrollArea::show_rows`, never a `Tree` over a whole repository.
2. **Diff review**, which the agent needs more than the user does: a tool call that edits a
   file should show the change, in the pane, before the next tool runs.
3. **`App::logic` instead of `ui` for draining.** eframe skips the whole `ui` pass while the
   window is hidden, so a run that finished in the background produces no `AgentEnd` on
   screen until the window is shown. That belongs in `logic`.
4. **Markdown in the transcript.** `pulldown-cmark` is already in the tree via the TUI.

## Open: keystrokes into the composer are not verified

`crates/epsilon-desktop/tests/window.rs` sets the draft and sends it through `set_draft` /
`send_draft` — the same functions the composer calls — but the route from an egui
`Event::Text` into the draft is **not** driven. Four attempts through `RawInput.events` failed
to move a character. Ruled out already: focus is requested unconditionally, the composer's
widget id is stable, `RawInput::focused` is set true, and the events use the documented
`Event::Text(String)` shape.

Next to try: a two-frame sequence (focus on one frame, type on the next) against a context
that has been tessellated, or drive the real `eframe` window. Guessing again would be the
fourth theory in a row.

## Traps paid for

- **Never `Remove-Item -Recurse` inside a cargo target directory.** Deleting
  `%TEMP%\epsilon-target\debug\incremental` on this box freed 18 GB and took the rest of
  `debug/` with it — `deps/` was down to 12 files and the next build failed with a wall of
  `extern location for mio does not exist`. Use `cargo clean`, which knows the layout, or
  delete nothing and let the space go. A cold rebuild with `CARGO_PROFILE_*_DEBUG=0` is
  cheaper than the hours this costs.
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

## Open: load-dependent input behaviour in the TUI (is this the reported freeze?)

`m233` attempt 1 failed `typing_still_works_while_a_turn_is_streaming`; attempt 2 passed the
same tree (`m233-r2`, 1,683 tests / 71 binaries). The captured screen showed the typed text
**arriving and truncated** — `typed while stre` for `typed while streaming` — on a turn that
was in flight.

**Measured after the battery, which narrows it a lot:**

| condition | result |
|---|---|
| the test alone, 8 runs | 8 pass, 0 fail |
| the whole `terminal.rs` binary, default parallelism, 4 runs | 4 pass, 0 fail |
| the full workspace battery | 1 fail, 1 pass |

So it needs **seventy other test binaries' worth of load**. Not a cross-test race between the
eight terminal tests; not this milestone's change (a header label cannot reach the composer's
width). It is load-dependent behaviour in the shipped TUI — the family the owner has been
reporting across four audits.

**Latency vs loss is undecided, and that is the whole question.** Five missing characters is
about 150 ms of extra delay: the right magnitude for a starved process, the wrong magnitude for
a lost keystroke.

Next experiment: **sample the screen every 50 ms for two seconds after the typing burst.**
Growing → latency. Stopping at 17 → input loss. Fifteen minutes, and it is the first
measurement that puts the reported freeze within reach (`audit/04-tui.md` records that it has
never been reproduced).

**Do not** raise the 800 ms settle, and do not add `--test-threads=1`. Both make the battery
green and answer nothing.

If it turns out to be loss, the prime suspect is already located in the code:
`crates/epsilon-tui/src/app/event.rs:142-153` wraps `term_events.next()` in
`tokio::time::timeout(Duration::ZERO, …)`. A zero timeout **drops the future** when the stream
is not immediately ready, and crossterm's `EventStream` carries one-shot wake state
(`stream_wake_task_executed`) that a dropped poll leaves armed — see
`crossterm-0.29.0/src/event/stream.rs:99-137` and the blocking thread it spawns at `:47-61`.
