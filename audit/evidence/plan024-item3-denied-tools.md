# Plan 024 item 3 — "Hide denied tools from the model's tool list" re-verified

**Investigator:** read-only pass, 2026-10-04. No file outside this one was modified; no `cargo`
command of any form was run (another process owns the build). Every line below was read out of
the tree today; where a document disagreed with the tree, the tree wins and the drift is named.

**Scope.** `docs/plans/024-finishing-the-objective.md` §3 (lines 92–114) and the same claim in
`docs/STATUS.md:752-765`, which is the longer source of the recipe. Both were written 2026-10-04
against a tree that has since moved.

---

## 1. Claim table

| # | Claim | Verdict | Settled by |
|---|---|---|---|
| 1 | `PermissionEngine::evaluate` exists and is `pub` | **VERIFIED** | `crates/epsilon-safety/src/engine.rs:89` |
| 1 | …at `engine.rs:83` | **STALE** (moved +6) | `crates/epsilon-safety/src/engine.rs:89` — `pub fn evaluate(&self, capability: &str, subject: &str) -> Action` |
| 1 | The engine exposes only `with_defaults` / `layered` / `evaluate`, and **no set-shaped accessor of denied tools** | **VERIFIED — the premise holds** | `engine.rs:73-105` is the whole inherent impl: `:75 with_defaults`, `:82 layered`, `:89 evaluate`. Nothing else. `engine.rs:140-155` holds the free fn `plan_ruleset`; `:157-239` is `#[cfg(test)]`. |
| 1 | A **defaulted** method belongs on the `PermissionGate` trait at `crates/epsilon-core/src/gate.rs:51` | **VERIFIED** (the trait and line are right; `gate.rs:51` is `pub trait PermissionGate: Send + Sync`) | `crates/epsilon-core/src/gate.rs:51-78` |
| 1 | …and **that default reuses `evaluate`** | **WRONG — architecturally impossible** | `crates/epsilon-safety/Cargo.toml:11` declares `epsilon-core = { path = "../epsilon-core" }`; `crates/epsilon-core/Cargo.toml:19-22` declares only `epsilon-contracts`, `epsilon-llm`, `epsilon-session`. Safety is **downstream** of core, so a default body in core cannot name `PermissionEngine`. The reuse has to happen in an **override** in `epsilon-safety`, not in the default. |
| 2 | `RulesetGate` owns the engine at `gate.rs:52` | **VERIFIED** | `crates/epsilon-safety/src/gate.rs:51-55` (`pub struct RulesetGate` at `:51`, `engine: PermissionEngine` at `:52`, `ui` at `:53`, `memory` at `:54`) |
| 2 | `child_gate()` exists, added in M300 | **VERIFIED, and it is correctly placed** | `crates/epsilon-safety/src/gate.rs:156-161`, inside the `#[async_trait] impl PermissionGate for RulesetGate` block that opens at `:79` — **not** in the inherent block, and the doc at `:151-155` records exactly why an earlier version that lived in the inherent block compiled, was documented, and was never called. It clones the engine (`PermissionEngine` is `Clone` for this reason, `engine.rs:65-69`) and shares `ui`, with a **fresh** `memory` via `Self::layered` (`:64-70`). |
| 3 | `session_allows` exists at `gate.rs:73` | **VERIFIED** | `crates/epsilon-safety/src/gate.rs:72-75` — doc says "Read-only view of promoted always-allow pairs (diagnostics/tests)" |
| 3 | It is an **allow** set, not a denied set | **VERIFIED** | it clones `self.memory` (`gate.rs:74`), which is only ever pushed to on `PermissionDecision::AlwaysSession` (`gate.rs:118-121`) |
| 3 | It has **zero production callers** | **VERIFIED — really unused** | Four matches tree-wide. `crates/epsilon-safety/src/gate.rs:225` and `:273` are inside `#[cfg(test)] mod tests` (opens `:164`). `crates/epsilon-safety/src/gate.rs:73` is the definition. `crates/epsilon-safety/tests/additive_authorization.rs:224` is an integration test. Nothing under `crates/*/src/` outside a test module calls it. |
| 4 | Both call sites are in `stream_one_turn` | **VERIFIED, but there are three, not two** | `fn stream_one_turn` is `crates/epsilon-core/src/lib.rs:3154`. The third site is `fixed_prefix_tokens` at `crates/epsilon-core/src/lib.rs:2735`, whose doc at `:2730-2733` claims it "cannot drift from what is actually sent". The recipe and STATUS both omit it. |
| 4 | Where the stable prompt tier is built | **VERIFIED** | `crates/epsilon-core/src/lib.rs:3209-3213` (`let stable = prompt::Stable { tools: &tool_lines, … }`), fed by `tool_lines` at `:3185-3193` |
| 4 | Where the request payload is built | **VERIFIED** | `crates/epsilon-core/src/lib.rs:3269-3274`; `tools: self.tools.descriptors()` at `:3273` |
| 4 | Where `stable_hash` is computed over the tool tier | **VERIFIED** | `crates/epsilon-core/src/lib.rs:3250` — `let stable_hash = prompt::stable_fingerprint(&stable);`. The tool tier is mixed in at `crates/epsilon-core/src/prompt.rs:109-113` (name, `\0`, snippet, per tool, `:97-119`). |
| 4 | `docs/STATUS.md:761-762`'s anchors (`lib.rs:3061-3069`, `:3126`, `:3149`) | **STALE** | actual: `3185-3193`, `3250`, `3273` |
| 5 | `ToolRegistry::subset` at `registry.rs:59` | **VERIFIED signature and line; WRONG crate** | `crates/epsilon-core/src/registry.rs:59` — `pub fn subset(&self, names: &[String]) -> ToolRegistry`. It is in **epsilon-core**, not `epsilon-tools`. `grep 'struct ToolRegistry'` across `crates/` returns exactly one hit: `crates/epsilon-core/src/registry.rs:12`. |
| 5 | One existing caller as the filtering precedent | **VERIFIED** | `crates/epsilon-core/src/lib.rs:3577-3580` — `match def.as_ref().and_then(\|d\| d.tools.as_ref()) { Some(names) => Arc::new(self.tools.subset(names)), None => Arc::clone(&self.tools) }`. Note the shape to copy: `subset` returns an owned `ToolRegistry`, and the "no filter" case clones the `Arc` rather than rebuilding. |
| 6 | **No existing mechanism tells the model in prose that a tool is unavailable** | **VERIFIED — nothing exists** | `grep 'denied_tools\|hidden_tools\|tool_notice\|unavailable_tool\|refused_tool\|available_tools\|is_denied'` over `crates/**/*.rs` → **no matches**. `crates/epsilon-core/src/prompt.rs` contains no permission, denial or availability text at all; its tool block (`:133-140`) only prints what it is handed. **The design is free to choose.** |
| 6′ | *(new, recipe misses it)* There **is** an existing in-band prose refusal, at call time | **NEW** | `crates/epsilon-core/src/exec.rs:469-475` — `"Permission denied by policy. Adjust approach or ask the user."`, delivered as a `ToolResultMessage` via `error_result` (`exec.rs:912-923`, `is_error: true`, `details: {"blocked": true}`). Siblings: `exec.rs:526-531` (no-progress guard) and `exec.rs:320-325` (the local/exec path, whose default is the shorter `"Permission denied by policy."`). This is the wording item 3 must not regress to. |
| 6″ | *(new, recipe misses it)* The bad string hiding produces | **NEW** | `crates/epsilon-core/src/exec.rs:388-392` — `format!("Tool '{name}' not found")`, same `error_result` shape. The plan's own warning at `:110` is accurate and is the single biggest design constraint below. |

