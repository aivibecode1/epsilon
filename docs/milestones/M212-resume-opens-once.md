# M212 — a resumed launch opens its session once (A19)

**Status:** landed. GATE PASS `m212-r2` — see [Verification](#verification). Reinstall: **done**
(2026-09-30) — verified against the installed binary, see below.

## What

[A19](../../audit/03-defects.md) fixed, per [`docs/plans/021`](../../docs/plans/021-a19-resume-opens-once.md).
The defect was an ordering: `build_runtime` opened the **primary** store (`:1813`), attached it
(`:1877`), and only then resolved the resume target and opened **that** (`:1891`) — two stores, two
leases, one path.

That was invisible while the primary had a fixed name, because `latest_session_file` returned the
same file. **M210's per-launch name made it fatal**: a launch writes its header at open, so the file
`-c` had just created was the newest in the directory. `-c` therefore resolved to its own empty
file, waited out a full `STALE_AFTER` window on its own lease, and restored nothing.

The fix is the reorder: **resolve first, open once, and let the resumed file be the primary.**

- the `Resume` match moves above the open, and `session_path` becomes the resumed path when there is
  one;
- the resume block no longer opens anything — it borrows the already-attached store through
  `agent.session_store()`, and drops that guard before `restore_history`.

## Measured, before and after, with the same fixture

Two TUI launches, then one `-c` launch, in a scratch workspace:

| | before (M211 binary) | after (this build) |
|---|---|---|
| files after `-c` | **three** — the launch created its own | **two** — it writes where it read |
| lease countdown lines on screen | 21/22/24 s elapsed, heading for 30 s | **zero** |

So a resumed launch now appends to the conversation it resumed, and starts without a thirty-second
wait. Both halves were verified through the conhost probe against a real console, not by reading the
diff.

## One decision worth stating

Removing the second open would have orphaned `resume_open_error` — the function that phrases a
refusal for the file that is *actually there* (a foreign or damaged session), which is exactly the
message a resuming user needs. Rather than delete it, the primary open's error mapping now selects
between the two messages: a resumed file keeps the specific phrasing, a plain launch keeps the short
one. Both name the path, which is the M202 lesson.

## Verification

GATE PASS, 2026-09-30 — battery **`m212-r2`** (the retry: the first attempt failed `fmt --check`
only, on my own formatting, and a reused label would have destroyed that evidence).
Seven gates, exit 0. **1,631 tests across 65 binaries, 0 failed** — M211 measured the same totals,
which is the expected result for a slice that changes an ordering rather than adding behaviour.

Evidence: [`audit/evidence/gate-m212.log`](../../audit/evidence/gate-m212.log).

**Confirmed against the installed binary (2026-09-30).** After `cargo install`, the same fixture in
a scratch workspace: two session files before `-c` and **two after** — no third file — with **zero**
lease-countdown lines on screen and the TUI reaching its composer about five seconds into the run
(probe wall 17 s, of which 12 s was the probe's own `--at` wait). So the fix holds in the artifact
the owner runs, not only in the debug build the battery produced.

## Limits — what this does not prove

- **No automated regression test.** Nothing drives `build_runtime`, and the extracted-helper test
  plan 021 proposed was **not written**. The evidence that this is fixed is the two measurements
  above plus the existing suite; a future reorder could reintroduce it with every gate green. This
  is the weakest part of the slice and it is recorded rather than implied.
- **The restore is exercised only in its empty case.** Both fixture files contain a header and
  nothing else — the probe never submits a turn — so "the conversation comes back" is *not* what was
  observed; what was observed is that no third file is created and no lease wait occurs. A fixture
  with real turns would test the restore, and it needs a provider the probe can talk to offline.
- **The resume block's migration code is now dead, and that was verified rather than asserted**
  (2026-09-30). The primary open calls `store.persist_migration()` itself — `main.rs:1860`, with its
  own message naming the `.pre-epsilon` backup — so by the time the resume block runs, the file it
  holds has already been migrated and its branch cannot execute. Left in place deliberately (deleting
  code on the same day as reordering it is how a regression hides), and recorded so the next reader
  does not mistake it for live logic. **One thing the check turned up:** those are two different
  sentences for one event, and with this fix the user now sees the primary one — "migrated session
  store to current version (previous bytes kept at …)". Both name the backup, so nothing is lost, but
  a reader comparing the two call sites will wonder which is live, which is exactly why this paragraph
  exists.
