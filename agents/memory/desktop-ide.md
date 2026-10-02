# Working note — the desktop IDE, and the shape it forced

**Status: M248 landed the tab strip and six frame-level defects — the ones a state test
cannot see.** The wiring slice before it is in; what is next is listed below. ADR-020
(toolkit) and ADR-021 (composition crate) are the two decisions everything else hangs off.

## M248: measure the frame, because reading it was not enough

The suite had 103 tests and every one of them passed while the **conversation was painted
on top of the file tree**. `paint()` shared one `Ui` across four panes; a `Ui`'s cursor
walks, so each pane started where the last stopped. Measured with a scratch probe before
the fix: agent pane `[920,42]-[1440,870]`, transcript laid out at `[0,142]-[504,817]`.

`layout.rs`'s non-overlap sweep kept passing throughout — **the panes never overlapped,
only their contents did.** That is the generalisation worth keeping: a geometry test on the
containers says nothing about the widgets inside them.

Now every pane gets a `Ui` of its own (`pane_ui`), and `FrameSummary` grew **`row_rects`**,
because `rows` records *what* was drawn and nothing recorded *where*. Expect the same class
of bug anywhere a container is tested and its contents are not.

Also in M248: `approve_pending` had **no caller in the crate** — the window painted
`[enter] allow [esc] refuse` and the composer ate both keys. Sixth instance of the
"faithful port, no production caller" class, and the second in this crate.

**Verified by battery `m248b`: 1,804 tests / 74 binaries, 0 failed.** Attempt 1 failed on a
new test that painted a frame and *then* checked whether a gate was waiting — it passed
alone and failed under load. Check-then-act, in the test.

## M249: the window was a snapshot, and so was the keyboard

Two more "it worked, in the pane that says it worked". `FileTree::load` ran once at launch
and the editor held the file as it was at open, so a file the agent **created** was absent
from the tree, one it **edited** showed the text the edit replaced, and one it **deleted**
stayed on screen presenting itself as a file. `push_change` already recorded every changed
path — the signal existed and had no consumer. It has one now (`take_changed_paths`), spent
by `refresh_changed_files`, which re-walks the tree and re-reads **the open file only if its
own path is among those changed** — which is what makes a vanished file trustworthy enough
to close and announce.

The composer called `request_focus()` every frame on the stated ground that this window had
one text input. The file filter is the second one, so the filter took focus and had it
stolen back on the next frame. The exception the old comment promised is now the code, and
`Escape` brings the keyboard home — a reader can get back to the composer from the keyboard,
which is the difference between a state and a trap.

**Battery `m249-r2`: 1,808 tests / 74 binaries, 0 failed.** Attempt 1 failed on
`check-line-endings`: the six Ponytail `SKILL.md` files M248 added had CRLF in the working
copy. Git normalised the *commits* (blob hash == HEAD hash, all LF), so the damage was
confined to the checkout — and it still took a gate to find it. **A passing battery
validates the source it compiled; it says nothing about the markdown it also checked.**

### Known open risk from M249, untested — **resolved by measurement in M250**

egui gives a clicked `Button` keyboard focus. The composer now yields to whatever holds
focus, so **clicking a tab may leave the keyboard on that button.** Untested, and probably
a defect. The likely fix is to give the filter a stable id like the composer's and yield
only to text inputs.

> **Measured in M250: it is not real.** Both clickable kinds — a tree row (a bare
> `allocate_exact_size`) and a tab (a real `egui::Button`) — leave the keyboard with the
> composer. Both are now asserted, so it is a guard rather than a note.

## M250: the review, and a green test about the wrong control

The transcript said *"… N more changed lines — open the file"* and **nothing was openable**:
no clickable row, no diff pane. `layout.rs` had called the centre pane *"The editor or
diff"* since M233 and it had never been one. A changed row is now a link; clicking it shows
the whole change in the editor pane, with added/removed in the palette's `success`/`danger`
roles rather than two greys, a coarse change labelled as a block replacement, and the pane's
own clip. Opening any file ends the review — one way back, not two.

