# Working note — the five reference harnesses, and what each is actually for

**Status: five references on disk. Read this before studying any of them again.** Two of
the five are not what their name suggests, and one of them has the same defect this
repository spends nine milestones fixing.

| Reference | What it actually is | Use it for |
|---|---|---|
| `pi-0.85.1` | The port target. A full agent harness in TypeScript. | Behavioural parity. `audit/02-port-fidelity.md` is the measured scoreboard. |
| `deepseek-harness-dsh-v0.2.0-rc.1` | A full application (web + headless + SDK profiles) and the **closest thing to the IDE experience the project is aiming at**. | The goal domain, the trajectory/inspector view, tool-declared render intents, the KV-cache-preserving compaction shape. |
| `opencode-1.18.31` | A full application. | Permission model (boundary escalation, `doom_loop`), keymap-as-footer, diff viewer with reviewed-state, shadow-git snapshots. |
| `hermes-agent-2026.9.14` | A full application. | **Goal gates** (the best answer found anywhere to "how does an agent know it is finished"), context breakdown with provenance, usage/rate-limit diagnostics. |
| `codewhale-ratatui-main` | **A component library, not an application.** No `src/main.rs`, no `[[bin]]`, zero async, zero threads. App-shaped code lives in `examples/`. | Ratatui **mechanisms**: role-based golden frames, machine-checked frame rules, the redraw scheduler, approval arm-after-paint, text safety. **Not** an event loop, **not** scrollback. |

## `codewhale-ratatui`: what to take, and what not to

Verified by two independent reads. `DESIGN.md:81-86` states the host owns events,
permissions, queue mutation, Markdown, clipboard, persistence **and the event loop**; the
library owns painting and local geometry.

**Take, in order — but read "what we already have" below first. Items 1, 2 and 5 turned out
to be implemented or superseded when checked, and porting them would have been duplication.**

1. ~~**Role-based golden frames.**~~ **We have this.** `crates/epsilon-tui/src/app/golden.rs`
   drives the real `draw` through a `TestBackend` and asserts on the assembled screen. We
   differ deliberately: those tests assert on **structure, not pixel-exact text**, because a
   golden test pinning every character gets rewritten on every copy change without being
   read. Their lesson applies to us too.
2. **Design rules machine-checked over the rendered frame.** Not adopted; our golden tests
   already assert frame structure at every size.
3. **An automated version of our M196 defect.** `every_paint_impl_has_a_gallery_entry`
   greps every `Paint` impl and fails if no test drives it. **Still worth adopting** — ours
   has no such guard, and M196 was a feature drawn inside another feature's renderer, fully
   tested, never rendered.
4. **`FrameBudget`** (`src/components/motion.rs:489-534`) — every animated component
   *requests when it next needs a frame*; `next_frame_in() -> Option<Duration>` where
   **`None` means sleep until an event**. Idle costs zero redraws. This is the cleanest
   answer to the unreproduced TUI freeze: `None` vs `Some(166ms)` is exactly the knob that
   separates "slow" from "lost input". **Purely inferred that we need it — our redraw
   cadence has never been measured. Measure before adopting.**
5. ~~**`ApprovalState::unarmed()` / `.arm()`**~~ **We have this, and stricter.**
   `handle_approval_key` compares the **specific ask id** against `approval_painted_id`
   and refuses otherwise — not merely "has this dialog been drawn once", but "is the
   dialog on screen the same ask these keys may decide". A FIFO channel parks parallel tool
   calls, so this matters.
6. **`src/text.rs` (152 tested lines)** — **the one clear gap, and M260 closed half of it.**
   `display_safe` strips bidi overrides, which ratatui 0.30 passes through. We had **zero**
   occurrences of `202E`/`2066`/`bidi`/`display_safe` in the workspace. **Not yet closed:**
   the approval *dialog* renders from `pending_ask`, not from a transcript line, so it does
   not pass through `push_line` and is not stripped — and that is the worst place for it.
   Still open: grapheme-safe truncation (we use `.chars().take(n)` at four sites).
7. **`grounds_differ(a, b)`** (`theme.rs:385-387`) — one predicate that makes panel
   elevation degrade from a fill to an edge wherever the fill is invisible.

**Do NOT take:**

- **`Keymap<A>` as-is** (`keymap.rs`) — the *design* is the best idea in the repo (one
  table that is simultaneously the dispatch table and the rendered footer, so `/hotkeys`
  cannot drift), but **`Keymap::lookup` has no production caller there.** Verified by grep:
  hits only in its own tests and in `gallery/hints.rs`, which uses it for *hints only*. So
  the mechanism is a port of the upstream engine's dispatch and this repo carried the
  rendering half. **That is the "faithful port, no production caller" class, in a reference
  we are reading for ideas — take the design, and wire it to our real dispatcher or not at
  all.**
- **Its transcript's rendering shape.** It re-wraps the whole document every frame, and
  `height()` does it a second time (`transcript.rs:896-928`). That is precisely the shape
  we measured and fixed at M198 (144µs → 276ns). `List::place` is the correct contrast.
- **Scrollback / follow-tail.** It has none: `Transcript.offset` is a host-supplied field
  and there is no `follow_tail` anywhere. Do not take scrollback design from here.
- **Its benchmarks as a gate.** Hand-rolled, dependency-free, prints median + p95, and has
  **no** allocation assertion, **no** diff-size check and **no** CI threshold. A printed
  report. Its one useful fact is that paint cost is the same at 100 rows and 10,000 rows,
  which is a claim about O(visible).
- **~40% of the crate.** `whale.rs`, `whale_motion/`, `ocean.rs`, `ombre.rs`, `habitat*`
  and ~4 MB of SVG/GIF assets are brand identity. `DESIGN.md:54-56` disclaims the optional
  compositions as *"not substitutes for the TUI baseline."*

## The rule this note exists to enforce

**A reference is a claim about a different program.** Three of the five differ from their
first impression (a port target that is a whole harness, an app that is a library), and the
most immediately useful file in the newest one is a *control for its own test harness*.
Read for mechanisms; verify the wiring before believing it.
