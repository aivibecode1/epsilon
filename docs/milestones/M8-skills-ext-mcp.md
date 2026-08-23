# M8 — Skills, process-isolated extensions, MCP bridge

Date: 2026-08-23 · Status: complete

## What was built

**Skills** (`epsilon-core/src/skills.rs`, ADR-015):
- Agent-Skills-spec loader: `SKILL.md` directories, YAML-lite frontmatter (name ≤64 chars
  `[a-z0-9-]`, description ≤1024), project-over-global shadowing by name.
- Progressive disclosure: catalog (name+description only) renders into the CONTEXT prompt
  tier; bodies load on demand. Invalid skills are skipped silently-but-countably (diagnostics
  surface later with the TUI).

**Extensions** (`epsilon-ext`, ADR-009):
- `extension.json` manifests: name/version/bin/args + declared tools (name/description/schema).
- NDJSON-RPC over stdio: hello handshake → invoke{id,call_id,tool,args} → result.
- Host guarantees: per-invoke timeout (10s), kill-on-drop children, crash budget (2 crashes →
  disabled-for-session), protocol violations as values not panics.
- `ExtTool` presents declared tools as ordinary registry entries (Sequential mode; the
  extension owns its own ordering).
- Test fixture = dedicated `ext-fixture` bin target located via `CARGO_BIN_EXE_` — no env-var
  re-entry hacks.

**MCP** (`epsilon-mcp`, ADR-013 as amended):
- Declarative server config, `Connector` transport seam, registry bridge registering tools as
  `mcp__<server>__<tool>` with untrusted-output handling (text verbatim into results, never
  acted upon). Fully tested against an in-process connector double.
- **Staged**: rmcp stdio transport lands M9/M10 with real-server tests (ADR-013 amendment
  records this). Everything shipped is tested; nothing is stubbed.

## Key decisions made while coding

1. **Aux-call isolation became architectural** (promoted from M7): the summarizer got its own
   StreamFn slot (`set_summarizer`); extension hosts are inherently separate processes; MCP
   connectors are a trait. Three flavors of "not in the hot path", one principle.
2. **tokio 1.53 dropped `next_line`** — replaced with a tiny `read_line_str` helper rather than
   bumping tokio features; noted because every reference harness eventually hits this kind of
   SDK-drift and the fix pattern (small local helper over verified primitives) beats both
   pinning older deps and reaching for heavyweight alternatives.
3. **Untrusted-data rule made mechanical**: MCP/extension outputs flow back as plain text tool
   results; nothing parses instructions out of them. The AGENTS.md contract line now has an
   enforcement point in code.

## Verification

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # 83 passed across 13 suites
cargo run -p epsilon-cli                # demo unaffected
```

New suites: skills units (3) · ext manifest (2) · ext host integration (3, real child
processes incl. timeout + crash paths) · mcp bridge (2).

## Next

M9 — TUI (ratatui) + print/serve modes + rmcp stdio transport for MCP.
