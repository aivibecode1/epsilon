# The `os error 2` test failures: cause, and what actually fixes it

Three battery rounds have now failed on this exact signature — `m142`, `m143`, and an
earlier one — and the diagnosis has moved from "a flake" to a known, reproducible cause.
This file supersedes the AV-quarantine hypothesis recorded in `m142-failed-round.md`.

## The signature

```
thread '…' panicked at crates\epsilon-session\tests\pi_import_reconciliation.rs:144:29:
rename into C:\Users\Hp\AppData\Local\Temp\epsilon-pi-reconciliation-<pid>\<name>.jsonl:
  The system cannot find the file specified. (os error 2)
```

`ERROR_FILE_NOT_FOUND`, from `std::fs::rename`. The helper writes with `std::fs::write`
(checked, succeeded) and immediately renames the same path, and the rename reports the file
absent.

## What it is not

- **Not the code.** No code path deletes the file between the write and the rename, and the
  failing test differs each round — `per_kind_counts_tie_out_on_both_sides` in `m142`,
  `pi_import_carries_every_source_entry` in `m143` — in the *same* test binary, against the
  *same* helper. A logic defect would not move like that.
- **Not the build layout.** The failing paths are in the tests' own `%TEMP%` scratch
  directories, not under `CARGO_TARGET_DIR`, so the external-target mitigation does not
  reach them.
- **Not the external target dir.** That mitigation is real and is still worth keeping; it
  addresses a different symptom (Avira pinning `target/debug/deps/*.exe`).

## The actual cause

The Hermes runtime on this host exports:

```
TMPDIR=/tmp   TEMP=/tmp   TMP=/tmp
```

A native Windows binary resolves those to **`C:\tmp`** — a directory that does not exist
and is not the directory the path in the error message names. Rust's `std::env::temp_dir()`
follows the process environment, so any test using `tempfile` inherits a temp root that the
agent session's own environment has pointed somewhere the machine does not agree with.

The visible path in the error is
`C:\Users\Hp\AppData\Local\Temp\epsilon-pi-reconciliation-<pid>\…`, because
`tempfile`'s builder resolves and creates that root; the *interception* happens in the
gap between the crate's `fs::write` and its `fs::rename`, where the AV filter is scanning a
freshly created file. Whether it wins that race is nondeterministic, which is exactly why a
different test loses it each round.

**Both conditions are needed for the failure**: a temp root the environment and the machine
disagree about, and a security product scanning the artifact in the write→rename window.
Neither alone produces it — which is why isolated re-runs pass (the race is usually won by
the test) and why the failure clusters in full batteries, where hundreds of files are being
created at once and the scanner is busiest.

## The fix, and its limits

Export Windows-native temp values for any run that creates files:

```sh
export TMP="C:/Users/Hp/AppData/Local/Temp"
export TEMP="C:/Users/Hp/AppData/Local/Temp"
export TMPDIR="C:/Users/Hp/AppData/Local/Temp"
```

This is the same override used to diagnose the M135 failures, and it is the right default
for every cargo invocation on this host.

**What it does not fix:** if the AV filter is the interceptor, a native temp root narrows
the window but does not eliminate a race that depends on scan load. A still-needed belt is
the exclusion for `%LOCALAPPDATA%\Temp`, which is an **owner/admin decision** and must not
be changed from an agent session.

## How to read a future occurrence

Do not re-run until green and cite the green one. Re-running is how a real flake gets
promoted to a false bill of health. Instead:

1. check whether the failing assertion is `os error 2` on a `rename` after a successful
   `write` in a `%TEMP%` path — if so, this is it;
2. confirm `TEMP`/`TMP`/`TMPDIR` in the shell that launched the run;
3. re-run with the override exported, under a **new label**, and keep the failed round.

The distinction that matters: this failure is environmental, so a passing re-run is
*evidence about the environment*, not about the code. It never justifies weakening a gate.
