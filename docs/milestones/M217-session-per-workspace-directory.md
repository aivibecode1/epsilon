# M217 — a workspace's sessions get their own directory

**Ledger row A16, plan [`020`](../plans/020-one-session-per-file.md) step 2b.** One directory per
working copy, one rule that both the writer and every reader call.

## What

A launch now writes `.epsilon/sessions/--<cwd>--/<ISO>_<token>.jsonl` — pi's layout
(`references/pi-0.85.1/.../session/jsonl/repo.ts:42-45`), which M209 transcribed into
`epsilon_session::time` but nothing called.

| | Before | After |
|---|---|---|
| the writer | `fresh_session_file(now, token)` → `.epsilon/sessions/<file>` | `fresh_session_file(now, token, cwd)` → the per-cwd directory, via `time::session_file_for` |
| `--continue` | `latest_session_file(root)` — newest file in the flat directory | `latest_session_for_cwd(root, cwd)` |
| `doctor` | `latest_session(root, Some(cwd))` | the same `latest_session_for_cwd` |
| the `/sessions` browser | `list_sessions(root)` | `list_sessions_for_cwd(root, cwd)` |
| the rule itself | composed in five places | **one** function, `time::session_dir_for`, called by both sides |

## What it fixes, stated without inflation

**The defect 2a already fixed** — three launches appending three conversations to one lane, with
`--continue` handing the model all three — was M210's, not this one's. What 2b adds is worth naming
precisely:

- **The sessions directory a user opens is now this workspace's.** Before, `.epsilon/sessions/` was
  one flat directory holding every session of the workspace *and* whatever else landed there; now the
  per-cwd directory holds this workspace's, and the flat root is only the pre-M217 history.
- **`--continue` cannot be offered another workspace's session.** The old reader took the newest file
  in the directory with **no cwd filter at all** (`latest_session_file` passes `None`); the new one
  filters by the directory first and by the header's `cwd` for the legacy files it still reads. In a
  shared directory that was a wrong answer waiting for a second workspace; it is now not reachable.
- **The `cwd` header filter stops being load-bearing for new files.** pi keeps writing it; this
  harness no longer needs to read it to know which directory it is looking at.

## Key decisions

- **One function, both sides — not four readers descending by hand.** The plan's own "hidden half"
  predicted the trap (every reader is handed the sessions *root*, so moving only the writer would
  have emptied `--continue`, `/sessions` and `doctor` with no error anywhere) and prescribed the
  answer: one `session_dir_for`, called by the writer and by the readers. That is A12's rule — one
  rule, one place — applied before the defect rather than after it.
- **The flat-root fallback is the current behaviour, kept.** It is the header-`cwd` filter
  `latest_session` has always applied, so nothing new was written to stay compatible: a workspace
  that upgrades keeps every session it has, and new files never go back to the flat directory.
  **Nothing migrates** — rewriting a user's history is a worse failure than an untidy directory.
- **`cwd` is a parameter of `session_rows`, not derived from `dir`.** The browser stores it
  (`SessionBrowser.cwd`) rather than re-deriving a workspace from a directory name, so a filter
  keystroke cannot change which sessions the list is over.
