# M226 — the last uncalled duplicate, and a plan step retired with its reason

**Plan 019 step 5, and the plan is now closed.** A12's row had five steps across four milestones;
two of them were mis-shaped when written, and this is the one where reading the code instead of the
plan found something the plan had not seen.

## What the step said, and what is true

> "`runtime.rs:138` parses `.epsilon/config.toml` into `toml::Value` for the trust hash. That parse is
> **two** facts: the file's bytes (for the hash) and its contents (for the settings). Move the parse
> next to the hash in `core::config` so 'what the hash covers' and 'what the settings are' cannot
> drift."

**They cannot drift.** `load_project_config` reads the file once — `raw` at line 132 — and uses that
one string for the hash (line 141) *and* the parse (line 175). The two facts are the same bytes by
construction. Moving the parse would make them the same bytes *by proximity*, which is a weaker
property wearing the same coat.

And the move was awkward in the direction that matters: `ProjectConfig` holds
`Vec<epsilon_mcp::McpServerConfig>`, and `epsilon-core` does not depend on `epsilon-mcp` (mcp depends
on core). Core could only have returned its own row types for the CLI to convert — a new type boundary
in exchange for nothing.

## What was actually there

Reading the call graph instead of the plan turned up a **seventh instance of the "no production
caller" class**, the one this repository documents most:

```text
crates/epsilon-cli/src/runtime.rs:328     current_project_config_hash(workspace)   ← nothing called it
crates/epsilon-core/src/config.rs:237     current_project_config_hash_in(workspace)  ← the TUI uses this
```

Two implementations of the same fact, 140 lines from the parse that also computes it, in two crates
that *can* see each other — and the copy nothing called was the one in the higher crate.

**The order matters: they were compared before one was deleted.** A test ran both against a real
`.epsilon/config.toml` and asserted they agree; only then was the uncalled one removed. A duplicate
that differed would not have been a duplicate, it would have been a second answer — and deleting the
wrong one would have changed which digest a trust prompt records.

## What is pinned now

The *fact* the pair was answering is a test rather than a coincidence of statement order:

- `the_recorded_hash_covers_the_parsed_bytes` — the digest recorded for trust is over exactly the
  bytes the settings were parsed from, **and one byte later is a different digest**, which is the
  reason the file is hashed at all.
- `a_workspace_without_a_project_config_has_no_hash` — the "never written" case, which is how a
  caller tells it from "written and accepted".

## Verification

**Battery `m226` GATE PASS** — `audit/evidence/gate-m226.log`: **1,652 tests across 65 binaries**
against M225's 1,650: **+2**, the two tests above. Gate shape: check-links 0 s, check-lint-tables 0 s,
fmt 2 s, clippy 7 s, test 208 s, demo 5 s, real-check 2 s. Run with `CARGO_PROFILE_DEV_DEBUG=0` /
`CARGO_PROFILE_TEST_DEBUG=0` (M208's disk knob).

**One failure worth recording, because it was mine and it was informative:** the first version of the
hash test asserted `notices.is_empty()` for a valid config — and it failed, because first-time trust
*does* emit a notice ("trusting project config … (hash recorded)"). The behaviour was right and my
assertion was wrong: what a valid file must not produce is a *complaint*, not a notice. The test now
asserts that, which is a more useful statement anyway.

## Limits

- **The project-config parse stays in the CLI**, and it should: it is the one place that reads
  `.epsilon/config.toml`, its types need `epsilon-mcp`, and its trust decision belongs to the frontend
  that asks the user. What this milestone retired is the *justification* for moving it, not the code.
- **Nothing asserts the project config is read exactly once per launch.** `doctor` calls
  `load_project_config` twice (`:298` and `:387`) and `main.rs` once at startup, so a doctor report
  re-reads a file it could have reused — harmless today, and a candidate for the next pass on
  `doctor`.
- **A12's row now describes four milestones of work** (M223–M226) whose net effect is: one reader
  crate, one path rule, one hash implementation, and no `toml` in `epsilon-llm`.

## Nothing exercises

The deletion rests on "nothing called it", which was established by a grep that no test repeats. A
reader who re-adds a caller to a deleted function gets a compile error, so the risk is not a silent
regression but a re-introduction that looks deliberate. The test that would catch the *other* half — a
second implementation appearing in a third crate — is M223's
`the_global_directory_is_computed_in_exactly_one_place`, which walks the workspace for `$HOME` reads;
it does not walk for `current_project_config_hash`, because there is nothing left to walk for.
