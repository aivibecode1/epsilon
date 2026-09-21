# M103 — the extension host protocol, from five tests to thirty

## What

`epsilon-ext` is the one place in this harness where a line of text written by an untrusted
child process becomes content in the model's transcript. It had **five tests for the whole
crate** — the least-covered surface relative to its size in the repository, and the one where
being wrong is worst. That combination is why this slice exists.

The host declares four error categories — timeout, crash, protocol violation, tool failure —
and they were largely unexercised. They now are, driven through the real fixture child rather
than a mock: a child that speaks no JSON at all, one that speaks JSON of the wrong shape, a
reply bearing an id nobody asked for, a child that never replies and one that replies after its
timeout, an over-long line, an absurd number of lines, a child that dies mid-invoke, a child
that dies at handshake, and the crash budget that disables an extension rather than retrying it
forever. The new `crates/epsilon-ext/tests/host_protocol.rs` carries the process-level cases as
one test each, the fixture gained modes instead of bespoke children, and the flake fix's own run
numbers the suite per binary (below).
**One real defect, and it is the interesting kind.** `read_line_bytes` bounded a frame only
while scanning for a terminator. If the newline arrived in the same read as the payload — the
ordinary case — the length check never ran, so a single oversized line came back as a **valid
frame**. The cap was advisory against exactly the input it existed to stop. The check now runs
on the finished line as well, and the comment says which branch was missing rather than
restating the code.

Two smaller things the same pass settled: invalid UTF-8 is handed to the JSON parser to refuse
rather than being silently lossily converted on the way in, and a `result` frame is validated
before anything in it is believed (`text` must be a string, `details` an object, `is_error` a
bool) because accepting a frame with no `text` would turn a broken child into a successful empty
tool result — indistinguishable, in the transcript, from a tool that legitimately returned
nothing.

## Key decisions

- **Tests drive the real fixture process, not a mock.** This is not a stylistic preference and
  the defect above is the argument: the cap check was in the right function, in the right place,
  doing the right thing for the wrong input shape. Reading it would not have found the bug.
  Sending it a line with a newline on the end found it immediately. The commit records that
  explicitly as the reason to write it this way.
- **Every wait is bounded, and the bound fails the test rather than hanging the suite.**
  `tests/support/mod.rs` carries `awake(fut, cap, what)`, which panics with the caller's label
  and the elapsed time, and `MANIFEST_TIMEOUT` for spawns. A protocol test that hangs is a test
  that reports nothing; a protocol test that fails with "invoke against a child that never
  answers but keeps writing frames did not return within 40s" reports the defect.
- **The invoke deadline is for the whole invoke, not per read.** `tokio::time::timeout_at` with
  one deadline computed before the loop replaces a per-read timeout, because a child that emits
  any complete frame inside the window would otherwise restart the timer and hold the call open
  indefinitely while the documented contract promises it ends. `a_chatty_child_cannot_extend_the_
  invoke_timeout_indefinitely` is the regression test.
- **Volume is bounded separately from time.** `MAX_FRAME_BYTES` (2 MiB) and `MAX_INVOKE_BYTES`
  (16 MiB) exist because a child chooses how many bytes it sends; a time bound alone leaves the
  host's memory to the child's discretion. Hitting either is a protocol violation, not a slow
  extension.
- **Frames are read as bytes, not `String`.** A line is only known to be UTF-8 once it is
  complete; deciding per 8 KiB pipe chunk would reject a multi-byte character that straddles a
  buffer boundary. `the_frame_reader_and_the_handshake_reader_agree_on_a_line` pins that the
  handshake reader and the frame reader frame a line identically — if they disagreed, a child's
  first line and its tenth would be read differently.
- **The fixture grows misbehaviour modes rather than bespoke children.** Nineteen modes, one per
  case, so each test is independent and the whole protocol suite is one fixture binary. The two
  "not JSON" modes share a marker string with `tests/support/mod.rs`, which is what the tests
  assert is *absent* from host-produced text — the host must never echo an untrusted payload
  back into a tool result.
- **The slice was performed by a delegated subagent under instruction to fix only what a test
  proved**, and the commit says so. The cap defect it found failing was the one genuine find and
  was fixed in the same commit; every number in the message was re-run by the committer rather
  than taken from the subagent's report. That sentence is part of the audit trail, not modesty.

## Gotchas

- **A protocol test failed because the machine was slow** (the second commit). `a_child_that_
  never_says_hello_times_out_at_spawn` flaked because two clocks disagreed by design: the
  fixture was silent for a *finite* 30 s while the test's outer bound was 20 s, so on a loaded
  machine the bound fired before the host's own 10 s handshake deadline and the test failed
  against a host that was behaving correctly. Measured on this box with a throwaway phase-timing
  probe: the host's handshake timeout fires at **10.03 s**, starting one small process costs
  **1.7 s**, and spawn plus teardown of a silent child cost **23.4 s** under load. The fix has
  three parts — `--slow-hello` is now silent *permanently*, so no race remains to lose;
  `MANIFEST_TIMEOUT` goes 20 s → 45 s with those measurements in its doc comment; and the spawn
  bound's failure message now says what reaching it means. The protocol suite that used to wedge
  for ~30 s as an unfiltered failure now finishes in ~15 s.
