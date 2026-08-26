# Epsilon

<p align="center">
  <img src="assets/Logo.png" alt="Epsilon logo" width="800">
</p>

A coding-agent harness in Rust. You type a prompt; the model reads your files,
edits them, runs commands, and keeps going until the job is done. Every tool
call passes permission rules and an OS sandbox first. Ships as a single static
binary.

Built by reading four open-source harnesses at source level and porting the
best of each. No code copied; every borrowed idea is cited where it appears.

- **pi** gave us the skeleton: two-level loop, steering queues, JSONL session
  trees ([github.com/earendil-works/pi](https://github.com/earendil-works/pi))
- **DeepSeek Harness (DSH)** gave us the armor: fail-closed OS confinement,
  bounded tool pools
  ([github.com/deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness))
- **Hermes Agent** gave us the discipline: byte-stable prompt tiers, replay
  fidelity ([github.com/NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent))
- **opencode** gave us the manners: wildcard permission gates, auth store,
  plan mode ([github.com/anomalyco/opencode](https://github.com/anomalyco/opencode))

Full credit table: [Credits and references](#credits-and-references).

---

## Quick start

You need Rust 1.88 or newer ([rustup.rs](https://rustup.rs) if you don't have it).

```sh
git clone <this-repo> epsilon && cd epsilon
cargo build --workspace              # compiles all 11 crates
cargo test --workspace               # full suite, offline and free
cargo run -p epsilon-cli -- --demo   # scripted demo, proves the loop works
```

Builds, tests, and demos never touch a paid API. They run against the **faux
provider**, a deterministic scripted LLM (pi tests its loop the same way), so
CI stays free and instant.

### Using real models

```sh
cargo build --release -p epsilon-cli --features real   # enables direct HTTP wires
epsilon auth login anthropic                           # stores a key once
target/release/epsilon                                 # finds it and runs REAL
```

Keys live in `~/.config/epsilon/auth.json`; environment variables like
`ANTHROPIC_API_KEY` work too (stored wins). Model selection reads a bundled
catalog of 386 models with real prices (generated from
[models.dev](https://models.dev)), so browsing needs no network and no key.
Retryable failures can walk fallback chains via repeatable `--fallback <model>`.

### Custom endpoints (LM Studio, vLLM, any OpenAI-compatible server)

Declare it once in `~/.config/epsilon/config.toml`:

```toml
[provider_custom.lmstudio]
base_url = "http://localhost:1234/v1"
# env_key = "LMSTUDIO_API_KEY"   # optional; omit for keyless local servers
```

Then use `<name>/<model>` ids like any other model:

```sh
target/release/epsilon --model lmstudio/qwen2.5-7b-instruct -p "hi"
```

Catalog ids always win over same-named custom rows.

### Everyday commands

```sh
epsilon                                  # interactive TUI (default)
epsilon -p "summarize this repo"         # headless print mode, reply to stdout
epsilon --serve --port 8787              # HTTP/SSE server
epsilon --rpc                            # NDJSON over stdio for embedding
epsilon --tui                            # explicit TUI
epsilon --list-models                    # show the bundled catalog
epsilon --continue -p "next step"        # resume newest session
```

(When running from source instead of the installed binary, prefix these with
`cargo run -p epsilon-cli --`.)

---

## What's inside

Epsilon reads, writes, edits, greps, globs, lists, and runs shell commands
inside its declared workspace. Two gates guard every call: permission rules
(last match wins), then OS confinement — Job Objects with memory/process/CPU
caps on Windows, bubblewrap on Unix. If the sandbox cannot start, the call
fails closed. It never runs free.

Long sessions auto-compact past a token threshold while the transcript on disk
keeps every line. Skills load from `SKILL.md` directories on demand.
Extensions run as separate processes; MCP servers attach over stdio.

Eleven crates with compile-enforced dependency direction (nothing depends
upward):

```
contracts ← llm ← core ← {tools, safety, ext, mcp} ← {tui, server} ← cli
contracts ← session ← core
```

| Crate | Role |
|---|---|
| `epsilon-contracts` | Shared vocabulary: types and traits only, zero logic |
| `epsilon-llm` | Provider layer behind the never-fail contract: three direct HTTP wires, faux provider, retry, fallback chains, catalog |
| `epsilon-session` | Append-only JSONL tree sessions |
| `epsilon-core` | The loop: queues, batch executor, event bus, prompts, compaction |
| `epsilon-tools` | Built-in tools: read/write/edit/bash/grep/glob/ls |
| `epsilon-safety` | Permission rulesets and sandbox backends |
| `epsilon-ext` | Process-isolated extension host |
| `epsilon-mcp` | MCP client bridging external tools |
| `epsilon-tui` | Terminal frontend: live transcript, dialogs, picker, palette |
| `epsilon-server` | HTTP/SSE frontend |
| `epsilon-cli` | Binary: mode dispatch and runtime composition |

A few mechanisms worth knowing by name:

- **Never-fail provider contract**: backends cannot throw; even panics become
  a normal terminal event, so the loop handles exactly one error shape.
- **Byte-stable prompt tiers**: identical inputs produce byte-identical
  prompts, hash-tested, so the provider's prompt cache keeps hitting. This is
  the economic core of the design.
- **Fail-all-on-truncation**: token-limit cutoff fails every tool call in that
  message, because truncated JSON arguments look plausible and execute wrong.
- **Shadowed model view**: compaction rewrites what the model sees; the JSONL
  on disk keeps every original line. Audit intact, context small.
- **Turn leases + crash repair**: CLI/TUI/server share session files safely;
  crashes repair idempotently on load.

The full tour lives in [`design/epsilon-architecture.md`](design/epsilon-architecture.md).

---

## Testing

249 tests offline (264 with `--features real`), zero clippy warnings in both
build configurations. CI runs all gates on Windows, a bubblewrap confinement
suite on Linux, and an MSRV 1.88 check. A weekly job regenerates the bundled
catalog against live models.dev and opens a PR when it drifted.

Everything above stays offline and free: the faux provider covers full-loop
integration tests, wire parsing is pinned by fixture tests recorded from real
HTTP exchanges, and real-provider checks sit behind env-var-gated `#[ignore]`.

---

## Documentation map

| You are... | Read this |
|---|---|
| New to coding agents | [`docs/qa/06-agent-harness-primer.md`](docs/qa/06-agent-harness-primer.md): what a harness even is, no background assumed. Keep [`docs/glossary.md`](docs/glossary.md) open beside it; it defines every term from StreamFn to shadow view. |
| A developer joining | [`AGENTS.md`](AGENTS.md) (developer contract), [`CHANGELOG.md`](CHANGELOG.md) (what shipped when), [`docs/history.md`](docs/history.md) (milestone timeline + decision index), [`design/epsilon-architecture.md`](design/epsilon-architecture.md) (components and data flow), [`docs/milestones/`](docs/milestones) (dev logs in order). |
| Researching harness design | [`reports/harness-analysis.md`](reports/harness-analysis.md): source-level audit of pi, DSH, Hermes, opencode with file-anchored strengths and weaknesses. [`reports/comparative-analysis.md`](reports/comparative-analysis.md) ranks who does what best. |
| A coding agent working here | [`AGENTS.md`](AGENTS.md) loads automatically; the newest entry in [`docs/milestones/`](docs/milestones) tells you the current state. The shipped product contract lives at [`crates/epsilon-core/assets/agent-contract.md`](crates/epsilon-core/assets/agent-contract.md). |

Contributing? Every change must pass:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets   # zero warnings tolerated
cargo test --workspace                   # full suite green
```

House rules: dependencies pinned exactly (`=x.y.z`); no logic in `contracts`;
architectural violations are build errors, not review comments; behavior
changes need an ADR note in [`design/epsilon-decisions.md`](design/epsilon-decisions.md);
commits look like `feat(mN): ...`, `fix(mN): ...`, or `docs: ...`.

---

## Credits and references

Epsilon is a learning project built by reading four open-source coding agents and
reimplementing the best of what we found. No code was copied; every idea is cited
where it appears in this repo, and the Rust implementation is written from scratch.
We studied pinned, unmodified snapshots of each harness (kept locally for reading;
they are not part of this repository).

| Harness | Source | What Epsilon took | Where it landed |
|---|---|---|---|
| **pi** (v0.84.2) by Mario Zechner / Earendil Works | [github.com/earendil-works/pi](https://github.com/earendil-works/pi) · [pi.dev](https://pi.dev) | The skeleton: two-level agent loop, steering and follow-up queues, never-fail `StreamFn` contract, fail-all-on-truncation, JSONL session trees with compaction shadow views, turn leases, model picker and cycling semantics, fuzzy matcher, thinking levels carried per session | `epsilon-core`, `epsilon-llm`, `epsilon-session`, `epsilon-tui` throughout |
| **DeepSeek Harness "DSH"** (v0.1.1-rc.2) by DeepSeek | [github.com/deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness) | The armor: fail-closed OS confinement seam (Job Objects on Windows, bubblewrap on Unix), bounded tool pools, sticky turn reasons, plugin-style process isolation for extensions | `epsilon-safety`, `epsilon-ext`, loop executor in `epsilon-core` |
| **Hermes Agent** (2026.8.19) by Nous Research | [github.com/NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent) | The discipline: byte-stable three-tier prompt assembly, replay fidelity via request hashing, cross-process turn leases, TTL health probes for optional tools (M24), error-classified fallback chains with per-turn primary restoration (M24), incident-driven documentation culture | `epsilon-core` prompts, `epsilon-session` leases, `epsilon-mcp` probes, `epsilon-llm/src/fallback.rs`, `docs/qa/` |
| **opencode** (v1.18.21) by Anomaly (formerly SST) | [github.com/anomalyco/opencode](https://github.com/anomalyco/opencode) | The manners: wildcard permission rulesets with last-match-wins ask/deny/allow, auth store with stored-key-wins precedence, picker favorites and two-stage selection, doom-loop brake, plan mode, structured compaction UX | `epsilon-safety` rulesets, `epsilon-llm/src/auth.rs`, `epsilon-tui` picker, plan mode in `epsilon-core` |

More credits worth naming: the [models.dev](https://models.dev) catalog feeds our
bundled model data through `scripts/regen_catalog.py`; the [improve](https://github.com/shadcn/improve)
skill (by shadcn) drives our audit-and-plan loop behind every improvement
milestone; and the [ponytail](https://github.com/DietrichGebert/ponytail) skill
(by Dietrich Gebert) keeps every change at the laziest solution that actually
works — this repo's smallest diffs are its fingerprints. Full source-level audits:
[`reports/harness-analysis.md`](reports/harness-analysis.md) and
[`reports/comparative-analysis.md`](reports/comparative-analysis.md).

---

*This repo is a research-through-implementation project. The harnesses credited
above belong to their authors; we cite their lessons and don't copy their code.*
