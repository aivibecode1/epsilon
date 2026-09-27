# 12 — Fresh reference audit (2026-09-27)

Supersedes nothing on its own. It is a new, evidence-based pass over the four reference
harnesses, written after four read-only first-hand studies
([`evidence/reference-studies-2026-09-27/`](evidence/reference-studies-2026-09-27/README.md)),
in which no existing Epsilon analysis was treated as input.

**The rule this file follows, and why.** Every claim below that is about *Epsilon* was checked
against Epsilon's own source before it was written down, and the check is cited by file and
line. Claims about the *references* carry the path the study read. Where I could not verify
something, the row says so rather than guessing — a gap that is honestly labelled is worth more
than a confident one, and this repository has already been bitten by the alternative.

---

## 1. The finding that governs all the others

**All four references have documentation that contradicts their own code.** This is four for
four, and it is the strongest single result of the pass:

| reference | the doc says | the code says |
|---|---|---|
| Hermes | `agent/AGENTS.md:15` — `max_iterations` defaults to 500 | `run_agent.py:238` — `sys.maxsize`, i.e. unlimited |
| Hermes | `agent/AGENTS.md:11` — `__init__` takes ~60 parameters | AST count is 81 (`run_agent.py:233`) |
| DSH | CLI help advertises `dsh tui --resume` in three examples (`apps/cli/src/args.ts:86-89`) | there is no `tui` package, bundle, or directory anywhere in the tree |
| opencode | `specs/storage/remove-opencode-db.md:21-44` lists three importers of a deleted module | those files now import from `@opencode-ai/core/database/database` |

**Consequence for how this repository works:** a reference's README, AGENTS.md, or skill file
is documentation of an intent, never evidence of a behaviour. The same rule this project
already applies to its own `audit/` folder now has to be applied outward, to
`references/*/AGENTS.md` and to `.agents/skills/`. This is the concrete form of "treat none of
it as gospel".

### 1a. And a live instance inside this repo

`.agents/skills/ratatui-tui/SKILL.md` says its `assets/theme.rs` encodes **four** decisions a
naive token struct gets wrong. The file's own header comment says **three**. Small, but it is
the same class of error, authored in this repository, and it is the strongest available argument
for checking skill claims against code.

---

## 2. Where Epsilon is already ahead of every reference

Stated plainly because the goal is the best harness, and an audit that only lists shortfalls
misrepresents the work:

- **Per-tool permission gates.** pi has **no per-tool approval at all** — the study searched
  for `confirm(`/`askUser`/`requireConfirmation`/`permission`/`approval` across
  `packages/coding-agent/src` and found only extension dialogs and one import prompt. Its safety
  rests on `executionMode: "sequential"`, not on a permission system. Epsilon has real gates
  plus doom-loop and no-progress guards (`crates/epsilon-core/src/exec.rs:371-530`).
- **OS-level confinement.** The opencode study searched the whole tree for
  `landlock|seccomp|LD_PRELOAD|bubblewrap|bwrap|nsjail|AppArmor|pledge_unveil|seatbelt|chroot|
  unshare|clone3|sandbox` and found exactly one production hit — a *deletion*
  (`packages/desktop/src/main/server.ts:218`). Its defaults are `"*": "allow"` including
  unrestricted `bash` (`packages/opencode/src/agent/agent.ts:119-136`). Epsilon has Windows Job
  Objects and bubblewrap.
- **Output spill.** Already implemented and already DSH-derived
  (`crates/epsilon-tools/src/lib.rs:7` — "caps and spills oversized outputs (DSH locator
  pattern)"; `spill_dir` at `crates/epsilon-core/src/exec.rs:691`).
- **Truncated-argument safety.** `fail_all_truncated` runs on `StopReason::MaxTokens`
  (`crates/epsilon-core/src/lib.rs:1598-1603`), matching pi's rule that streamed arguments may
  be silently incomplete.
- **Session history is a real tree**, like pi's `EntryBase { parentId }` — and unlike
  opencode, whose fork re-inserts every message and part with fresh IDs
  (`packages/opencode/src/session/session.ts:691-732`), duplicating every row.

---

## 3. Gaps, ranked by consequence

### G1 — The interrupted-tool-call repair is implemented and untested

`crates/epsilon-core/src/lib.rs:1180-1188` synthesises, for every call whose result never
arrived:

> `[harness] this call was never executed: the previous run ended before its result arrived.`

This is the right design, and it is also the only thing in the tree that *proves* a tool call is
durable before execution — you cannot synthesise that repair unless the call reached the store
first. That makes it a load-bearing claim about transcript integrity.

> **CORRECTED 2026-09-27 — the first draft of this section was half wrong.** It said "no test
> names it", citing a search that had missed M70. `crates/epsilon-core/tests/sequence_repair.rs`
> already had three tests. They are not nothing, but they build their `Agent` with **no
> `SessionStore` attached** and seed history through `restore_history()` — so they prove the
> repair works on the *runtime-history* view and say nothing about the *store*.
>
> The accurate statement is: **the runtime view was tested; the durable view was not.** Since the
> whole point is that the file already held the call before the tool ran, an assertion about
> process memory does not establish it. Closed by
> `the_repair_is_durable_and_leaves_the_answered_call_untouched`, which writes the interrupted
> turn through a real store, resumes from that store's own `model_view()`, and requires the
> repair to land in both views as the *same* `ToolResultMessage` — plus asserts the answered call
> is left alone, since an over-eager repair is as much a bug as an absent one.

