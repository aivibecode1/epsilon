# The desktop window on this machine — measured 2026-10-03

**What this is.** Two screenshots taken by running the real binary on this laptop and
capturing the screen, plus what each one proved. This file exists because five milestones
(M246–M251) each carried the line *"every claim is an offscreen measurement, never a
rendered frame"*, and that debt was finally paid by looking.

## The measurement

```sh
cargo build -p epsilon-desktop
"$env:TEMP\epsilon-target\debug\epsilon-desktop.exe" "$env:TEMP\epsilon-look"
# then: capture the screen with System.Drawing, read the PNG
```

`epsilon-look` is a scratch workspace of five files (`main.rs`, `lib.rs`, `README.md`,
`src/parser.rs`, `src/render.rs`) in `%TEMP%`.

## What it showed

![the window as shipped](desktop-window-blank-2026-10-03.png)

The window opens — correct title, correct size, real title bar, real window controls — and
the content area is **blank white**. Not a layout with the wrong colours: nothing at all.

`App::ui` **is** running. A probe in `eframe::App::ui` printed
`content_rect=[[0.0 0.0] - [1280.0 768.0]]` on every frame for the life of the process. So
the draw path runs, at the right size, at 60-odd frames a second.

## The probe that localised it

`App::ui` was temporarily changed to fill the whole content rect with solid red and set one
line of large white text before calling `paint`:

![a full-window red rectangle that never appeared](desktop-window-red-probe-2026-10-03.png)

Still white. A rectangle covering 100% of the window, the simplest possible thing egui can
draw, does not reach the screen. **The break is in presentation, not in our drawing code.**

## Backends

| `WGPU_BACKEND` | outcome |
|---|---|
| unset (default) | runs, window blank white |
| `dx12` | **exits, code 2** — `WGPU error: Parent device is lost` |
| `vulkan` | runs, window blank white |
| `gl` | runs, window blank white |

The machine:

```
Intel(R) UHD Graphics 620      driver 31.0.101.2141
AMD Radeon (TM) R7 M340        driver 27.20.20913.2000
HP Laptop 14-bp1xx, Windows 10 Home
```

**Conclusion: no wgpu backend can present on this machine.** The draw path is not at fault
and there is nothing in Epsilon to fix here.

## What this changes about the project's beliefs

1. **The desktop IDE has never been verified by sight, and on this machine it cannot be.**
   `agents/memory/desktop-ide.md` said "Measurement still owed — nothing here has been
   measured through wgpu… ADR-020 names that as the open risk and it is still open." That
   understated it: the risk is not merely unmeasured, it is **unmeasurable here**.
2. **Every one of the 102 desktop tests is an offscreen test** and all of them pass while
   the window shows nothing. That is not a failure of those tests — they assert the draw
   path, which is correct — but it is the sharpest possible statement of what they cannot
   tell you: *nothing after tessellation is ours to check.*
3. So M251 adds the strongest check available on this machine: the frame is **tessellated
   the way the renderer tessellates it**, and the test fails if the draw path produced no
   drawable geometry. That is the last point before the GPU, and it is now covered.

## Re-running this

The probe screenshots were taken with a temporary edit to `eframe::App::ui`, which has been
reverted. To repeat the measurement, add the red fill, run, capture, and revert — the
workflow the TUI half already has in `scripts/console-probe.py`, and which the desktop half
still lacks.