# M129 — OSC 52 for `/copy`, the glyph fallback that was only promised, and a real card duration

The TUI audit's last two items, plus the independent review's sharpest blocker.

## OSC 52 — `/copy` stopped lying over SSH

`/copy` used `arboard` alone. Over SSH — or in any terminal without a local display server — it printed
**a success notice while copying nothing**, which is the worst shape of failure: the user believes their
text is on a clipboard that does not exist. A new `crates/epsilon-tui/src/clipboard.rs` emits
`ESC ] 52 ; c ; <base64> BEL` as well, so the terminal can put the text on the clipboard of whoever is
looking at the screen.

Decisions, each of which had a plausible alternative:

- **Native first, terminal second** — and on a local terminal with a working clipboard the sequence is
  deliberately **not** emitted, because a redundant OSC 52 races the native write for the same
  clipboard.
- **The notice says "sent via the terminal", not "copied".** OSC 52 is a *request*: it is fire-and-forget
  over the same pty as the frame, and a terminal that ignores it (iTerm2 ships it off) says nothing. Only
  the native path claims the text is on a clipboard.
- **Refused, not truncated, past pi's `MAX_ENCODED = 100_000`** — a silently shortened payload is a
  wrong paste.
- **Written at the frame boundary**: `/copy` parks the sequence on `App::osc_pending` and the event loop
  writes it through `terminal.backend_mut()` *after* `terminal.draw` returns, so it can never land inside
  a frame or be emitted from a key handler.
- **`base64` accepted**, because OSC 52 has a normative alphabet and hand-rolling the encoder risks the
  silent-encoding bug whose failure mode here is exactly "copies nothing, reports success". It was
  already a workspace dependency at `=0.23.1` (M43), so the direct edge cost one line of `Cargo.lock`
  and no new build.

## The `NO_COLOR` fallback that was only promised

`theme.rs:744` said *"widgets that rely on a band for legibility must fall back to a gutter glyph when
this is false"* — and **no such branch existed**. All seven `LineKind`s returned the same glyph in every
mode, so with `NO_COLOR` an answer and the reasoning trace behind it were **the same pixels**.

The old test passed because it asserted a hardcoded `" "` and `"|"` rather than *distinctness* — a test
checking the glyph table instead of the frame. That is the M123 failure mode for a third time, and the
reason this crate now insists on reading cells from `render_buffer` → `draw`.

```
before                        after
03|  an assistant answer  →   03|* an assistant answer
05|| weighing the options →   05|= weighing the options
```

Unicode `* answer` / `≡ reasoning`; ASCII `*` / `=`. Every role now has a glyph of its own in every mode.

## The card duration (review blocker B1)

Every result card reported `0ms`, because `state.rs` stamped its start *and* end with the same `now` —
while the slice's own comment reasoned that `0s` "reads as a field that does not work". `settle_call` now
returns the pending card's `started_at_ms` and the result inherits it, with a documented fallback for a
result whose call is not in the transcript.

## Verification

Battery `m129c`: **all five gates exit 0, 1,165 passed / 0 failed / 1 ignored across 61 binaries** —
seventeen more than M128's 1,148. `cargo test -p epsilon-tui` 300 passed / 0 failed / 0 ignored;
`cargo clippy -p epsilon-tui --all-targets -- -D warnings` clean.

Both items carry negative controls: reverting `row_style`'s two branches fails
`the_bandless_gutter_is_the_fallback_the_theme_documents` ("the fallback must be a different glyph, not
the same one") and the frame test; reverting `decide()`'s trigger to the pre-fix `if !local` fails
`a_remote_session_sends_the_sequence_even_when_the_native_write_worked`.

**Three battery attempts, and the middle one is a lesson.** `m129a` failed clippy on a `let…else` in my
own B1 fix. `m129b` failed on a clipboard test — and that was **my battery reading the tree while the
delegated author had that same test's negative control armed in place**. A control is
indistinguishable from a bug to anyone reading the tree while it is armed, so the defence is that
controls stay short-lived and announced. I recorded it first as an "environment-sensitive flake", which
was wrong: that test reads no environment (`remote` is a parameter of `decide()`, which is exactly why
it is).

## Limits

- **No OSC 52 read path.** Reading the clipboard back needs a round-trip a terminal may ignore, i.e. a
  paste that hangs instead of failing; `Ctrl+V` stays on `arboard`, documented in `keys.rs`.
- **The gutter glyph and the row's own body marker still repeat** for several roles (`> > hello`,
  `* * run finished`). Deliberate and test-pinned; consolidating it is a layout decision, not a role
  separation bug.
- **`Pending|Running` share a glyph** and `Failed|Denied` share one too — the *word* carries the
  distinction, documented as deliberate.
- The audit's citations about `has_surfaces()` are now true; nothing else in that file was audited.