- **The stored relative path uses `/`.** `session_file_for` joins with `\` on Windows; the
  `session_file` field is one string that is joined, printed and compared, so it is normalised once
  at the seam.
- **The unfiltered readers stay.** `list_sessions`/`latest_session` are still the right answer for
  "any session in this directory" — RPC discovery and a human with a path — so nothing was removed.

## The failing test that was the design working

`app::tests::sessions_command_browses_and_enter_switches_the_session` failed on the first run:

```text
panicked at crates\epsilon-tui\src\app\tests.rs:788: browser open
```

Its fixture helper wrote sessions with `SessionStore::open_in(path, &std::env::temp_dir())`, so the
headers named `%TEMP%` while the test's workspace was a different temp directory. The new reader
filtered them out — **correctly**: that is precisely the filter that stops another workspace's
sessions being offered as this one's. The test had been passing for a reason that M217 removed.

The fix is not a loosened filter. The helper takes the workspace and writes it into the header, and
the test now plants **one session in the per-cwd directory and one in the flat root**, so it asserts
the property this slice exists for — a browser that shows both layouts — instead of two files in one
directory. The old log is kept as `audit/evidence/gate-m217-FAILED-fixture-cwd.log` because a test
that fails for the right reason is evidence, not noise.

## The negative control

There is no pre-fix RED for a union that did not exist, so the control removes the compatibility
fallback — the one line that keeps old sessions visible — and the two new tests fail for the reasons
they name:

```text
panicked at crates\epsilon-session\src\discover.rs:507:
an un-migrated workspace still continues      left: None   right: Some("legacy.jsonl")
panicked at crates\epsilon-session\src\discover.rs:476:
both layouts, newest first, and no stowaway: left: ["ses_new"]   right: ["ses_new", "ses_legacy"]
```

Restored, mtime touched (so cargo rebuilds rather than re-running the control binary), and re-run.

## Verification

**Battery `m217-r2` GATE PASS** — seven gates, `audit/evidence/gate-m217-r2.log`: **1,638 tests across
65 binaries**, against M216's 1,634 across the same 65: **+4**, which is exactly the new tests (two in
`epsilon-session::discover`, two in `crates/epsilon-cli/tests/session_file.rs`). Gate shape:
check-links 0 s, check-lint-tables 0 s, fmt 4 s, clippy 31 s, test 314 s, demo 20 s, real-check 8 s.

The first attempt (`m217`, kept as `audit/evidence/gate-m217-FAILED-fixture-cwd.log`) failed on the
one test described above; a full `cargo test --workspace --no-fail-fast` after the fix reported no
other failures, which is the evidence that one failing binary had not been hiding a second.

**Writer and reader agree, asserted.** `the_writer_and_the_reader_agree_on_where_a_file_goes`
compares `fresh_session_file`'s output against `time::session_file_for` for the same inputs — the two
sides of the boundary, compared to each other rather than to a hand-written expectation, which is
the only comparison whose answer does not come from the implementation.

**Run with** `CARGO_PROFILE_DEV_DEBUG=0` / `CARGO_PROFILE_TEST_DEBUG=0` (M208's disk knob,
disclosed: a backtrace from this round would be close to useless).

## Limits

- **`reset_leaf` still has no production caller** (plan 020 step 3, untouched here). The plan's
  decision stands and is recorded rather than half-executed: with one file per session, `/new`
  should start a *new file* — a store swap — and `reset_leaf`, which is the one-file-many-conversations
  primitive, goes with it. Deleting a public API and changing what `/new` does to the user's file is
  a behaviour change with its own frame check, and it is not smuggled in here.
- **Nothing prunes.** A daily workspace now accumulates one small file per launch and one directory
  per working copy; that is the accepted price of the model (pi has the same shape), and the pruner
  is still an open question rather than a hidden one.
- **The directory name can collide**, and this one is inherited from the reference: `/a/b` and
  `/a-b` both become `--a-b--`, so two workspaces whose paths differ only in separator substitution
  share a directory. The header `cwd` filter covers the legacy files but not these. pi has the same
  property from the same two lines, and "deviate from the reference here" was not a call this slice
  was entitled to make.
- **No end-to-end test drives a real launch into the new directory.** The writer is asserted at
  `fresh_session_file`, the readers at the library union, and the browser through the app; nothing
  launches the binary and checks the path on disk (M212's `Limits` records the same gap for
  `build_runtime`). **That gap is closed by measurement at M218**, which drove the *installed* binary
  through `scripts/console-probe.py` in a real console and read back what the launch wrote:

  ```text
  .epsilon/sessions/--C--Users-Hp-AppData-Local-Temp-eps-m218w-f1f6d08b--/
    2026-09-30T16-11-08-449Z_ses_1a0f31583210.jsonl
    2026-09-30T16-11-08-449Z_ses_1a0f31583210.lock
  ```

  with the file's header carrying `"cwd": "C:\Users\Hp\AppData\Local\Temp\eps-m218w-f1f6d08b"` — the
  directory is pi's name for that path, the file is the timestamp and the monotonic token, and the
  header agrees with both. Artifact:
  `audit/evidence/console-probe-m218-sessions-layout.json`. What remains unproven is that no *test*
  asserts it, which is a different claim and still true.

## Nothing exercises

No test asserts that `--continue` from the *CLI* resolves through the union — `main.rs`'s
`Resume::Latest` arm is read, not run — and no test drives `doctor`'s report against a workspace
holding both layouts. The union is tested once, in `epsilon-session`, and three callers depend on it
sharing that one answer.