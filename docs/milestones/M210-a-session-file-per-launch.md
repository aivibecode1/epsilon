# M210 — a launch writes a session file of its own

**Status:** landed. GATE PASS `m210` — see [Verification](#verification). Reinstall: **required**
(this changes which file a launch writes, and the owner tests the installed binary).

## What

Plan 020 step 2a. Every frontend used to open a **fixed** file — `tui.jsonl`, `cli.jsonl`,
`serve.jsonl`, `rpc.jsonl` — so three TUI launches appended three conversations to one lane. The
user-visible consequence was in `--continue`: it restored the file's active branch, which by then
was *all three conversations* presented to the model as one context with no boundary between them,
while the header still carried only the first conversation's id.

The TUI now names its file per launch:

```text
.epsilon/sessions/2026-09-29T14-22-05-881Z_ses_19ab34c0.jsonl
```

using M209's rule (`epsilon_session::time::session_file_name`, transcribed from pi's
`repo.ts:47-50`) and the shared, monotonic id generator (`epsilon_contracts::new_id`).

## The three decisions in it

- **Only the TUI, deliberately.** `cli`, `serve` and `rpc` keep their fixed names. `-p` is a
  one-shot run and `serve`/`rpc` are long-lived single sessions by design; giving them
  per-invocation files is a separate question about those frontends, not a step in this one. The
  scope limit is the reason `RunParams.session_file` became an owned `String` rather than the
  defaults changing wholesale.
- **The timestamp alone would not do.** Two launches in the same millisecond would land on one file
  — the exact failure this closes, reintroduced through a race. The name therefore carries a token
  from `new_id`, which is monotonic within a process and unique across processes.
- **The name's token is a uniqueness component, not an identity claim.** The store still mints the
  session's own header id (`store.rs:746`), so the id *in the name* and the id *in the header* are
  different strings today. That is a wart, it is stated rather than hidden, and unifying them is the
  next step in the plan — the alternative was adding an id parameter to four store constructors in
  the same slice that changes which file a launch writes.

## Why `--continue` gets better without being touched

It already resolves "newest `*.jsonl` for this cwd" (`latest_session_file_in`). With one file per
launch that question now has the answer it always claimed to have: the newest *conversation*,
rather than the newest *file that contains every conversation*. Nothing in the resume path changed
— which is the evidence that the change is at the right layer.

## Verification

GATE PASS, 2026-09-29 — battery **`m210`**, documentation final before it started, run with the
disk knob below. Seven gates, exit 0 each: `check-links` 0 s (479 relative links, 0 broken),
`check-lint-tables` 0 s, `fmt --check` 4 s, `clippy --workspace --all-targets` 11 s,
`test --workspace` 269 s, `demo` 6 s, `real-check` 1 s — the short legs are a warm target.

**1,629 tests across 65 binaries, 0 failed** — summed from the 65 `test result: ok.` lines.
M209 measured 1,626 across 64; this slice adds the three in
`crates/epsilon-cli/tests/session_file.rs`, which is itself the 65th binary.

**Disk knob, disclosed:** `CARGO_PROFILE_DEV_DEBUG=0` / `CARGO_PROFILE_TEST_DEBUG=0`, as M208
established for this drive.

**What the battery cannot check, and did not:** nothing in this round launched the TUI twice in a
scratch workspace and listed the files it wrote. The naming call site is tested, the rule is tested,
and the wiring between them is *read* — so the claim of this slice rests on those two tested halves
plus the compiler, not on an observed pair of files. It is stated here because the first version of
this section asserted that observation, and it had not been made. An earlier round in this session
set exactly that precedent in the other direction (the installed binary's own frame, quoted from a
real console), and this one does not meet it.

Evidence: [`audit/evidence/gate-m210.log`](../../audit/evidence/gate-m210.log).

**Reinstall: outstanding, and deliberately deferred.** It is required — this changes which file a
launch writes — and it has not run, because a release build is ~30–45 minutes cold and the M207
notice is already queued behind the same install. The installed binary is therefore still the
**M206** build: two milestones (M207, M210) are in the tree and not yet in the artifact the owner
runs. Recorded rather than glossed, and the next round that spends the build should do both.

## Limits — what this does not prove

- **No end-to-end two-launch test.** The naming call site is tested (`crates/epsilon-cli/tests/
  session_file.rs`: two tokens in one millisecond do not collide), and the resume path is tested
  elsewhere, but nothing launches the TUI twice and asserts that `-c` restores one conversation.
  That is the plan's step-2 evidence and it is still outstanding — recorded here rather than
  implied, because "the pieces are tested" is not "the behaviour is".
- **The per-cwd directory is not in this slice** (plan 020 step 2b). Sessions from every workspace
  still share `.epsilon/sessions/`, so `-c` picks the newest by cwd *filter* rather than by
  directory. Correct, and not yet the reference's layout.
- **Old files are not migrated.** `tui.jsonl` stays readable and stops being created. A user who
  resumes it by name gets the many-conversation file they always had.
- **Nothing prunes.** A workspace used daily accumulates a file per launch; the sessions browser is
  the only way to see them, and there is no retention policy. That was true before this change in
  the sense that the *growth* was there — it is now visible as files instead of hidden inside one.
