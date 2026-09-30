# M213 — the invariant A19 lacked, with a test on it

**Status:** landed. GATE PASS `m213` — see [Verification](#verification). Reinstall: **not
required** (no user-visible behaviour changes; the extraction is byte-for-byte equivalent).

## What

M212 fixed [A19](../../audit/03-defects.md) — a resumed launch opened two stores, and the second one
landed on the empty file the launch had just created — but its Limits named the weakest part
plainly: **nothing drove the rule, so a future reorder could reintroduce it with every gate green.**
This is that test.

The decision moved out of `build_runtime` into a named function:

```rust
fn session_path_for(
    workspace: &Path,
    session_file: &str,
    resume_path: Option<&Path>,
) -> (PathBuf, bool)
```

with the invariant in its doc comment — **the resume target wins, because a launch must write to the
file it resumes** — and two tests:

- `a_resume_target_becomes_the_file_this_launch_opens` — the returned path *is* the resumed path,
  and the `bool` says so (the caller needs it to phrase a refusal for a resumed file rather than for
  a launch);
- `a_launch_without_a_resume_keeps_its_own_file` — a plain launch is unchanged, still relative to the
  workspace.

Extracted for the same reason `resumed_model_to_use` was, in the same file, with the same argument:
**a copy of the rule inside a test tests the copy.** The precedent is the repository's own.

## What this does and does not catch — the honest version

- **Catches:** a revert of the *rule*. If someone restores `session_path = workspace.join(session_file)`
  and puts the resume handling back in a second open, these two tests fail, because the helper's
  contract is the property A19 violated.
- **Does not catch:** a second open added *elsewhere*, around the helper. The test pins the decision,
  not the number of `SessionStore::open*` calls that follow it. A stronger test would count opens or
  drive `build_runtime` end to end; both need more machinery than this slice had, and the manual
  conhost check in M212's Verification remains the evidence that the wiring is right.
- **Not covered at all:** the resume path's *error message* selection (`resume_open_error` for a
  resumed file). The `bool` exists for it and nothing asserts the phrase.

## Verification

GATE PASS, 2026-09-30 — battery **`m213`**, documentation final before it started, disk knob as
M208 established. Seven gates, exit 0, per-gate seconds in order: 0s, 0s, 3s, 5s, 166s, 4s, 2.

**1,633 tests across 65 binaries, 0 failed** — M212 measured 1,631 across the same 65, so this slice
adds exactly the two above.

The compile caught one mistake on the way, of the kind that is cheaper to record than to repeat:
`session_file` is an owned `String` in `RunParams` at the call site, not a `&str`, so the
first version failed with `E0308` and the fix was one `&`.

Evidence: [`audit/evidence/gate-m213.log`](../../audit/evidence/gate-m213.log).

**Reinstall: not required** — the extraction is behaviour-preserving, and the installed binary
already carries M212.