*Severity: highest in this audit while open.* A durability invariant with no RED is one refactor
away from being quietly untrue.

### G2 — Extension capability model

Hermes mints exactly seven capability ids, each mapped 1:1 to an **already-existing** enforcement
gate, with the rule that "ids without an enforcing gate are deliberately never minted"
(`hermes_cli/plugin_capabilities.py:1-4,27-50`). Declaration is consent metadata, not a grant;
`plugin_capability_granted()` fails closed on any error; and `record_consent()` stores a hash of
the declared set the user actually saw, so a plugin that *adds* a capability on update is
re-prompted (`plugin_capabilities.py:155-186`).

Epsilon has a `SkillTool` and an extension surface, and no consent model. Not a parity gap — a
security-design gap the reference solved and this project has not considered.

### G3 — Slash commands: the pattern, not the count

Hermes has 99 and Epsilon has about 8. The count is not the finding. The finding is that
Hermes derives **CLI dispatch, the gateway, Telegram's BotCommand menu, Slack's subcommand map,
and autocomplete from one `CommandDef` list** (`hermes_cli/commands.py:23-47`), so they cannot
drift, and each command declares what happens when the agent is mid-run (`busy_policy`:
`reject` / `dispatch` / `interrupt_then_dispatch`). Epsilon's `picker.rs` is 1,458 lines and
holds its own list.

### G4 — Theme selection

> **CORRECTED 2026-09-27. The first draft of this section was wrong, on a narrow grep.**
> It claimed Epsilon had "no theme selector at all", from zero hits for `theme_pick`,
> `ThemePicker` and `set_theme`. The method name is `set_palette`, and the picture is much
> better than the claim: `ThemeMode::{Dark, Light}` exists (`ui/theme.rs:110`), both palettes
> are contrast-tested in both directions, and selection works through `EPSILON_THEME`,
> `[tui] theme` and the CLI's own resolution order (`crates/epsilon-cli/src/main.rs:187`).
> `app.palette` (`ctrl+p`) is the **command palette**, not a theme picker — a name collision
> that made the grep look emptier than the code was.

pi ships light and dark behind a picker (`components/settings-selector.ts`, 18 references) and
tests both (`test/theme-detection.test.ts`, `test/theme-picker.test.ts`). The one real gap was
narrower: **the palette was decided before the process started**, and `ThemeMode::flipped()`
carried the doc comment "for the runtime toggle" with no production caller — `set_palette` had
exactly one, in `app/event.rs:51`, applying the CLI's choice at startup.

Closed in M160 with `/theme`, which required first fixing a latent cache bug: the palette was a
rendering *input* to `reconcile` but not part of the invalidation *decision*, so a naive runtime
switch would have restyled the chrome and left every cached transcript row in the old palette.

> **The method lesson, now that it has happened three times.** G1 ("no test names it"), this G4,
> and the skill asset that "did not exist" were all **asserted absences from a single narrow
> search**. A negative claim has exactly one piece of supporting evidence — the search — and
> no second opinion. From here: a negative claim needs two independent searches, or it gets
> phrased as what was actually checked ("no caller found for `set_palette` outside
> `app/event.rs`") rather than as a claim about the world.

### G5 — `app.rs` is 4,497 lines with 75 methods

Concentrated in four separable concerns: picker/favourites (~500–1500), command dispatch
(~2000–2500), chrome and panels (~2500–3500), viewport measurement (~3500–4000). pi splits
exactly these into a component layer, and the reference study of pi's TUI confirms the
inversion that matters — **`agent` does not depend on `tui`; `coding-agent` depends on both**.

### G6 — In-transcript search, partially delivered

M157 delivered search that moves the view. Still absent: incremental re-filter, in-row
highlighting, scrollbar. Recorded so the slice is not mistaken for finished.

---

## 4. A trap present in three of four references

opencode ships **two complete tool implementations** (~1,700 lines each) with only one live on
the CLI path; pi ships a v3 session manager and a v4 tree repo where only v3 runs, with v4
reachable solely from `experimental/`; Hermes carries 332 files of `PLUGIN-COMPAT` shims dated
for removal on the snapshot's own date.

The lesson is not "these projects are bad". It is that a harness under pressure accumulates a
parallel future architecture, and it is very hard to tell from inside which one is shipping. For
Epsilon the operative rule is: **do not begin a v2 of anything while v1 is still moving.**

---

## 5. What I could not verify

- Whether any of the four references would actually run on this machine. All four findings are
  read-only source readings; no reference was built or executed.
- The DSH and Hermes study reports were returned inline and were never written to disk by the
  delegation layer. Their complete traces are archived, but the reports themselves are not
  verbatim in the repository. This is recorded in the evidence README rather than papered over.
- Claims about the references rest on the subagent's reading. Claims about Epsilon were
  re-checked here. That asymmetry is deliberate and should be preserved in any follow-up.

---

## 6. Recommended next work, in order

1. ~~**G1** — test the interrupted-tool-call repair.~~ **Done**, and the gap turned out to be
   half of what this document first claimed: the runtime view was already covered by M70, the
   durable view was not.
2. ~~**G4** — a theme selector.~~ **Done in M160** as `/theme`, and the gap was narrower than
   first claimed: the palettes and their contrast tests already existed; only the runtime switch
   was missing, behind a latent render-cache bug.
3. **G5** — split `app.rs` along the four seams the measurement found, not along intuition.
4. **G3** — one command table feeding every surface.
5. **G2** — the capability model, which is a design task rather than a feature.
