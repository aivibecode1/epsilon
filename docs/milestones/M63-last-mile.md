# M63 — the last mile

**Scope:** an audit of the *built binary* rather than the source, followed by the fixes its
findings demanded. No new capability was added; ten defects that made an otherwise complete
harness unusable by anyone but its author were closed, each with a regression test.

The full review lives in [`audit/`](../../audit): the usability rating
(`01-usability-review.md`), the defect ledger with reproductions (`02-defect-ledger.md`), the
consolidated port plan from the four reference harnesses (`03-port-plan.md`), the roadmap
(`04-roadmap.md`) and the raw evidence (`05-verification-log.md`).

## What

| # | Defect (evidence in the ledger) | Fix |
|---|---|---|
| D-01 | `epsilon --help` → `unknown argument: --help`, exit 2 | full help text in the binary; `-h`/`--help`/`help`, exit 0; a test asserts every parser flag appears |
| D-02 | `-p` printed `no final assistant text produced` for any non-clean ending, discarding the provider's error | `print_outcome()`: reason on stderr, partial text on stdout, exit 1; 6 unit tests |
| D-03 | `real` off by default → `cargo install` yielded a faux-only binary | `default = ["real"]`; `--no-default-features` for offline builds |
| D-04 | a model reference could resolve to a provider the user cannot call, silently | `resolve_model_for_run()` prefers a callable sibling and reports it; the CLI names provider, env var, login command, available credentials and a working suggestion |
| D-05 | tool descriptors came out of a `HashMap` → prompt bytes differed between runs | sorted descriptors + a regression test that builds two registries in different orders |
| D-06 | child processes inherited the harness environment | `epsilon_safety::env` scrub on all three spawn paths; `EPSILON_ENV_PASSTHROUGH` escape hatch |
| D-07 | spill files: predictable name, world-readable, failures swallowed | private root, exclusive create, `0600`, honest failure text |
| D-08 | output truncation kept only the tail (losing the first compiler error) | head+tail retention with exact omission counts, char-boundary safe |
| D-09 | a running command produced no output until it exited | `Sandbox::run` output sink → chunked pipe drains → throttled `ToolExecutionUpdate` → TUI status line |
| D-10/11 | no `LICENSE` though MIT is declared; build-probe junk in the repo root | added `LICENSE`; removed `probe_test_write` and `.tmp-empty-test/` |

Plus the documentation pass: README rewritten as an entry point with a table of contents,
`docs/README.md` as the map, `design/`/`plans/`/`reports/` moved under `docs/` (reports →
`docs/research/`), all cross-references rewritten and link-checked, `AGENTS.md` updated, and
`scripts/gate.sh` added so the whole gate battery is one command.

## Key decisions

- **Audit the binary, not just the code.** Every P0 above is invisible to `cargo test`: the
  suite was 459-green while `--help` did not exist and the default install could not reach a
  model. Running the artifact the user runs is a different activity from running the tests, and
  it is the one that found the defects worth fixing.
- **Feature flags are for capabilities, not for whether the product works.** `real` gated the
  entire network path behind a flag that the documented install did not pass. The offline build
  is now the opt-in one.
- **Report, never substitute silently.** Credential-aware model resolution and the credential
  env scrub both change what the user's command does; both say what they did. A harness that
  quietly runs a different model, or quietly empties a variable, is worse than one that fails.
- **A stream is a view; the tool result is the record.** The live-output path is throttled, tail
  bounded (4 KB per update, one update per 250 ms) and always secondary to the complete output
  returned by the call itself. Nothing downstream can come to depend on partial data.
- **Honest locators.** A tool result that says "full output saved to X" must have a file at X. If
  the spill fails, the result says the output could *not* be saved.
- **Keep the salvage rule uniform.** M62 kept partial assistant text at the loop and wire layers;
  M63 applies the same rule at the CLI boundary so a headless caller sees everything the model
  managed to say, flagged correctly.

## Test-caught bugs (the ones the new tests caught during this work)

1. **The credential-aware resolver had no case for "no callable sibling".** The first test
   asserted that `deepseek-chat` would move to the user's DeepSeek key; it does not, because the
   DeepSeek registry has no row containing that string. The test was wrong, not the code — and
   rewriting it produced the better rule: substitute only when a *true* alternative exists,
   otherwise explain. The CLI now suggests a concrete model instead of guessing one.
2. **The `real`-by-default change broke a TUI test** that asserted the "needs the real build"
   guidance. Feature unification means `epsilon-tui` compiles with `real` during a workspace test
   run, so the assertion had silently depended on how the workspace was built. It now asserts the
   invariant (OAuth is never mistaken for a pasted key; nothing is stored) and lets the message
   differ per build.
3. **`GIT_ASKPASS` is not a credential.** The env-scrub test asserted that a helper *path* would
   be scrubbed; it is not, and removing it would break a legitimate credential-helper workflow
   without protecting anything. The test now documents that boundary.
4. **A single huge line has no line to keep.** The first head+tail implementation produced an
   empty window for output that is one enormous line (minified JSON, a base64 blob) and returned
   only the omission marker. Caught by the char-boundary test; fixed with a byte-window fallback.
5. **Turning `real` on by default changed what CI lints.** The provider code compiled in every
   configuration for the first time and carried a `needless_borrow` that the previous default
   build never compiled. The local gate had been reporting "0 warnings" from a warm cache while a
   cold CI run would have failed on `-D warnings`. Lesson: after changing default features, run
   the lint gate *for the affected crate* with the feature explicitly enabled
   (`cargo clippy -p epsilon-llm --all-targets --features real`), not just the workspace default.

## Verification

```
$ bash scripts/gate.sh post
===== fmt --check =====      clean
===== clippy =====           0 warnings
===== test =====             532 tests, 0 failed
===== demo =====             completes
===== real-check =====       compiles
```

Behavioural checks with the built binary (raw transcripts in `audit/05-verification-log.md`):

- `--help` renders and exits 0; an unknown flag still exits 2.
- A real provider 401 now prints the provider's own message and exits 1 (previously a
  content-free `no final assistant text produced`).
- `--model deepseek-chat` with only a DeepSeek key prints the missing provider, the env var, the
  login command, the credentials that do exist, and `try: --model deepseek-flash (deepseek)`.
- A scripted `bash` call printing a credential-shaped variable returns `KEY=[]` while `PATH` and
  `USERPROFILE` survive — the scrub works through the real tool, not just in unit tests.
- An RPC client received three live `tool_execution_update` frames *during* a command that
  printed three timed lines, plus the approval request/decision pair for each call.

## Follow-ups (not in this milestone)

Live-provider end-to-end coverage for every wire (D-13), committing the M33–M62 body of work
(D-12), release artifacts (D-21), and the port plan's Tier 1 items — usage-anchored token
accounting, legal-cut compaction, verification enforcement and the stale-read guard. See
[`audit/04-roadmap.md`](../../audit/04-roadmap.md).
