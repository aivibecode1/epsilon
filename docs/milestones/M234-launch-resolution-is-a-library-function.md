# M234 — the second half of ADR-021: launch resolution is a library function too

M233 moved `build_runtime` out of the `epsilon` binary so a second frontend could compose the
same agent. It left one half behind, deliberately, and named it: *"The one thing this does not
fix, named so it is not assumed fixed."* This is that half.

## What moved

`crates/epsilon-cli/src/main.rs` `dispatch` opened with ~320 lines that decided, in order: which
custom providers exist, what the project's config says and whether it is trusted, which model a
`--model` reference resolves to, whether that model's credentials are ready and a sibling's are
not, whether the launch runs real or faux, and what the user is told about any of it.

All of that is now `epsilon_runtime::resolve_launch(&LaunchRequest) -> Result<Launch, String>`.
The CLI's `dispatch` is about forty lines: build a request, call it, print its notices, unpack
it into the `SharedRun` the modes already take.

## The reason it had to move, which is not tidiness

**Four branches of that block called `die()` — a process exit.**

| Branch | Before | After |
|---|---|---|
| a model reference matching nothing | `die("unknown model …")` | `Err("unknown model …")` |
| a reference matching several rows | `die("ambiguous model … — matches: …")` | `Err(…)` |
| `--provider real` with no credential anywhere | `die("--provider real requested …")` | `Err(…)` |
| `--script` that will not read or parse | `die("read …" / "invalid script json …")` | `Err(…)` |

A desktop window needs to show that sentence **and keep running**. It cannot exit, and it cannot
copy a function whose error path terminates the process — so this was the one part of the
composition a second frontend could not have had at any price. The sentences are unchanged, which
is the point: the behaviour moved, the wording did not.

## What is now a field rather than a re-derivation

`Launch` carries `offline` as a **decided fact**. A user can have a saved `deepseek/…` model
*and* ask for the faux provider, and then the window has to name the faux row. Three tests freeze
it:

- `an_unknown_model_is_an_error_a_frontend_can_show_not_an_exit` — and it quotes what the user
  typed, which is the only part of the sentence they can act on;
- `a_faux_launch_resolves_without_a_provider_or_a_key` — the good path still resolves, so the
  test above is not passing because everything fails;
- `a_named_model_does_not_make_a_faux_launch_look_online` — **the M233 lie, as a property of the
  field that prevents it.**

`Launch` deliberately does not derive `Debug`: `dyn StreamFn` is not `Debug`, and a hand-written
shim would print nothing a reader could use. The test above uses `let … else` rather than
`expect_err` for that reason, and says so.

## A default that is a policy

`LaunchRequest::for_workspace` defaults to `Lsp::Off` and `Confinement::Auto`. The CLI passes
`--lsp auto`; the library default is "off", because a caller that did not ask for language-server
diagnostics should not spawn one. The difference is recorded in the type rather than left to be
rediscovered as a behaviour difference between the two frontends.

## Verification

**Battery `m234` — GATE PASS on attempt 1, 1,687 tests across 71 binaries, 0 failed**
(`audit/evidence/gate-m234.log`). Up from 1,683: the four launch tests, which are the whole point
of the slice. clippy clean across 13 crates; check-links 566/566; check-line-endings 829 files,
0 CRLF.

## What nothing exercises

- **`resolve_launch` against real credentials.** Every test here runs faux, offline, against a
  temporary workspace. The real-provider branches — the retry wrapper, the fallback chain, the
  key-source diagnostic — are moved code that has never executed in a test and are covered only
  by the CLI's own paths.
- **`LaunchRequest::fallbacks`.** Nothing constructs a non-empty chain in a test, so the
  `FallbackProvider` branch is unexercised here for the same reason.
- **Process-global state.** `install_providers` mutates a process-wide registry. Two tests in one
  process both run it, which happens to be harmless because neither declares a custom provider —
  a test that did would be order-dependent.
- **The trust store.** `resolve_launch` reads `~/.config/epsilon` and the project trust file; the
  tests use an empty temp workspace, so the trusted and untrusted branches are not distinguished
  by anything a reader can see in the test names.