**Verdict on the recipe: the premise is right, the anchor is right, the `evaluate`-reuse half is
wrong, and it is one call site short.**

---

## 2. Smallest correct implementation sketch

Four files, ~45 lines of product code. Every insertion point is a line number in today's tree;
they will move the moment anyone edits above them, so re-grep by symbol before applying.

### 2.1 `crates/epsilon-core/src/gate.rs` — the seam

Insert **inside `pub trait PermissionGate`, immediately after `child_gate` (ends `:77`,
trait closes `:78`)**, ~8 lines including the doc:

```rust
    /// Tool names this gate refuses outright, so the model is never offered them.
    ///
    /// Defaulted to "nothing refused": `AutoAllow` (`:81`), `AutoDeny` (`:91`) and
    /// every ad-hoc test gate keep their current behaviour. **A gate that wraps another
    /// gate must forward this**, or the filter silently switches off behind it —
    /// `AGENTS.md` §2.7 is exactly that shape.
    fn denied_tools(&self, names: &[String]) -> Vec<String> {
        let _ = names;
        Vec::new()
    }
```

The default cannot do the work (see row 1): `epsilon-core` does not depend on
`epsilon-safety`. That is the correction to the recipe.

### 2.2 `crates/epsilon-safety/src/gate.rs` — the policy

