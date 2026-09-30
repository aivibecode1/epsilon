# M222 — the notice and `-c` now agree about what exists

**A17's open half, the one M217 opened and nobody had looked at.** A plain launch tells you how many
earlier sessions the workspace holds and names the command that opens one. Since M217 that number
was computed from the wrong directory, so on an upgraded machine the notice said nothing while
`epsilon -c` had something to open.

## The defect, precisely

`note_prior_history` listed `own.parent()` — the directory the launch's own file sits in. Before
M217 that *was* the sessions root. M217 moved each launch's file into `sessions/--<cwd>--/`, so the
same line of code now counts only this workspace's own sessions and **none of the pre-M217 files in
the flat root** — which is exactly where every session on a user's machine already lives, and which
`epsilon -c` opens, because M217's readers deliberately read both layouts so nothing needs
migrating.

The failure mode is the one this repository treats as a defect class rather than a nit: **a message
that contradicts the thing it points at.** The notice names `epsilon -c`; `epsilon -c` could open a
session; the notice was silent. A user who saw silence concluded there was nothing to continue.

## The fix, and the part that made it testable

```rust
let root = if session_dir.file_name() == Some(OsStr::new(&session_dir_name(&workspace_root))) {
    session_dir.parent().unwrap_or(session_dir)   // in a per-cwd directory
} else {
    session_dir                                  // the pre-M217 layout
};
let earlier = epsilon_session::list_sessions_for_cwd(root, &self.workspace_root)
    .into_iter()
    .filter(|s| s.path != own)
    .count();
```

**The same union `-c` resolves**, so the two cannot disagree — and the *root* is derived from the
per-cwd directory's **name**, which is the rule `session_dir_name` applied when the writer wrote it.
That is the third reader of one rule (the launch, `-c`, and this notice), and the point of writing it
down once is that it stops being three.

**The fixture was the reason this was invisible.** `test_app_with_store` planted the session at
`<temp>/session.jsonl` — not in a sessions tree at all. Every line of code reasoning about *where* a
session lives was therefore exercised only by accident, and the name-matching above had never run
against a real layout. It now opens the store where a launch opens it — the sessions root, the
per-cwd directory under it, the file inside — so the directory logic is under test at last. Fifteen
call sites share that fixture, so this was not cosmetic: it moves the whole TUI test suite onto the
production layout.

## Verification

**Battery `m222` GATE PASS** — `audit/evidence/gate-m222.log`: **1,645 tests across 65 binaries**
against M221's 1,644: **+1**, the legacy-file test. Gate shape: check-links 0 s, check-lint-tables 0 s,
fmt 2 s, clippy 11 s, test 219 s, demo 8 s, real-check 3 s. Run with
`CARGO_PROFILE_DEV_DEBUG=0` / `CARGO_PROFILE_TEST_DEBUG=0` (M208's disk knob).

**The new test** — `a_legacy_session_in_the_flat_root_is_still_an_earlier_session` — plants a session
where every session before M217 lives, with a header naming this workspace, and asserts two things:
the notice fires, **and** `list_sessions_for_cwd` (the resolver `-c` uses) finds the same two files.
Asserting the second inside this test is deliberate: the notice's job is to agree with `-c`, and a
test that checks only the notice would pass if `-c` changed underneath it.

**The control** is the old count, restored verbatim:

```text
a_legacy_session_in_the_flat_root_is_still_an_earlier_session ... FAILED
panicked at crates/epsilon-tui/src/app/startup_tests.rs:104
test result: FAILED. 3 passed; 1 failed
```

**One test fails and the other three stay green** — which is the shape worth having: the failure is
specific to the legacy-file case, so it is the union that is being tested and not the fixture move.

## Limits

- **The other way round is not covered by a test.** A *resumed* legacy file is attached directly, so
  its directory is the flat root and the name check fails, sending the code down the legacy branch —
  which is correct, and is the branch the old fixture exercised. Both layouts now have a test, one of
  them by accident of history rather than by design.
- **The count is a directory scan on every launch.** `list_sessions_for_cwd` reads a header line from
  every file in two directories. At a few hundred sessions that is a few hundred small reads before
  the first keystroke — bounded and cheap per file, and the same cost `-c` already pays, but it is
  not measured here and it grows with the session count the per-file model creates.
- **Nothing prunes**, so that cost grows. It is the open question plan 020 raised when it chose one
  file per session, and it is still open.