**The lesson worth more than the feature.** `clicking_a_tab_switches_the_editor_to_that_file`
had passed since M248 while **never clicking a tab**: `FrameSummary::hotspot` returns the
first match, and the tree records the same id for the same path as the strip does, so it
clicked the tree row — which opens the file, so the assertion passed. Giving tabs their own
`tab:<path>` id exposed it, the honest version **failed**, and the failure was a real defect:
egui hands a widget its click result *while drawing it*, so the strip drew `beta` as current
while the body already showed `alpha` — one frame where the pane contradicts itself.

The fix is the same data read in the opposite order: test the pointer against **last frame's**
rects *before* drawing. M248 read the frame's hotspots *after* the pane that fills them and
always got an empty answer. **When a mechanism returns the answer, check where you read it
relative to where it is produced.**

Battery `m250`: **1,813 tests / 74 binaries, 0 failed**, attempt 1. Clippy 46 s, test 381 s,
demo 149 s — against 1,058/1,696/290 s for the same battery with debug info on.

## M251: the window had two keys

`Enter` and `Escape`, both the composer's. Everything else needed the mouse, and the file
tree — the first pane to vanish as the window narrows — was the only route to a file.
`Ctrl+P` now opens a palette listing every workspace file plus the window's own commands.

**The order lesson, for the third time now.** The palette reads `Enter`/`Escape` **before**
the composer, because the composer takes both unconditionally. That is M250's tab-strip bug
one layer down: *read a key before the widget that wanted it, not after.* Three instances in
two milestones is a pattern, not a coincidence.

Four commands, not thirty — there is no command table in this crate, and working entries
beat aspirational ones. `palette_entries` is the seam to widen when one exists.

**Battery `m251`: 1,817 tests / 74 binaries, 0 failed**, attempt 1.

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

## Measured 2026-10-03 (M252): the window cannot be seen on this machine

Ran the real binary and screenshotted it. **The window opens and the content area is blank
white.** `App::ui` runs every frame with `content_rect=[[0,0]-[1280,768]]`. Filling the
whole window with **solid red** also shows white, so the break is in presentation.

| `WGPU_BACKEND` | outcome |
|---|---|
| unset | runs, blank white |
| `dx12` | exits 2 — `WGPU error: Parent device is lost` |
| `vulkan` / `gl` | runs, blank white |

This box is a 2018 HP laptop, UHD 620 + R7 M340, 2018/2019 drivers. **No wgpu backend
presents here.** Not our drawing code; nothing in Epsilon to fix.

**What it changes.** Every one of the 102 desktop tests is an offscreen test and all pass
while the window shows nothing — they assert the draw path, which is correct. The limit is
precise: **nothing after tessellation is ours to check**, and until M252 nothing before it
was checked either, only recorded. `a_frame_tessellates_into_geometry_a_renderer_could_draw`
now covers the last point before the GPU.

Evidence, with both screenshots:
[`audit/evidence/desktop-window-measurement-2026-10-03.md`](../../audit/evidence/desktop-window-measurement-2026-10-03.md).

**The desktop half still has no `console-probe.py`.** The TUI can be driven and captured
from a script; the desktop needed a temporary source edit and a hand-written capture. That
asymmetry is why this took five milestones to happen, and building the probe is the next
thing that would actually move the needle on verification.

## M258: the window had no goal at all

`grep -i goal` over `crates/epsilon-desktop` returned **one hit**, and it was a doc comment:
`layout.rs:143` promised *"the bottom bar: goal phase, pending approvals, key hints"* — never
built. The terminal has had `/goal`, gates, rounds and gate-aware continuation since
M240–M257; the window had none of it. Largest asymmetry left in the product.

Now: a **goal band** above the transcript (phase, round budget, gate count, shortened
objective, and `not armed — it will not continue` in words), plus a **round driver** that
runs the shared `epsilon_runtime::begin_goal_round` **off the paint thread** and applies the
answer from a channel. `IdeSession::start` switched to `build_runtime_and_gate_runner` so the
runner shares the agent's sandbox rather than the window resolving a second one.

