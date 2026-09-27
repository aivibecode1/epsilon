# M174 — every source-shaped test, and whether each one bites

## Why this milestone

Three separate times now, a test of mine has passed for the wrong reason: M160's two
controls that went 363/363, M167's `/name` misroute, M172's CRLF byte search. All three
were **source-shaped tests** — assertions that read a file and match text rather than
exercising behaviour. That is a category, and a category deserves an answer rather than a
run of bad luck.

So: census every `include_str!` in the tree, and for each one that reads source as an
assertion, either point at the control that already proved it or run one.

## The census

`include_str!` appears in 19 places. Seventeen of them load **data**, not source — the
tool descriptions in `crates/epsilon-tools/descriptions/*.txt` and `AGENT_CONTRACT` in
`prompt.rs`. Those are asset loads; nothing to prove.

Four read source for an assertion:

| site | status |
| --- | --- |
| `app/tests.rs` `APP_MODULE_SOURCE` (9 files) | already controlled — M164 restated `card_is_open` and got 362/2 |
| `picker.rs` `the_parser_does_not_know_a_command_the_list_does_not` | controlled on first run — M172, failed and named seven unlisted heads |
| `main.rs` `the_serve_boot_task_reports_a_failed_run_instead_of_discarding_it` | **controlled here** |
| `ui/theme.rs` `no_doc_comment_line_is_written_twice` | **controlled here** |

## Two controls, and one of my own was vacuous first

**theme.rs.** Duplicating a doc line — the exact defect the test is named after:

```
a doc line written more than once reads as emphasis and is a copy:
[
    "\"whereas falling back to 16 colors merely looks plainer.\" at lines [83, 84]",
]
```

It bites, and it names the two lines.

**main.rs.** My first attempt was **vacuous**, and worth recording rather than quietly
redoing. `admitted_run_failure(&run)` occurs three times in `main.rs`; I replaced the first
one, which produced

```
error: expected expression, found `let` statement
    --> crates\epsilon-cli\src\main.rs:2238:37
     |  if let Some(line) = let _ = &run; {
```

A compile error is not a test failure. It told me nothing about whether the test bites, and
a careless run would have logged it as a control. Retargeted at the `serve_mode` statement,
and deliberately written so the test's *second* assertion stays satisfied — the failure then
isolates the first:

```
the server's boot task must report the run's failure through the same path the rest of
the server logs
```

## What the failure text revealed

The failure message prints the `body` the test read, and its first line is not the
function's first line:

```
shared: &SharedRun, initial_task: Option<String>, port: u16) -> i32 {
```

`serve_mode`'s signature wraps, so `split_once("async fn serve_mode(")` leaves the reader
starting **mid-signature**. The substring assertions are unaffected, and the
`take_while(|line| *line != "}")` end-detection is safe because nested Rust blocks are
indented and the first column-zero `}` is the function's own. So this is not a defect — but
it is M164's wrapped-signature trap in its benign form, and worth knowing that the test's
`body` is not what its comment implies it is. The comment says "up to the end of the
function" and does not claim to describe the start, so it is accurate.

## Verification

Both control targets restored and confirmed **clean by `git status`** — not by reading the
files back, because "I put it back" is exactly the claim that has been wrong before.

Battery `s319`: FMT=0, CLIPPY=0, TEST=0, DEMO=0; **62 binaries, 1,298 passed, 0 failed** -
unchanged, because this milestone adds no test and changes no code. The work was controls,
not edits, and the commit is honest about that.

This is the first battery in a while whose count is *expected* to be unchanged, and that
expectation is the point: the milestone's deliverable is evidence, not a diff.

## Limits

- **This covers the four source-shaped tests that exist today.** The next one written needs
  a control, and nothing enforces that — there is no rule saying "a source assertion needs a
  mutant", only this milestone's argument. A lint cannot check it either.
- **A control proves the test fails on that one mutation**, not that it fails on all the
  wrong states. `no_doc_comment_line_is_written_twice` is proven against a duplicated line,
  not against, say, a moved one.
- **The behavioural tests are not audited by this milestone**, and there are ~1,300 of them.
  Proving each bites would be mutation testing over the whole workspace, which is a project
  in itself and not something to claim as done.
- **`serve_mode`'s guard still describes intent, not behaviour.** It asserts the boot task
  *calls* the reporter; it does not assert that a failing run is reported to a client. The
  behavioural test above it does that, and the doc comment says why both exist.
