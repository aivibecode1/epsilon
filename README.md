# Epsilon

<p align="center">
  <img src="assets/Logo.png" alt="Epsilon logo" width="800">
</p>

A coding-agent harness in Rust. You type a prompt, the model reads your files, edits them, runs commands, and keeps going until the job is done. Every tool call passes permission rules and an OS sandbox first. One static binary, works offline in tests, talks to real models when you ask it to.

Built by reading four open-source harnesses at source level and porting the best of each. No code copied, every idea cited where it appears.

- **pi** (v0.84.2) gave the skeleton: two-level loop, steering queues, JSONL session trees — [github.com/earendil-works/pi](https://github.com/earendil-works/pi)
- **DeepSeek Harness (DSH)** (v0.1.1-rc.2) gave the armor: fail-closed OS confinement, bounded tool pools — [github.com/deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness)
- **Hermes Agent** (2026.8.19) gave the discipline: byte-stable prompt tiers, replay fidelity — [github.com/NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent)
- **opencode** (v1.18.21) gave the manners: wildcard permission gates, auth store, plan mode — [github.com/anomalyco/opencode](https://github.com/anomalyco/opencode)

Full credit table at the bottom. The tour lives in [`design/epsilon-architecture.md`](design/epsilon-architecture.md).

---

## Quick start

You need Rust 1.88 or newer ([rustup.rs](https://rustup.rs)).

```sh
git clone <this-repo> epsilon && cd epsilon
cargo build --workspace              # all 11 crates
cargo test --workspace               # full suite, offline and free
cargo run -p epsilon-cli -- --demo   # scripted demo, proves the loop
```

Tests and demos never touch a paid API. They run against the **faux provider**, a deterministic fake LLM. Same idea pi uses, so CI stays free and fast.

### Real models

```sh
cargo build --release -p epsilon-cli --features real
epsilon auth login anthropic         # saves the key once
target/release/epsilon              # finds it and runs with real models
```

Keys live in `~/.config/epsilon/auth.json`. Environment variables like `ANTHROPIC_API_KEY` also work, stored keys win. A bundled catalog of 386 models with real prices (generated from [models.dev](https://models.dev)) lets you browse with no network and no key. Retryable failures can walk fallback chains with `--fallback <model>` repeatable.

### Custom endpoints

For LM Studio, vLLM, or any OpenAI-compatible server, declare it once in `~/.config/epsilon/config.toml`:

```toml
[provider_custom.lmstudio]
base_url = "http://localhost:1234/v1"
# env_key = "LMSTUDIO_API_KEY"   # optional for local servers
```

Then use it like any other model:

```sh
target/release/epsilon --model lmstudio/qwen2.5-7b-instruct -p "hi"
```

Bundled catalog ids always win over same-named custom rows.

### Everyday commands

```sh
epsilon                                  # TUI, the default
epsilon -p "summarize this repo"         # print mode, reply to stdout
epsilon --serve --port 8787              # HTTP/SSE server
epsilon --rpc                            # NDJSON over stdio for editors
epsilon --tui                            # explicit TUI
epsilon --list-models                    # bundled catalog
epsilon --continue -p "next step"        # resume newest session
```

From source, prefix with `cargo run -p epsilon-cli --`.

---

## How it works

Two gates guard every tool call. First, permission rules (last match wins). Then OS confinement: Job Objects on Windows, bubblewrap on Unix. If the sandbox cannot start, the call fails closed. It never runs free.

Long sessions auto-compact past 80% of the context window. The human transcript on disk keeps every line, the model sees the compacted view. Skills load from `SKILL.md` folders on demand. Extensions run as separate processes, MCP servers attach over stdio.

Slash commands in the TUI: `/models` and `/model <id>`, `/think <level>`, `/compact` to reclaim context, `/timeline` to browse branches, `/fork` to copy the session to a new branch, `/search <query>`, `/clear`, `/help`, `/quit`. Typing `@` in the prompt will complete file paths. `/export` writes the transcript to HTML or JSON.

Prompt memory: `MEMORY.md` at the workspace root and `~/.config/epsilon/MEMORY.md` are injected into the CONTEXT tier alongside `AGENTS.md`. They survive compaction and are hash-tagged for cache stability (Hermes pattern, added in m31).

Eleven crates, dependency direction enforced at compile time:

```
contracts <- llm <- core <- {tools, safety, ext, mcp} <- {tui, server} <- cli
contracts <- session <- core
```

| Crate | What it does |
|---|---|
| `epsilon-contracts` | Types and traits only, zero logic |
| `epsilon-llm` | Provider layer behind the never-fail contract: three HTTP wires, faux provider, retry, fallback, catalog |
| `epsilon-session` | Append-only JSONL tree sessions, leases, repair |
| `epsilon-core` | The loop: queues, batch executor, event bus, prompts, compaction |
| `epsilon-tools` | Built-ins: read, write, edit, bash, grep, glob, ls, todo, task. webfetch is Ask-gated and ships next (rule already in safety) |
| `epsilon-safety` | Permission rulesets and sandbox backends |
| `epsilon-ext` | Process-isolated extension host |
| `epsilon-mcp` | MCP client |
| `epsilon-tui` | Terminal frontend: live transcript, picker, palette, slash autocomplete |
| `epsilon-server` | HTTP/SSE frontend |
| `epsilon-cli` | Binary: mode dispatch and composition |

A few ideas worth naming:

- **Never-fail provider contract**: backends cannot throw, even panics become a normal terminal event, so the loop has one error shape.
- **Byte-stable prompt tiers**: identical inputs give byte-identical prompts, so provider caches keep hitting. This is the economic core.
- **Fail-all-on-truncation**: token-limit cutoff fails every tool call in that message, truncated JSON looks plausible and would run wrong.
- **Shadow view**: compaction rewrites what the model sees, the JSONL on disk stays complete.
- **Turn leases and crash repair**: CLI, TUI, and server share session files safely, crashes repair on load.

---

## Testing

Offline by default, zero warnings tolerated:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
cargo test --workspace
cargo check -p epsilon-cli --features real
```

CI runs the same gates on Windows, a bubblewrap suite on Linux, and an MSRV 1.88 check. A weekly job regenerates the catalog against live models.dev and opens a PR when it drifts.

---

## Where to read next

| You are | Read this |
|---|---|
| New to coding agents | [`docs/qa/06-agent-harness-primer.md`](docs/qa/06-agent-harness-primer.md) — what a harness is, no background needed. Keep [`docs/glossary.md`](docs/glossary.md) open beside it. |
| Joining as a developer | [`AGENTS.md`](AGENTS.md), [`CHANGELOG.md`](CHANGELOG.md), [`docs/history.md`](docs/history.md), [`design/epsilon-architecture.md`](design/epsilon-architecture.md), [`docs/milestones/`](docs/milestones) in order. |
| TUI keys and behavior | [`docs/qa/07-tui-keymap-audit.md`](docs/qa/07-tui-keymap-audit.md) |
| Researching harness design | [`reports/harness-analysis.md`](reports/harness-analysis.md) and [`reports/comparative-analysis.md`](reports/comparative-analysis.md) — source-level audits with file anchors. |

House rules: deps pinned exact (`=x.y.z`), no logic in `contracts`, architecture violations are build errors, behavior changes need an ADR in [`design/epsilon-decisions.md`](design/epsilon-decisions.md), commits look like `feat(mN): ...` or `docs: ...`.

---

## Credits and references

No code copied, Rust written from scratch, every borrowed idea cited inline.

| Harness | Source | What Epsilon took | Where it landed |
|---|---|---|---|
| **pi** (v0.84.2) | [github.com/earendil-works/pi](https://github.com/earendil-works/pi) · [pi.dev](https://pi.dev) | Skeleton: two-level loop, steering and follow-up queues, never-fail StreamFn, fail-all-on-truncation, JSONL trees with shadow views, turn leases, picker and fuzzy matcher, thinking levels | `epsilon-core`, `epsilon-llm`, `epsilon-session`, `epsilon-tui` |
| **DSH** (v0.1.1-rc.2) | [github.com/deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness) | Armor: fail-closed confinement (Job Objects / bubblewrap), bounded pools, sticky turn reasons, process isolation | `epsilon-safety`, `epsilon-ext`, `epsilon-core` executor |
| **Hermes Agent** (2026.8.19) | [github.com/NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent) | Discipline: byte-stable tiers, replay fidelity, cross-process leases, TTL probes, error-classified fallbacks, MEMORY.md tier (m31) | `epsilon-core` prompts, `epsilon-session` leases, `epsilon-mcp`, `epsilon-llm/src/fallback.rs` |
| **opencode** (v1.18.21) | [github.com/anomalyco/opencode](https://github.com/anomalyco/opencode) | Manners: wildcard rulesets (last-match-wins), auth store, picker favorites and two-stage selection, doom-loop brake, plan mode | `epsilon-safety`, `epsilon-llm/src/auth.rs`, `epsilon-tui` picker, `epsilon-core` plan mode |

Also: [models.dev](https://models.dev) feeds the catalog via `scripts/regen_catalog.py`, the [improve](https://github.com/shadcn/improve) skill drives the audit loop, and [ponytail](https://github.com/DietrichGebert/ponytail) keeps changes minimal.

*Research-through-implementation. The harnesses above belong to their authors, we cite lessons and do not copy code.*