Insert **inside `#[async_trait] impl PermissionGate for RulesetGate` (opens `:79`), just before
`child_gate` at `:156`**, ~8 lines:

```rust
    fn denied_tools(&self, names: &[String]) -> Vec<String> {
        names
            .iter()
            .filter(|n| self.engine.evaluate(n, "*") == Action::Deny)
            .cloned()
            .collect()
    }
```

**The subject `"*"` is not an arbitrary probe.** It is the subject the gate itself uses for
every tool with no `path`/`file_path`/`dir` argument and no `command`:
`crates/epsilon-core/src/exec.rs:153` — `(name.to_string(), "*".to_string())`. Path tools and
`bash`/`shell` get a real subject (`exec.rs:139-152`).

Two properties make this safe rather than clever:

- **It can only over-approximate.** `evaluate` returns `Deny` early and unconditionally on the
  first matching deny rule (`engine.rs:96-99`), so anything this hides, `authorize` would have
  denied anyway — and `authorize` checks the engine **before** session memory
  (`gate.rs:81-90`), so a promotion can never resurrect it. **The filter is a lower bound on
  guaranteed refusal, never a new decision.**
- **It never fires on `Ask`.** Hiding an `Ask` tool would destroy the approval flow. Only
  `Deny` is filtered, and only `Deny` is filtered *outrightly*: a narrow deny such as
  `Rule::new("bash", "rm*", Action::Deny)` is **not** detected and still produces the existing
  policy refusal. Best-effort by construction, and the method name should say "refused
  outright", not "refused for every subject".

Edge case worth one line in the doc comment: `?` compiles to `.` (`wildcards.rs:32`), so a deny
rule whose pattern is literally `*`, `**` or `?` matches the probe subject `"*"`. A rule denying
one-character subjects would hide the tool outright. Harmless, but the doc must not claim a
universal quantifier it does not have.

### 2.3 `crates/epsilon-core/src/lib.rs` — one helper, three call sites

Add a private helper on `Agent`, **immediately before `fn fixed_prefix_tokens` at `:2734`**,
~8 lines:

```rust
    /// The tools this turn advertises: the registry minus anything the gate refuses
    /// outright. Execution still uses `self.tools` (see `crate::exec::execute_batch`),
    /// so a call the model already made is refused by policy, never reported missing.
    fn offered_tools(&self) -> Vec<epsilon_contracts::tool::ToolDescriptor> {
        let all = self.tools.descriptors();
        let names: Vec<String> = all.iter().map(|d| d.name.clone()).collect();
        let denied = self.gate.denied_tools(&names);
        if denied.is_empty() {
            return all;
        }
        all.into_iter()
            .filter(|d| !denied.contains(&d.name))
            .collect()
    }
```

`subset` is **not** the right helper here, and the difference is the point: `subset` returns a
*new registry*, which is what delegation wants (`lib.rs:3578`) because the child must not execute
what it cannot see. For hiding, the payload must be filtered while `self.tools` stays whole —
see risk 2. `descriptors()` is already name-sorted (`registry.rs:45-53`), so the filtered order
is deterministic and stable; that matters for risk 1.

Three one-line edits:

| line | from | to |
|---|---|---|
| `crates/epsilon-core/src/lib.rs:2735` | `let tools = self.tools.descriptors();` | `let tools = self.offered_tools();` |
| `crates/epsilon-core/src/lib.rs:3187` | `.descriptors()` (the `self.tools` chain above it) | `offered.descriptors()` — or, simpler, build `tool_lines` from `self.offered_tools()` at `:3185` |
| `crates/epsilon-core/src/lib.rs:3273` | `tools: self.tools.descriptors(),` | `tools: self.offered_tools(),` |

`:2735` is not optional. Its doc at `:2730-2733` states the count "cannot drift from what is
actually sent"; filter the payload without filtering here and that sentence becomes a lie, and
compaction pressure is computed from a tool set the provider never saw.