- **The root cause came from instrumenting, not from reading.** The memory note for this round
  says it plainly: the phase-timing probe found the disagreement; reading the test would have
  shown two numbers and no reason to prefer either.
- **The chatty fixture's rate had to change with the deadline fix.** `--pace` wrote a line every
  second, which is both far below the new byte budget (so the byte cap can never end that call)
  and slow enough that a per-read timer would look harmless; it now writes every 20 ms, and
  `MAX_INVOKE_BYTES` is deliberately far above the ~200 KB per invoke that produces. The two
  defences are tested apart, not raced. Getting that backwards would have made the timeout test
  pass for the wrong reason.
- **`read_line_bytes` leaves the rest of an over-long line unread on purpose.** The host stops
  draining, so callers must treat the stream as desynchronized; a cap that silently skipped
  ahead would resynchronize onto arbitrary bytes and report them as frames.
- **Protocol violations do not spend the crash budget.** `protocol_violations_do_not_spend_the_
  crash_budget` pins that a broken child is reported rather than accumulating toward the
  disable-an-extension threshold — a violation is a bug in the extension, a crash is a failure
  of it, and conflating them means one malformed frame could silently disable a working host.
- **The G14 row and the defect are recorded outside the code too** (`96b6f77`, docs only): the
  audit now says the frame cap bounded only lines whose newline had not yet arrived, so the
  defect survives in a document a future reader will meet before the code.

## Verification

The measurements that follow are as the two commit messages state them. No battery log for this
slice is preserved in `audit/evidence/` — there is no `gate-m103` log and no exported workspace
log; `gate-m93b`, `gate-m94c`, `gate-m95b` and `m101-gates.log` are other slices' bytes.

From `5bfd560` (the slice):

- `cargo test -p epsilon-ext` — **30 passed, 0 failed** (5 → 30; the crate's own tests plus the
  new `tests/host_protocol.rs` and the new unit tests in `host.rs`).
- `cargo test --workspace` — **1017 passed, 0 failed, 1 ignored** (985 → 1017).
- `cargo clippy -p epsilon-ext --all-targets -- -D warnings` — exit 0.
- `cargo fmt --all -- --check` — exit 0.

From `6f3d544` (the flake fix), per binary: **6/0, 2/0, 3/0, 22/0**, all green at default
parallelism, and the protocol suite's wall time fell from a ~30 s wedge to ~15 s. That commit
does not say which binaries these are, and this document does not guess: both figures are quoted
as their messages state them, without being reconciled into one number.

`agents/memory/next-hardening.md` was updated in the same commit to carry the 1,017 baseline and
the flake's measurements, so the numbers are in two places rather than only in a commit message.

## Limits

- **No frozen-candidate battery ran on this milestone's bytes.** The 1,017 figure is a working-tree
  workspace run, not a gate battery against a hash-verified candidate, and no per-binary
  reconciliation against the previous battery exists for it.
- **The crate's test-count bookkeeping is not reconciled.** The slice's message says `cargo test
  -p epsilon-ext` reached 30 passed; the flake fix reports four binary totals (6, 2, 3, 22)
  without naming which binaries they are. Both are quoted as stated and are not added up here.
- **No pre-fix RED was preserved for the frame-cap defect.** Its test now fails without the fix
  by construction — the over-cap line is accepted as a frame — but the red run itself was not
  captured as an artifact.
- **The timeouts are wall-clock bounds on a shared, loaded machine.** They were widened once, with
  measurements, and can be widened again; a much slower or much more loaded box can still lose
  the race between the fixture's own deadline and the test's outer bound. The permanent-silence
  fixture removes the specific race that was found, not the class.
- **`MANIFEST_TIMEOUT = 45 s` makes a genuinely hung spawn slow to report.** That is the deliberate
  trade — a false failure is worse than a slow one — but it means a real hang costs the suite 45
  seconds before it says so.
- **Live extension use is still not exercised.** Everything here drives the fixture binary; no
  test spawns a real third-party extension, and the MCP bridge and event-forwarding paths are
  outside this slice. Coverage improved on the protocol surface specifically.
- **The read-only review that AGENTS.md asks of non-trivial slices is not recorded for M103.**
  This slice was itself delegated and its numbers re-run by the committer, but there is no
  `audit/evidence/m103-review.md` and no independent review artifact beside it.
- **Windows-only, and the suite now depends on process spawn latency.** The measurements are this
  machine's; the Linux CI job has not run on these bytes.
