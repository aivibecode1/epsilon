# M224 — the OAuth table is read once, by the crate that owns the file

**Plan 019 step 2, and the half of A12 that named a parse rather than a path.** Two functions in
`epsilon-llm` each opened `config.toml` and each parsed the whole document for **one** key:
`client_id_from_config` (`oauth_login.rs:798`) and `enterprise_domain_from_config` (`:1180`). A login
therefore read the same file twice, and those two reads were the reason `epsilon-llm` carried a
`toml` dependency.

## What changed

| | before | after |
|---|---|---|
| `client_id_from_config(dir, provider)` | read + parse the file | **gone** |
| `enterprise_domain_from_config(dir, provider)` | read + parse the file | **gone** |
| — | — | `core::config::oauth_config_in(dir, provider) -> OAuthConfig` — **one** parse, both keys |
| `resolve_client_id(flow)` | env, then the file | `resolve_client_id(flow, &OAuthConfig)` — env, then the caller's value |
| `login(provider, ui, allow_callback)` | resolved both values itself | `login(provider, &OAuthConfig, ui, allow_callback)` |

**The value type lives in `epsilon-llm` and the read lives in `epsilon-core`**, which is the same
asymmetry M223 measured: `core` depends on `llm`, so the lower crate may name the type and the higher
one may fill it. The two callers (`main.rs:809`, `app.rs:863`) ask core for the value and hand it
down. Nothing else in the tree reads that table any more.

Two things come out of it that the plan did not mention: **one read instead of two** (a login that
parsed `config.toml` twice could see two different files if the user edited it mid-login), and the
impossibility of the two keys disagreeing about the same table.

## What moved with the code, and what it cost

The two tests that covered the old readers were **moved, not deleted, and kept their names** — in
`epsilon-core`, because that is where the read now is:

- `client_id_resolution_reads_the_oauth_config_table`
- `enterprise_domain_comes_from_the_config_table`

Keeping the names was not cosmetic. The repository has a guard,
`no_source_file_lost_a_test_since_head`, that compares **removed test names against added ones** in
the diff and fails on a net deletion — it exists because five tests once vanished while the suite
stayed green (M196). The first battery caught exactly this: the guard named both tests, because
their behaviour had moved under two new names in a different crate. A guard that fires when coverage
moves is doing its job, so the fix was to *name* the coverage, not to relax the guard.

**Two tests are new, and one of them covers something that had never been covered:**

- `the_environment_wins_then_the_configured_value_then_a_telling_error` — the resolution order. While
  the value came from a file, no test could say "the environment wins": the read always happened and
  the environment check sat above it. With the value handed in, the whole rule is a pure function.
  It asserts against whatever the runner's environment holds and skips only the unreachable branch,
  because clearing a variable needs `set_var` (unsafe in edition 2024) and this suite is threaded.
- `the_default_config_claims_nothing` — `OAuthConfig::default()` must not invent a credential.

## Verification

**Battery `m224-r3` GATE PASS** — `audit/evidence/gate-m224-r3.log`: **1,649 tests across 65
binaries** against M223's 1,646: **+3** net (two moved, two new, one test that had become redundant
with its pair). Gate shape: check-links 0 s, check-lint-tables 0 s, fmt 2 s, clippy 1 s, test 286 s,
demo 12 s, real-check 6 s. Run with `CARGO_PROFILE_DEV_DEBUG=0` / `CARGO_PROFILE_TEST_DEBUG=0`
(M208's disk knob).

The two earlier attempts are kept because they are the evidence for the two sentences above:
`gate-m224-FAILED-guard.log` is the guard firing on moved coverage, and
`gate-m224-r2-FAILED-clippy.log` is a `needless_borrows_for_generic_args` in the new test (two `&env`
where `env` is already a `String`).

**Where `toml` stands now:** `grep toml::` in `epsilon-llm/src` returns three hits, all in
`custom.rs` — the custom-provider reader (`:74`) and one test (`:161`). **`toml` cannot leave the
manifest until that one is gone**, which is plan 019 step 3, and the proof for step 4 is still
`cargo tree -p epsilon-llm -i toml` returning nothing.

## Limits

- **The enterprise-domain path is not covered end to end.** `login()` now takes the value rather than
  reading it, and the test asserts the reader in core and the ordering in llm — but nothing drives a
  Copilot device login with a configured domain, which was equally true before this change and is
  why the ceremony's own tests are the fixture-driven ones they are.
- **The `enterprise_domain_of` family is untouched** and still resolves the global directory itself
  (`auth.rs:158`). It reads the *credential store*, not `config.toml`, so it is not one of A12's
  three parses; it is, however, a second reader of the directory rule and the next thing M223's
  single-implementation test would flag if the rule ever moved again.
- **`providers/mod.rs` hand-parses `~/.aws/credentials`** (`:1101`, including its own `$HOME`
  resolution). That is AWS's file format rather than Epsilon's config, so it is out of this plan's
  scope and recorded here only so the next reader of that file knows it was seen and not missed.

## Nothing exercises

No test asserts that a **login with a configured client id and no environment variable** completes —
the ceremony needs a provider endpoint. What is pinned is the part that is pure: the read, the
resolution order, and the error's three instructions. The boundary between "resolved" and "logged in"
is unchanged by this milestone and untested before and after it.