Two things the tests caught:
- **`tokio::spawn` panics outside a runtime.** The product's window has one; a headless
  paint (and every test) does not. The driver now uses `Handle::try_current()` and gives the
  round back rather than crashing the window.
- **A freshly created goal is *armed*.** Arming happens on creation; it is **resume** that
  disarms (`references/deepseek-harness-dsh-v0.2.0-rc.1/packages/goal/goal/src/types.ts:72`).
  The first unarmed test asserted otherwise and would have passed on a build that never drew
  the state line.

Battery `m258`: **1,842 tests / 76 binaries, 0 failed**, attempt 1.

**Next slice: there is still no way to create, pause or clear a goal from the window** — only
to watch one. That wants the command table, not another hand-written palette entry.

## M259: a command table, so the window can control a goal

M258 let the window *watch* a goal. The palette's four hardcoded entries are now a table of
`Command { id, label, hint, action }` carrying **goal create/pause/resume/clear**, which is
the last thing the terminal's `/goal` has and the window did not.

**The `id` is the point, and it needed a consumer or clippy would be right.** A row records
`command:<id>` in its hotspots, so tests press ids rather than the words on screen. Without
that the field was dead — and deleting it would have lost the seam a future key binding
needs. A label is what a reader sees and therefore what gets reworded.

`goal: create` takes **the palette's own query** as the objective: type it, choose the verb —
one gesture, the same one `/goal <objective>` is. Empty query refuses **in words**.

**A test that passed on a build where the command never ran.**
`goal_verbs_without_an_agent_say_so` asserted *"no agent attached"*, which the agent-less
panel prints as its **empty state**. It passed under the negative control, which is how it
was caught. This is the `FakeHost` trap one layer up: **run the control against the NEW tests
specifically, not as a formality.**

Battery `m259`: **1,847 tests / 76 binaries, 0 failed**, attempt 1.

## M263: the goal band was painted at the window's left edge

`draw_goal_band` took a **width** and built its rect at `pos2(0.0, top)`; `with_clip_rect`
replaces rather than intersects the clip, so the pane's own clip did not contain it either.
Measured with a negative control: the band painted at `[0,97]-[520,131]` while the agent pane
is `[920,42]-[1440,870]` — on the activity bar, the file filter and the first tree rows.

**The lesson is the fixture, not the code.** `app.rs`'s containment sweep iterates every
`row_rects` entry and would have caught this on sight; its fixture seeded **no goal**, so the
band never entered `row_rects`. A green sweep over a fixture that cannot produce the thing you
are worried about is not a green sweep. The band also needed a *real session* to exist, which is
why this lived entirely in `tests/window.rs` and not in the unit tests.

## Next slice, in order

0. **The window has never shown what a tool printed.** `ToolExecutionUpdate` falls into
   `_ => {}` (`session.rs:497`), so `▸ bash {…}` is followed by `✓` and nothing in between —
   **while the resume path does show results** (`:257-268`), so the same session shows two
   different transcripts before and after a session switch. `RowKind::Tool` has no output field.
   The terminal's pattern (M152): `partial` is a **delta**, so keep a bounded tail.
1. **A command table.** The palette reaches four things because that is all the window
   offers. A real registry — **goal create/pause/clear** (M258 made the window *watch* a goal
   but left no way to control one), model picker, sessions, diff review — is the next slice
   and wants an id/label/handler table rather than more palette entries.
2. **Drive the round driver end to end.** The tests drive the answer half and the band; no
   test starts a round from an armed goal and watches it arrive.
3. **Build a desktop probe** — a `scripts/desktop-probe.py` that launches
   `epsilon-desktop`, injects input and captures the window, the way `console-probe.py`
   does for the TUI. M252's measurement needed a temporary source edit and a hand-written
   capture; that is why it took five milestones. Nothing after tessellation is verifiable
   without it (A25).
4. **Reviewed state for changes** (OpenCode): "what is left to look at" is unanswerable.
5. **A filesystem watcher** for the tree, and `App::logic` draining so a run that finishes
   in the background produces an `AgentEnd` while the window is hidden.
