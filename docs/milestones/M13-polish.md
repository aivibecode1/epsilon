# M13 — Polish: live catalog fetch, markdown-lite, expandable cards, recents, bwrap tests

Date: 2026-08-23 · Status: shipped. This closes every actionable item on the roadmap;
what remains requires hardware we do not have (a Unix host) or is genuinely future work.

## What was built

- **Live model listing** (`real` feature): `catalog::live_models(hint)` resolves any
  id to its provider family via genai's `AdapterKind::from_model` and fetches the
  provider's `/models` list. The TUI picker gains **Ctrl+F**: fetches for the
  highlighted row's provider, merges unknown ids under a `⟳ live` group (known ids
  dedupe into the catalog section), one fetch in flight at a time. The CLI injects the
  fetcher via a new `TuiConfig.live` hook so the TUI stays decoupled from genai.
- **Recents** (opencode convention, env-var-auth compatible): every applied model is
  persisted to `[picker] recent` in the global config (dedup, cap 8); the picker shows
  them in a `↻ recent` group when unfiltered.
- **Markdown-lite transcript rendering**: fenced code blocks render as dim gutter rows
  (`│ …` with `···` boundaries); inline `**`/`` ` `` markers stripped from prose.
  Deliberately not a full renderer — structure survives without span-per-fragment rows.
- **Expandable tool cards**: tool-result rows keep their full payload (`TranscriptLine
  ::full`); global `x` toggle switches between truncated heads and complete output.
- **bwrap integration tests** (`tests/bwrap_unix.rs`, `#[cfg(unix)]`): echo-under-
  sandbox sanity, host-/tmp isolation proof, root-read-only + workspace-writable probe.
  These are the runtime half of M10's "Linux confinement verified" criterion — they run
  on any Linux box with bubblewrap; Windows builds skip the file entirely.

## Test-caught notes

- **The `--features real` CLI path had never compiled**: `GenaiAdapter` was cfg-gated
  but never re-exported, and no gate run had ever enabled the feature on the cli crate.
  Caught immediately by building both feature configurations this milestone. Gates now
  should include `cargo check -p epsilon-cli --features real` going forward (recorded
  for AGENTS.md candidacy).
- PowerShell backtick-n corruption struck twice more before I switched permanently to
  structured edits for multi-line Rust — the lesson is now "never bulk-replace code".
- genai's `ProviderConfig` lives in `genai::resolver`, not `genai::adapter`.
- `VecStream` naming: fixture streams that implement EventStream directly keep tests
  free of async-runtime plumbing.

## Verification

```
cargo fmt --all -- --check                        # clean
cargo clippy --workspace --all-targets            # zero warnings
cargo test --workspace                            # 131 passed (was 129)
cargo build -p epsilon-cli --features real        # real path compiles (regression fix)
cargo run -q -p epsilon-cli                       # demo unchanged
cargo run -q -p epsilon-cli -- -p smoke           # exit 0
tests/bwrap_unix.rs                               # 3 tests, cfg(unix)-gated for CI
```

## Final ledger

Everything else already carries rationale: ACL runner (ADR-005/D02), full `(turn,step)`
retry-ledger keying (ADR-010 amendment), models.dev registry sync (catalog is data,
additive), credential store (contradicts ADR-012 by design), interactive trust re-prompt
(TUI candidate). Linux runtime verification moves to "runs when a Unix host/CI exists"
with the bwrap suite ready.
