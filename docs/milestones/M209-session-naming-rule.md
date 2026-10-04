# M209 — the session-naming rule, transcribed from the reference

**Status:** landed. GATE PASS `m209` — see [Verification](#verification). Reinstall: **not
required** (nothing calls the new functions yet; see Limits).

## What

Plan 020 step 1: the rule that decides **where a session file lives and what it is called** now
exists in one place, `epsilon_session::time`, transcribed from pi rather than invented
(`references/pi-0.85.1/packages/agent/src/harness/session/jsonl/repo.ts:42-50`):

```ts
function sessionDirectoryName(cwd: string): string {
	return `--${cwd.replace(/^[/\\]/, "").replace(/[/\\:]/g, "-")}--`;
}
function sessionFileName(createdAt: number, id: string): string {
	const timestamp = new Date(createdAt).toISOString().replace(/[:.]/g, "-");
	return `${timestamp}_${encodeURIComponent(id)}.jsonl`;
}
```

Three functions, one call that composes them (`session_file_for`), and the reason the whole thing
sits in the same module as M208's ISO formatter: the timestamp in a *file name* is the timestamp
the *wire* renders, with `:` and `.` turned into `-`. Two copies of that string would be A12 again.

## The details that are not decoration

- **The leading separator is stripped before the replacements**, which is why a POSIX path yields
  `--home-u-p--` and not `---home-u-p--`. The order is the reference's, and the test asserts both
  shapes rather than the one this machine happens to use.
- **A Windows drive colon becomes a dash** (`--C--Users-Hp-Epsilon--`), because after the leading
  strip a `C:` is just another character in a name that cannot contain `:`.
- **`encodeURIComponent` is transcribed, not assumed.** Every id this harness generates is already
  in the unreserved set, so the function is not *needed* today — but an imported or forked session
  can carry an id from outside, and a name built without escaping could contain a separator and
  write outside the directory the caller just computed. The test drives `../../etc/passwd` through
  it and asserts the result has no `/`.
- **One call composes the path** (`session_file_for`), so a caller cannot apply half the rule. A12's
  finding was two crates each holding half of one rule about a directory.

## Verification

GATE PASS, 2026-09-29 — battery **`m209`**, documentation final before it started, run with the
disk knob below. Seven gates, exit 0 each on the retry.

**The first attempt failed, and both failures were mine, in the same three characters.** fmt
--check and clippy (clippy::tabs_in_doc_comments) both rejected the TypeScript snippets I had
pasted into the rustdoc **with their original tab indentation**. Three tabs, two gates, one cause —
and the reason it is recorded rather than quietly fixed is that the failure is the *useful* kind: a
gate caught a defect that reading the diff would not have. The retry ran under a fresh label
(`m209-r2`, per the rule that a reused label destroys the failed attempt's evidence), and the
original log is preserved as the record of what failed.

**1,626 tests across 64 binaries, 0 failed** — summed from the 64 `test result: ok.` lines.
M208 measured 1,622 across the same 64; this slice adds the four naming tests.

**Disk knob, disclosed:** `CARGO_PROFILE_DEV_DEBUG=0` / `CARGO_PROFILE_TEST_DEBUG=0`, as M208
established for this drive. Nothing changes about what is compiled or asserted; a panic backtrace
would be much less useful, so the round that debugs a failure should turn it back on.

Evidence: [`audit/evidence/gate-m209.log`](../../audit/evidence/gate-m209.log) (the passing run).

**Reinstall:** not required — nothing calls these functions yet.

## Limits — what this does not prove

- **Nothing calls it yet.** This is the *rule* only: plan 020 steps 2–4 (a launch creates a file;
  `reset_leaf`'s disposition; the docs) are what make a user's `-c` mean one conversation. Until
  then this is the "faithful port, no production caller" shape this repository watches for, which
  is why it is recorded as step 1 of a plan with a named next step rather than as a feature.
- **The reference's behaviour is transcribed, not executed.** The expected strings in the tests
  were derived by hand from the TypeScript above; no JavaScript was run. A transposition error in
  my reading of the two `replace` calls would produce a self-consistent implementation and green
  tests — the same failure this repository names as "a control that passes is a control you have
  not yet proven works". The independent check is a session file produced by pi itself, which
  nothing here does.
- **No filesystem test.** `session_file_for` composes a path; nothing asserts the directory is
  created, that two sessions in one millisecond do not collide (the id component is what prevents
  it, and it is inherited rather than exercised), or that a path longer than `MAX_PATH` fails
  legibly.