6. **The tasks and sessions panes are never measured for containment**; `row_rects` covers
   the transcript and the editor only.
7. **Arrow keys in the palette are implemented and untested.**

**The full parity and defect list is not here**: it is rows A27–A44 of
[`audit/03-defects.md`](../../audit/03-defects.md), written by four parallel read-only reviews
at M262, and [`review-sweep-m262.md`](review-sweep-m262.md) says what to do first.

## egui lessons worth not re-learning

- **Allocate a clickable row, do not position it.** `ui.allocate_exact_size` returns the
  screen rect *and* the response together, so they cannot drift. Positioning by hand cost
  three bugs: a scroll area never pinned to its pane (it started at `(0,0)`), a `show`
  nested inside a `show_rows`, and `next_widget_position` being content-relative.
- **A click is three frames**: `PointerMoved`, `PointerButton{pressed:true}`, then
  `pressed:false`. Four attempts used a one-frame "click" and all four failed.
- **A frame paints top to bottom.** A row added by a widget *below* the transcript cannot
  appear in that frame's output; it appears on the next repaint. Asserting on the frame
  that caused a change is a test of a state the product is never in.
- **`RawInput::focused` defaults to false** and egui drops keyboard events for an unfocused
  window. A default-constructed input is a window nobody can type into.
- **`Context::painter()` is gone**; paint through a `&Ui` or `&mut Ui`.

## Keystrokes: settled, and how

Typing works, and it is tested with real egui input (`RawInput.events`, `focused: true`).
Getting there took **four rounds of changing plausible causes** — focus requests, widget id,
`RawInput::focused`, the `Event::Text` shape — and changing any of them changed nothing.

What ended it: a twenty-line **probe** rendering one bare `TextEdit` off-screen and feeding
it the same events. It received them, *unfocused*, which ruled out all four at once.

The real cause was immediate-mode drawing: an egui frame paints top to bottom, the transcript
is painted **before** the composer, so a row the composer adds during a frame cannot be in
that frame's output. The prompt is on screen on the next repaint.

**When something does not respond to input, probe the toolkit before changing the product.**

## Traps paid for

- **Never `Remove-Item -Recurse` inside a cargo target directory.** Deleting
  `%TEMP%\epsilon-target\debug\incremental` on this box freed 18 GB and took the rest of
  `debug/` with it — `deps/` was down to 12 files and the next build failed with a wall of
  `extern location for mio does not exist`. Use `cargo clean`, which knows the layout, or
  delete nothing and let the space go. A cold rebuild with `CARGO_PROFILE_*_DEBUG=0` is
  cheaper than the hours this costs.
- **A bare `pwsh` session cannot build this repo.** `cargo check` from a fresh shell failed
  in `proc-macro2`'s build script. The fix is gate.ps1's vcvars load; the cheapest
  reproduction in one line is
  `cmd /c '"<vcvars64.bat>" >nul 2>&1 && set "CARGO_BUILD_JOBS=1" && set "CARGO_TARGET_DIR=…" && cargo …'`
  — note `set "VAR=value"`, because `set VAR=value && …` puts the space in the value.
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

## egui 0.36 facts worth not re-learning

- **`RichText` has private fields and no public concatenation.** Mixed styling is built with a
  `LayoutJob` (`job.append(text, 0.0, TextFormat::simple(font, color))`) and handed to a widget
  via `WidgetText::from(job)`. Collecting `RichText`s with `collect()` does not compile.
- **`egui_extras` 0.36 ships only `datepicker` and `loaders`** — there is no `markup` module.
  For inline markdown, build your own or take a dependency; there is nothing to reuse.
- **`ScrollArea::show_rows(ui, row_height, count, …)`** — height before count, the opposite of
  the obvious order.
- **A `Ui` inside `show_rows` is positioned from the scroll area's content**, so a row's
  rectangle must come from `allocate_exact_size` and not from `next_widget_position`.
- **pulldown-cmark's table header cells arrive with no `TableRow` around them**; `End(TableHead)`
  needs the same handling as `End(TableRow)` or the header disappears.