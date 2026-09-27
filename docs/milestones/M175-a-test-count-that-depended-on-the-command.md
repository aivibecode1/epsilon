# M175 — a crate whose test count depended on which crates were in the command

## Where this came from

M174 ended by saying that proving behavioural tests bite needs mutation testing over the
whole workspace — a project in itself. Before reaching for that, there is a much cheaper
and sharper question in the same spirit: **which tests exist but never run?**

Censused every `#[test]` / `#[tokio::test]` in the tree (1,305) and attributed each to the
`cfg` immediately above it. Six are gated to a platform:

| test | gate | runs here? | runs in CI? |
| --- | --- | --- | --- |
| `auth_file_is_owner_only_on_unix` | `cfg(unix)` | no | yes (ubuntu `--workspace`) |
| `bwrap_argv_binds_workspace_writable_and_unshares_all` | `cfg(unix)` | no | yes (ubuntu, plus `bwrap_unix`) |
| `bwrap_argv_builder_is_platform_independent` | `cfg(windows)` | yes | yes (windows-latest) |
| `the_job_object_backend_does_not_claim_to_confine_content` | `cfg(windows)` | yes | yes (windows-latest) |
| `the_windows_shell_skips_the_user_profile` | `cfg(windows)` | yes | yes (windows-latest) |
| `bash_runs_powershell_with_exit_codes_and_caps` | `cfg(windows)` | yes | yes (windows-latest) |

**Every one of them runs somewhere.** The first draft of this milestone said four of them
"never run in CI", which was wrong: `windows-latest` runs `cargo test --workspace`, so
`cfg(windows)` tests run there. Recorded because the correction is the useful part.

## The claim I was about to make, and the measurement that stopped it

Reading `ci.yml`, the Windows job runs `cargo test --workspace` with **no
`--features real`**, while `epsilon-tui` declares `real` as *not* a default feature. That
looks like a real gap: the whole OAuth adapter, three `cfg(feature = "real")` gates in
`app/login.rs`, and the M165 test would never be compiled by CI.

It is false, and only measurement showed it:

```
cargo test --workspace          (no --features)  ->  62 binaries, 1,298 passed
   M165 oauth test ran: True
cargo test -p epsilon-tui --lib (no --features)  ->  373 passed
   M165 oauth test ran: False
```

Feature unification carries `epsilon-tui/real` in from `epsilon-cli`, whose `real` *is* a
default feature (M63). So the workspace invocation is fine and CI is fine.

But the second line is the actual defect, and it is a nasty one: **the same crate yields
373 or 374 tests depending on which other crates are in the command.** The test that
silently disappears is the one M165 added after a no-op `TuiLoginUi::notify` passed
364/364. Anyone — including a subagent — checking a TUI change with
`cargo test -p epsilon-tui` would get a green run that does not include the OAuth adapter.

## The fix

`default = ["real"]` in `epsilon-tui` and `epsilon-core`, for the reason `epsilon-cli`
already gives at M63. `epsilon-llm` is left off by default deliberately — its own
comment says that keeps the faux provider free and deterministic — and the new defaults
re-enable it through the same forwarding the CLI relies on.

Measured after the change:

```
cargo test -p epsilon-tui --lib (no --features)  ->  374 passed
   M165 oauth test ran: True
```

Per-crate and workspace invocations now collect the same tests.

## What this does not claim

- **It does not make the real path *tested* in CI any more than it already was.** CI was
  already compiling it; that part of my hypothesis was wrong.
- **`epsilon-llm` still defaults off**, deliberately. A single-crate `cargo test -p
  epsilon-llm` therefore still builds the offline path, and that is correct — its doc
  comment says so.
- **No test asserts that the two invocations agree.** The defect was invisible precisely
  because nothing compared them, and a test that counts its own crate's tests by parsing
  its own source would be a worse problem. This milestone fixes the cause, not the
  symptom, and leaves the comparison unasserted rather than pretending otherwise.
- **Six platform-gated tests still exist and were left alone.** All six are correctly
  gated by name and all six run in at least one regime.

## Verification

- Before/after per-crate counts, measured on the same machine and the same target dir:
  373 → 374, with the OAuth test's presence checked by name in the log.
- Workspace battery `s329`: FMT=0, CLIPPY=0, TEST=0, DEMO=0; **62 binaries, 1,298 passed,
  0 failed** — unchanged, as expected for a feature-default change that aligns an
  invocation rather than adding a test.
