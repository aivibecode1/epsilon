# M18 — Alignment pass: docs audit, pi parity closure, plan mode

Date: 2026-08-24 · Status: shipped in this session. Trigger: the project's three
priority list — documentation quality first, then codebase alignment + reference
research, then a full re-audit with /improve and ponytail.

## What landed

### 1. Documentation audit and repair (phase 1)

Every `.md` in the repo was re-read against a four-part standard (scope/goals, what is
implemented, problems-and-solutions, decisions-and-reasoning). Findings and fixes:

- **M14 had no dev log** — the milestone jumped M13 → M15 in `docs/milestones/` even
  though commit `73eda8d` exists. Backfilled `docs/milestones/M14-harness-maturity.md`
  reconstructed from the commit diff (TUI-by-default, provider auto-detect from env
  keys, slash-command repairs on the real key path). The log says plainly that it was
  backfilled and cites its evidence.
- **Stale genai-era claims survived M17** in living docs: README quick start still said
  "pulls genai 0.6.5", the crate table still listed a `GenaiAdapter`, architecture §9
  still described wrapping genai, ADR-003 had no supersession note, ADR-018 called
  replacing genai "a candidate future milestone" that M17 had already completed,
  qa/01-why-rust carried two stale rows. All corrected; historical mentions inside old
  milestone logs were left alone (they were true when written — logs are records, not
  living claims).
- **Fact drift**: README claimed Rust ≥ 1.85 while workspace/CI enforce 1.88 (fixed);
  quick-start test count "90+" vs actual suite (aligned); M14 missing from the status
  table (added).
- **Beginner accessibility layer**: new `docs/qa/06-agent-harness-primer.md` explains
  what an agent harness IS, walks one Epsilon turn step by step, glossaries the jargon
  (steering, compaction, prompt cache, JSONL, gate, sandbox, StreamFn, faux provider),
  and gives newcomers an exploration order. Wired into both README reading paths.
- **Two doc overclaims corrected against code truth**: architecture §4 promised an
  edit-tool "diff preview + file-mutation queue" that does not exist (ordering actually
  comes from the executor's sequential-barrier rule) and §13 promised tracing-crate
  spans + OTLP export that were never built (observability today = event bus + session
  log). Docs now state shipped reality; the deferrals are recorded rather than silent.

### 2. Pi alignment report + gap closure (phase 2a/2b)

New `docs/research/pi-alignment-2026-08.md`: layer-by-layer correspondence table between
pi-0.84.2 packages and Epsilon crates, verified by reading both trees this session.
Post-M17 verdict: aligned layer-for-layer at semantics level with three ADR'd
divergences. Gaps found: G1 session search (CLOSED here), G2 eval scenarios (deferred,
rationale written), G3 telemetry/tracing (docs corrected; crate remains future work),
G4 edit mutation queue (doc fix only), G5 protocol crate split (deferred,
brick-earns-existence), G6 env probing (absorbed already).

**G1 closed**: new `epsilon-session/src/search.rs` — case-insensitive scan over user /
assistant (text+thinking+tool-call args+error message) / tool-result text, newest hits
first, limit + active-branch-only options, ±40-char snippets on char boundaries, one
hit per entry. Surfaced as `/search <query>` in the TUI (command variant, slash
registry entry with description, help line) routed through `Agent::search_history`.
The search surface is re-exported from epsilon-core so epsilon-tui keeps ZERO direct
dependency on epsilon-session (architecture law preserved: tui -> core -> session).

Tests: 6 unit tests on the search module (roles/newest-first/snippet context/
empty-query misses/limit/non-message entries skipped) + 1 TUI key-path test seeding a
real store and asserting hit lines through handle_key.

### 3. Reference research pass 2 + import (phase 2c/2d)

New `docs/research/research-pass-2-2026-08.md`. Community signal gathered: r/PiCodingAgent,
HN threads (pi launch + opencode thread), DSH GitHub discussion, independent DSH
review. Headline findings: users confirm pi's grounded-minimal behavior; pi's most-
missed feature is PLAN MODE; HN endorses sandbox-over-permission-theater (Epsilon
ships both layers); DSH third-party review confirms our "developer preview, sandbox
not a security boundary for self-modification" reading.

**Plan mode imported from opencode** (`plan` agent pattern):

- `epsilon_safety::plan_ruleset()` — write/edit DENIED outright, `git commit*` /
  `git push*` bash patterns denied. Deny is monotonic across layers, so no project or
  CLI rule can punch through: plan mode is a boundary, not a suggestion.
- CLI `--plan` flag threaded through all four frontends (print/tui/serve/rpc share
  build_runtime); startup banner states the posture in plain words.
- Live-verified against the faux provider: with `--plan` the scripted write is denied
  and the target file never exists; without it, the same script writes the file
  (control proves the block is plan-mode's doing).
- Tests: engine-level (denies even when a project layer re-allows; commit/push
  blocked; reads/grep stay free; ordinary shell unchanged at ask).

## Test-caught bugs / gotchas this round

- `StopReason::Stop` does not exist (`EndTurn`) — fixture bug, caught by compile.
- Plan ruleset first draft tried to allow `.epsilon/*` writes under a `write *` deny;
  monotonic deny correctly refused to let the allow win. The exception was unnecessary
  anyway: internal session bookkeeping never travels through the tool gate. Design and
  tests simplified together.
- The store uses std Mutex, not tokio Mutex — test fixture initially used the wrong
  type (compile-caught).
- Clippy useless-conversion in the search fixture (`.into()` on an already-String id).

## Verification

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # 213 passed, 0 failed (was 205)
cargo run -q -p epsilon-cli -- --demo   # completes, reason Completed
--plan live smoke                       # write denied + banner; control run writes
```

Suite delta: +8 (search module 6, TUI key-path 1, safety engine +2, minus none).

## Known limits / next

- /search scans sequentially via read_entry per seq — fine at thousands of entries;
  if sessions reach tens of thousands, consider a cached parse pass (recorded in the
  alignment report, not built).
- Plan mode currently blocks write/edit/commit/push; other mutating shell commands
  still follow default ask posture (deliberate: research runs can still run read-only
  shell like cargo test).
- Full re-audit (/improve deep + ponytail) runs immediately after this log lands;
  results become docs/plans/ entries.
