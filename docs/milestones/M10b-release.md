# M10b — Release packaging: real providers, model plumbing, changelog + tag

Date: 2026-08-23 · Status: completes M10. Epsilon v0.1.0.

## What was built

- **Real-provider wiring**: `--provider real --model <id>` routes every frontend
  (print/TUI/serve) through `epsilon_llm::GenaiAdapter` behind the existing `real`
  feature (forwarded from the CLI crate). genai resolves auth per model family from
  standard env vars (`ANTHROPIC_API_KEY`, …) — keys never stored, matching ADR-012.
  Without the feature, `--provider real` fails at startup with rebuild instructions.
- **Model-id plumbing** (behavior change, ADR-003 amendment): the loop previously
  hardcoded `"faux-1"` in three places (main stream call, compaction summarizer,
  VOLATILE prompt tier). `Agent::set_model_id` now feeds all three so real models are
  routed by name and the prompt stays byte-stable per configured id. Faux default is
  unchanged; prompt-hash tests still pass.
- **Release profile**: measured `epsilon.exe` at 8.76 MB with `lto="thin"` +
  `strip="symbols"` — under the 15 MB criterion without further tuning. Release binary
  smoke-tested (`-p "release smoke"` → exit 0).
- **CHANGELOG.md** added and linked from the README reading guide.

## Verification

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # 104 passed
cargo run -q -p epsilon-cli             # demo unchanged
cargo run -q -p epsilon-cli -- -p smoke # exit 0 (confined bash)
target/release/epsilon.exe -p "release smoke"   # exit 0; binary 8.76 MB < 15 MB
--provider real without feature → clean startup error (no panic)
```

Real-API smoke is deliberately NOT automated here: it would need paid keys. The genai
adapter carries its own mapping tests; end-to-end real runs are a manual step documented
in the README quick start.

## Known gaps / deferred ledger

- Windows ACL restricted-token runner — deferred with rationale (ADR-005 amendment).
- Linux landlock/bwrap runtime verification — code ships cfg-gated; needs a Unix host.
- rpc(stdio) mode + OpenAPI emission — editor-integration milestone.
