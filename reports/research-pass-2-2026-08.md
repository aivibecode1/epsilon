# Reference Research Pass 2 — Where opencode / DSH / Hermes Beat pi (2026-08)

Second deep pass over `references/` plus real community signal (Hacker News threads on
pi and opencode, r/PiCodingAgent, DSH GitHub discussions, independent comparisons).
Goal per project scope: find where the non-pi references genuinely outperform pi, and
decide what Epsilon imports. Companion to `reports/pi-alignment-2026-08.md`.

## 1. Community signal (external, gathered this pass)

**What pi users praise (confirms our bet):**
- "Simple, smart, keeps its thinking grounded, hallucinates less, extremely good at
  following instructions" (r/PiCodingAgent). The minimal-prompt thesis holds.
- HN consensus endorses pi's no-permission-system stance *for pi*: "permissions are
  security theater if your agent can execute code — do proper sandboxing."
  **Epsilon's position is stronger than either pole**: we ship BOTH real confinement
  (Job Objects/bwrap) AND intent-level gates, wired together.

**What pi users miss (the actionable list):**
1. **No plan mode** — recurring request; users improvise TODO.md workflows.
2. No built-in todos/background bash/MCP-without-extension (MCP: Epsilon has it
   built-in; todos: covered by skills philosophy, documented exclusion).
3. Auto-compact cache-friendliness is a live discussion topic even among harness
   authors — validates ADR-007/008 as a differentiator worth keeping prominent in docs.

**opencode vs the field:** praised for provider breadth and AGENTS.md portability;
criticized for Effect-beta churn under the product (matches HA-opencode W1) and
token-efficiency vs tuned-native stacks. Lesson unchanged: adopt its *interaction
patterns*, not its framework posture.

**DSH reality check:** third-party review (wavect.io, Aug 2026) confirms our audit:
compelling architecture, developer-preview maturity, and its own sandbox "is not a
security boundary" for self-modification. Epsilon's decision to skip self-modifying
plugin toolsets in v1 (architecture §14) is externally validated.

## 2. Import decisions from this pass

| Candidate | Source | Decision | Rationale |
|---|---|---|---|
| **Plan mode** | opencode `plan` agent | **SHIPPED this pass**: `--plan` flag + `plan_ruleset()` + startup banner | Highest-frequency community gap for pi; maps perfectly onto our existing ruleset engine (deny is monotonic, so plan mode is a real boundary); ~60 lines total |
| Session search | pi itself (`search/scanning.ts`) | **SHIPPED this pass** (see alignment report G1) | Was a pi-parity gap, closed alongside |
| Todos tool | pi exclusion / community DIY | Keep excluded (ADR-consistent): skills + session log cover the need; catalog budget rule (design/epsilon-skills.md) argues against always-on schema cost | Documented, revisit only with user demand |
| Background bash | pi "use tmux" stance | Defer — serve/rpc modes already enable remote orchestration; a background-bash tool needs an approval-story design first | Recorded as candidate future milestone |
| Cache-friendly auto-compact messaging | community discussion | No code change needed: ADR-007 already isolates summary calls; ensure docs keep claiming this loudly (they do) | None |

## 3. Verification of plan-mode import

```
cargo test -p epsilon-safety          # 22 passed (incl. 2 new plan-mode tests)
cargo build -p epsilon-cli            # clean
Live faux-provider run, --plan:       # write DENIED, banner shown, file absent
Same script without --plan:           # write allowed, file created (control)
```

The control run matters: plan mode must block *only* what it claims to block.
