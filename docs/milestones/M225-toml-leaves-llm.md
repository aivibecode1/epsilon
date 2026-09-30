# M225 — `toml` leaves `epsilon-llm`, and a config file is read once per process

**Plan 019 steps 3 and 4 — the part that makes the plan's proof possible.** `config.toml` is now read
in exactly one place (`epsilon-core::config`), and `epsilon-llm` no longer depends on a TOML parser.

## What was actually happening

```text
custom::load_custom_providers()          providers/mod.rs:1278   (try_stream_direct)
  ├─ opens config.toml                  providers/mod.rs:1350   (DirectProvider::stream — every request)
  ├─ parses the whole document          fallback.rs:97          (fallback chain's key resolution)
  └─ returns rows                       main.rs:1279, :1384    (CLI composition)
                                         tui/app/session.rs:60  (TUI composition)
```

**A configuration file was opened and parsed on the hot path of every streamed request**, by a crate
that is not allowed to own the parse. The plan's step 3 said "custom providers arrive parsed", which
is the right sentence and did not say how; reading the call graph first is what turned it into a
decision rather than a slogan.

## The decision, and the alternative that was rejected

**Rejected: threading `&[CustomProvider]` through.** Measured first. The readers are
`DirectProvider::stream` (behind the `StreamFn` trait), the fallback chain's key resolution, and
`try_stream_direct`. A value crossing there has to cross the trait boundary into every stream call —
configuration in the middle of the request path, in a trait implemented by the server and the TUI as
well as by core.

**Chosen: install once, read anywhere.** `custom::install_providers(rows)` is called exactly once, in
`dispatch()` before anything reads; `custom::custom_providers()` is what the five internal readers
call. The parse lives in `core::config::custom_providers_in`, and the value type stays in `llm`
because `epsilon-core` depends on `epsilon-llm` and not the other way round — M223's asymmetry again,
now used for the second thing it settles.

**Why a process value is defensible here, stated plainly:** this is *composition state*, not
resolution. It is written once, by the process that owns configuration, before any run; it is never
written again; and the failure mode of getting it wrong is loud rather than silent — a custom id that
cannot be resolved says so at the point of use. That is the same shape as `Glyphs::detect()`
(M219) and the theme, and the opposite of what ADR-012's amendment warned against for the *directory*
— where a second writer really could answer differently.

## The proof the plan asked for

```text
$ cargo tree -p epsilon-llm -i toml
error: package ID specification `toml` did not match any packages
```

Not "no dependents of `toml` from llm" — `toml` is not in that crate's graph **at all**. The manifest
edit is a claim; this is the measurement, and it is the same shape M223 got when the LSP edge left
`epsilon-tools`.

## What moved with it

The two tests that covered the parse are in `epsilon-core` now, **with their original names**
(`load_from_toml_dir_sorted_and_tolerant`, `missing_file_reads_empty`) — same reasoning as M224: the
guard `no_source_file_lost_a_test_since_head` keys on names, and a guard that cries wolf gets disabled.
`missing_file_reads_empty` gained the two cases the old version could not state (a config with no
`[provider_custom]` table, and an unparseable file), because the reader is now reachable from tests in
a crate that can parse as well as read.

One new test in `epsilon-llm`: `installing_replaces_the_previous_set` — the contract the hot-path
readers now depend on, including that a second install **replaces** rather than appends, so a test can
install twice and see what it installed.

## Verification

**Battery `m225-r2` GATE PASS** — `audit/evidence/gate-m225-r2.log`: **1,650 tests across 65
binaries** against M224's 1,649: **+1**. Gate shape: check-links 0 s, check-lint-tables 0 s, fmt 2 s,
clippy 32 s, test 296 s, demo 13 s, real-check 6 s. Run with `CARGO_PROFILE_DEV_DEBUG=0` /
`CARGO_PROFILE_TEST_DEBUG=0` (M208's disk knob). The first attempt failed on `fmt --check` alone — an
edit after the last format — and is kept as `audit/evidence/gate-m225-FAILED-fmt.log`.

## Limits

- **A test that exercises the hot path with custom providers installed does not exist.** The
  readers ask for an installed value, and the install/accessor contract is pinned; nothing drives a
  `DirectProvider::stream` with a custom row in place. That was equally true before, when the path read
  the file itself, so this is not a new gap — but it *is* the gap that made the file read invisible, and
  it is worth closing.
- **`try_stream_direct` and the fallback chain now depend on composition having run.** In a test that
  builds those paths without going through `dispatch()`, custom ids will not resolve where they used
  to. No current test does that (the suites pass), and the failure is a visible "no API key for X".
- **The project-config parse (plan 019 step 5) is still in the CLI**, in `runtime.rs`, parsing
  `.epsilon/config.toml` for the trust hash. That is a *different file* from the global one, and it
  is the last reader in the tree — but it is in the wrong crate for the rule this milestone
  established.

## What nothing exercises

Nothing asserts that **the CLI installs before anything reads** — that ordering is one line in
`dispatch()` above the diagnostics collector, and it is the whole reason the hot path sees a value.
The failure it prevents would be "a custom id resolves in `dispatch` and not in `stream`", which is
exactly the kind of split-brain this repository treats as a defect, and no test would catch it.
