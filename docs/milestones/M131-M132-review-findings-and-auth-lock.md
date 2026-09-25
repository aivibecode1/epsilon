# M131 + M132 — the review's seven findings, and the auth lock that reported a permission error

Two slices that close out the independent review (`audit/evidence/m126-m127-review.md`) and the defect
its own battery run exposed.

## M131 — F1 to F7

**F1 — a comment claiming a survival order the code did not have, and a defect one band lower.** The
footer's doc said the position and the exit survive a narrow row and "further back" gives way. The author
**probed the real footer rather than arguing from the review**, and the review's stated failure mode does
not happen: at 60 columns with a realistic location the row is `insert ^ 12/40 · g j …` — the badge is
**whole**, and whole even with the new protection disarmed. What the comment actually got wrong is that
the exit's own *words* survive (they do not: `g j`). The real defect is one band lower: disarmed, widths
**26–31** printed `^`, `^ 1`, `^ 12`, `^ 12/`, `^ 12/4` — a **partial badge, a wrong total**, which is
worse than a missing one because absent information is noticed and a wrong number is not. Fixed by
`side_by_side`'s `whole_span` protection plus a reserve so a location cannot starve the badge, a corrected
comment, and a sweep test over widths 26..=60.

> **The correction is the point.** The review read the code; the author measured the screen. Where they
> disagree about what a user *sees*, the measurement wins — and my first commit message repeated the
> review's narrative as fact. Fixed in `32a0305`.

**F2 — one convention, implemented twice.** `render_cache` decided what is *drawn*, `app.rs` what a
keypress *records*: identical text, no test that they agree, and the whole per-card convention lives in
that agreement. Now `App::card_is_open` builds the same `RenderState` and calls `RenderState::is_open`.
**Control 2 is the finding's real lesson:** restoring the duplicated body **passed the agreement test** —
two implementations that agree cannot be told apart by running them — so there is also a *structural*
guard that `card_is_open` delegates and holds no default of its own. Expect that shape wherever "two
places implement one convention" appears.

**F3 — two glyphs with no reader.** `Glyphs::caret` and `Glyphs::warn` were referenced only by the ASCII
test's own hand-written checklist, and the streaming tail is drawn as plain prose spans, so "draw it"
would have meant inventing an affordance rather than wiring one. Both deleted; `theme.warn()` is the
*role* and stays. The hole that produced them is closed by `Glyphs::ALL`/`named()`, so the ASCII test
iterates the table and a glyph added later is checked by construction.

**F4 — eight doc defects**, seven duplicated lines plus one stale sentence sitting above the *light*
palette's bands, plus a `set_palette` doc describing a repaint that does not happen. Guarded by
`no_doc_comment_line_is_written_twice`, because a repeated doc line reads as emphasis and survives review.

**F5 — the artefacts, adjudicated rather than swept.** `zz_frame_dump_for_the_report` /
`zz_frame_dump_no_color` **are not in the tree and never were in git** — `git log --all -S` across 300
commits finds them only inside the review document; they were a previous author's working-tree leftovers.
`fn filler` was real and dead, kept alive by an `allow(dead_code)`, and duplicated an inline loop: the test
now calls it and the allow is gone.

**F6 — documentation hygiene.** Two `CHANGELOG.md` entries, three missing `docs/README.md` links (with an
explicit note that the list was last kept current at M115, so a reader can tell drift from "nothing
landed"), and `docs/history.md` verified current through M130. **Stated limit rather than silent scope:**
the changelog stops at M82, so M83–M121 have no entries — backfilling 39 milestones from dev logs is a
different job and doing it badly is worse than the gap.

**F7 — an assertion partly about ratatui.** `fg_of` maps a missing foreground to `Color::Reset` while the
NO_COLOR test asserted `Some(Color::Reset)` literally. Now cited as `None | Some(Reset)`, the form the same
file already used, with `fg_of`'s doc saying why it collapses the two.

**Controls:** three, each announced before arming, armed for one run from a backup taken *before* the arm
into a directory outside the repo, restored byte-identically and **grepped for the control text** (one
earlier author's restore silently did not land because the backup had been overwritten by the armed file).
F4 and F6 are prose and have none; none was invented. Honest limit on F7: the *relaxation* has no control —
a test cannot fail because it now accepts `None` — only the "still refuses a hue" half does.

## M132 — the battery failure that was a production defect

`m131a` died in `epsilon-llm`'s `concurrent_saves_keep_every_provider`:

```
concurrent save: "acquire …\.tmpfrd0zk\auth.lock: Access is denied. (os error 5)"
```

The crate was untouched by the slice and the test uses a **unique** `tempdir`, so it was not the
shared-path flake M128 fixed. Reading the code found the cause: `acquire_auth_lock` retried on
`AlreadyExists` and returned **immediately** on every other error — including `ACCESS_DENIED`, which on
Windows is what the second caller gets when the first is mid-delete of the lock path. So the user sees
"Access is denied", which is wrong twice: it is not a permission failure, and it is not permanent. Two
processes saving at once — the CLI and the TUI, the case the test is *named* for — is a realistic way to
hit it.

A lost race to a transient sharing violation is now treated as **contention**, with the same backoff, so
the retry budget covers both reasons for losing.

## Verification

Battery `m132a`: **all five gates exit 0, 1,175 passed / 0 failed / 1 ignored across 61 binaries.**
`epsilon-tui` 308 tests (303 before), `epsilon-cli` 39/13/7/3/2/5. Eight consecutive runs of the auth
concurrency test passed — stated for what it is: **eight runs cannot establish a rate**, only that the
failure path changed from a hard error to a retry.

## Limits

- **`HEAD` was not self-consistent between the two commits**: `app/tests.rs` carried a rustfmt fix that
  the findings commit left uncommitted, so the passing battery had built the *working tree*. The same
  trap as M127, closed the same way — commit the file, do not re-run and hope.
- **The review's F1 narrative is preserved in the `f6cba05` message and is wrong**; the measured version
  is in `audit/evidence/f1-f7-dispositions.md` and corrected in `32a0305`.
- **M83–M121 have no changelog entries**, stated rather than half-filled.
- **The review's structural suggestion is not adopted**: a one-line "what nothing exercises" section per
  milestone would have surfaced two of the five blockers without a reviewer. Recorded as the
  highest-value convention this review produced.