### 2.4 The prose notice — `crates/epsilon-core/src/prompt.rs`

Where it goes is the one design decision the recipe leaves open, and the answer follows from the
engine's mutability: **`RulesetGate.engine` is a plain field with no interior mutability**
(`gate.rs:52`), so the denied set is **constant for the agent's lifetime**. A constant belongs in
the **stable** tier, not the volatile one and not an `AgentEvent`.

- Add `pub denied: &'a [ToolLine],` to `pub struct Stable<'a>` — after `tools` at `:33`.
- In `build` (`:125`), after the tool loop closes at `:140` and before the `# Guidelines` block
  at `:141`, print a block only when non-empty (~6 lines):

  ```
  # Tools unavailable by policy

  The following tools are registered in this session but refused by the current
  permission policy. Do not call them; the call will be refused, and asking again
  will not help. Continue with the tools listed above.
  - bash: refused by policy for every input
  ```

- In `stable_fingerprint` (`:97-119`), mix `denied` in after the tool loop at `:113` (~3 lines).

**That last edit is not optional and is the easiest thing in this plan to get wrong.**
`prompt.rs:94-96` states "A change in this number is a genuine cache event", and `:107-113`
hashes exactly the bytes of the tier. Add prose to the tier without hashing it and two different
stable tiers hash identically — the fingerprint starts lying about the only thing it is for.

### 2.5 What to reject

The recipe is leaner than most, but these are the pieces that would turn it into work:

- **Do not** add a `ToolUnavailable` `AgentEvent`, a notification bus, or a new prompt tier.
  The model already receives refusals as tool results (`exec.rs:469-475`); the prose block is a
  *list-level* statement, and the existing machinery covers the *call-level* one.
- **Do not** thread the denied set into `ToolRegistry` itself. It would make the registry lie
  about what it holds, and it would break `exec.rs:364`.
- **Do not** build the predicate as "does any deny rule exist for this name" by walking layers —
  that means adding an accessor to the engine, which is precisely the thing the premise says
  does not exist and `evaluate` already answers.
- **Do not** gate on `Ask`. It would hide every tool that needs approval, which is most of them.

---

## 3. Risks

**1. What breaks when the stable tier's fingerprint changes.** Nothing mechanically, and one
thing diagnostically. `stable_hash` (`lib.rs:3250`) is handed to `cache.observe` at
`lib.rs:3348-3353`, and a change in it is reported as `MissCause::PrefixChanged`
(`crates/epsilon-core/src/cache_stats.rs:40`) with the re-billed tokens priced
(`cache_stats.rs:72-82`). Today that cause can only mean "the tool set, the identity block, or
the appended system text moved" (`prompt.rs:94-96`). After this change it can also mean "a deny
rule exists", which is not a cache regression — it is the feature working. Record that in the
ADR rather than in code. **The dangerous variant is a per-turn flap:** if the notice text were
built from anything that varies per request (a date, a call count, the current subject), the
prefix would differ every turn and every request would be reported `PrefixChanged` — a
self-inflicted cache-miss storm with a correct-looking label. Hence: derive the notice from
`denied_tools` and nothing else, and keep `descriptors()`'s existing sort (`registry.rs:51`) so
the bytes are a function of the rule set alone. In practice the hash moves **once**, when the
agent is built against a ruleset that denies something (notably `--plan`, whose ruleset is built
at `crates/epsilon-runtime/src/lib.rs:4763-4768`).

**2. Can hiding break a tool call the model already made?** **Yes — and the sketch is built to
avoid it.** `execute_batch` resolves the tool at `crates/epsilon-core/src/exec.rs:375` from
`registry = &agent.tools` (`exec.rs:364`). If the filter ever reached `agent.tools`, a call that
reached `None` would be answered `Tool '{name}' not found` (`exec.rs:388-392`) — an `is_error`
tool result telling the model the tool **does not exist**, instead of
`Permission denied by policy. Adjust approach or ask the user.` (`exec.rs:474`). That is the
regression the plan itself calls "strictly worse" at `plans/024-…md:110`, and it would fire on
every resumed transcript whose history predates the change. Because the sketch filters only the
request-build path, a call the model made after seeing the filtered list still resolves normally
and is then refused by the real gate with the existing policy prose — which is the correct
outcome. **The rule to hold: `self.tools` is the execution registry and is never filtered.**

**3. A wrapper gate silently switches the feature off.** The method is defaulted, so any gate
that wraps another without forwarding it returns "nothing denied". `RecordingGate` at
`crates/epsilon-safety/tests/additive_authorization.rs:53-58` is exactly such a wrapper, and it
would keep passing while proving nothing about production. Same class as the `child_gate` defect
recorded at `gate.rs:151-155` and `docs/STATUS.md:745-751`. Forward it there in the same commit.

**4. The default ruleset denies nothing, so a naive test proves nothing.**
`default_ruleset()` (`engine.rs:111-133`) contains **no `Deny` rule at all** — reads, writes and
edits `Allow`; `bash` and `webfetch` `Ask`; secrets `Ask`. Production wires
`PermissionEngine::with_defaults()` when a project has no `[[rules]]`
(`crates/epsilon-runtime/src/lib.rs:4769-4770`). So with default settings **this feature hides
nothing and is invisible**. It bites in `--plan` (`plan_ruleset`, `engine.rs:143-155`, which
denies `write`/`edit`/`bash`/`shell`) and under a project deny rule. Any test written against
`with_defaults()` would pass on a tree where the feature does not exist — this is the single
easiest way to ship a green no-op here, and the reason the RED test below pins an exact number.

**5. `external_dir` is a second, per-subject capability.** A path tool can be advertised — its
own capability `Allow` — while every call outside the workspace is denied through `external_dir`
(`exec.rs:139-143`, `:442-443`). The filter is keyed on the tool-name capability only. That is
correct, but the prose must not say "this tool is gone"; it must say it is refused by policy.
`AGENTS.md` §2.7: nothing a UI advertises may be a no-op, and the reverse is also true — nothing
may be advertised as gone while it is actually there.

**6. Delegated children are already covered, and there is no leak.** `build_task_child` takes
`self.gate.child_gate()` (`lib.rs:3571-3574`), which returns a fresh `RulesetGate` over a clone
of the same engine (`gate.rs:156-161`). Same rules, so the same tools are hidden from the child.
No change needed; worth one line in the ADR because "subagents see fewer tools" is a
user-visible consequence.

---

## 4. RED test plan

**File:** `crates/epsilon-safety/tests/denied_tool_visibility.rs` (new, ~90 lines).

*Why a new file and not `additive_authorization.rs`:* that file's `run()` helper returns
`(RunOutcome, usize)` (`additive_authorization.rs:80`, `:119`) and hands the `FauxProvider`
straight to `Agent::new` at `:100`, discarding the very handle this test needs. Widening its
signature touches four existing tests (`:132`, `:158`, `:178`, `:201`, `:232`); a new file with
a small local fixture is the smaller diff.

*Why this crate:* `epsilon-core` does not depend on `epsilon-safety` (row 1), so only a test in
`epsilon-safety` can construct a real `RulesetGate`. `epsilon-llm` and `epsilon-tools` are
already dev-dependencies (`crates/epsilon-safety/Cargo.toml:22-23`) and both are unused by this
test, which is fine — Cargo does not require dev-deps to be used per-test-file.

**Fixture**

- Two inert tools in one `ToolRegistry` (reuse the `InertTool` shape from
  `additive_authorization.rs:17-34`): `"readfixture"` and `"bashfixture"`, both
  `ExecutionMode::Sequential`, both never touching a file or a process.
- Gate: `RulesetGate::layered(PermissionEngine::layered(vec![Ruleset { name: "deny",
  rules: vec![Rule::new("bashfixture", "*", Action::Deny)] }]), Arc::new(AutoApprove))`.
  **`AutoApprove`, not `AutoDeny`** — with an auto-approving UI the *only* reason
  `bashfixture` can be missing is the deny, so the assertion cannot pass for the wrong reason.
  Using `PermissionEngine::with_defaults()` here would prove nothing (risk 4).
- `let faux = Arc::new(FauxProvider::new(vec![FauxStep::say("done")]));` — keep the handle;
  `FauxProvider::recorded_requests()` returns `Vec<LlmContext>` at
  `crates/epsilon-llm/src/faux.rs:108-110` and the payload is built at
  `crates/epsilon-core/src/lib.rs:3269-3274`.
- `Agent::new(LoopLimits { verification_nudges: 0, ..Default::default() }, faux.clone(),
  Arc::new(registry), gate, workspace)` (`crates/epsilon-core/src/lib.rs:747-753`), then
  `agent.run(Message::User(…)).await` exactly as `additive_authorization.rs:108-117`.

**Test 1 — `a_tool_the_policy_refuses_is_absent_from_the_payload_and_named_in_prose`** (the RED)

```rust
let req = &faux.recorded_requests()[0];
let names: Vec<&str> = req.tools.iter().map(|d| d.name.as_str()).collect();
assert_eq!(names, ["readfixture"],
    "offered: {names:?} — a tool the ruleset refuses outright must not reach the payload");
assert!(!req.system_prompt.contains("- bashfixture:"),
    "the denied tool must not be listed in the stable tier");
assert!(req.system_prompt.contains("bashfixture"),
    "hiding alone answers \"Tool 'bash' not found\"; the model must be told it is refused");
```

**What the defect prints today**, measured off the code rather than reasoned about:

- `names` is **`["bashfixture", "readfixture"]`** — length **2**, expected **1**. The assertion
  message will read `offered: ["bashfixture", "readfixture"] — …`, and the first failure is
  `left: ["bashfixture", "readfixture"], right: ["readfixture"]`.
- `system_prompt` contains the string `- bashfixture: Inert authorization fixture`, produced by
  `crates/epsilon-core/src/prompt.rs:138` (`format!("- {}: {}\n", t.name, t.snippet)`) from
  `crates/epsilon-core/src/lib.rs:3185-3193` — so the second assertion fails with
  `the denied tool must not be listed in the stable tier`, which is a *different* failure from
  the first and names the second of the two call sites. Asserting both is what makes this test
  prove the payload **and** the tier were filtered, not one of them.

The negative control is the definition: revert only the filter and the test fails with the string
above; restore it and it passes. A control that passes is a control not yet proven, so run it.

**Test 2 — `a_tool_the_policy_refuses_only_for_some_inputs_is_still_offered`**, ~12 lines, same
fixture with `Rule::new("bashfixture", "rm*", Action::Deny)`. Both names must be in `names`.
This is the control for over-filtering: it fails if somebody widens the predicate to "any deny
rule mentions this tool", which would hide `bash` from every project with a narrow deny and break
`AGENTS.md` §8's promise that reads and searches keep working.

**Test 3 (characterisation, not RED) — `denied_tools_lists_only_outrightly_refused_names`** in
`crates/epsilon-safety/src/gate.rs`'s `mod tests` (opens `:164`): over
`PermissionEngine::layered(vec![plan_ruleset()])`, `denied_tools(&["read", "write", "edit",
"bash", "shell", "mystery"])` returns `["write", "edit", "bash", "shell"]` in input order, and
`mystery` is absent. Stated honestly: **this test cannot fail before the method exists — it will
not compile.** Its job is to pin the predicate's edge (`unknown capabilities Ask`,
`engine.rs:186-191`) so a later widening is caught. Say so in the test's doc comment rather than
counting it as evidence.

---

## 5. One-line corrections for the plan

- `plans/024-finishing-the-objective.md:100` — "`evaluate` is already `pub`
  (`crates/epsilon-safety/src/engine.rs:83`)" → **`:89`**.
- `plans/024-finishing-the-objective.md:100-103` — "a **defaulted** method belongs on the
  `PermissionGate` trait … and reuses `evaluate`" → the trait and `:51` are right; the default
  **cannot** reuse `evaluate`. `epsilon-safety` depends on `epsilon-core`
  (`crates/epsilon-safety/Cargo.toml:11`), not the reverse. Put the default (empty) in core and
  the `evaluate` call in the `epsilon-safety` override.
- `plans/024-finishing-the-objective.md:105-107` — "Two call sites" → **three**:
  `lib.rs:3185-3193`, `lib.rs:3273`, and `lib.rs:2735`.
- `docs/STATUS.md:754` — `engine.rs:69,76,83` → `engine.rs:69,75,82,89`; `:761-762` —
  `lib.rs:3061-3069`/`:3126`/`:3149` → `3185-3193`/`3250`/`3273`.
- Neither document names **where** the prose goes. It is the stable tier
  (`prompt.rs:31-49`, `:133-140`), it must be mixed into `stable_fingerprint` (`:109-113`), and
  `agent.tools` must stay unfiltered (`exec.rs:364`).