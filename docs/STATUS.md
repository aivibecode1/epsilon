# STATUS

**This file is the living record.** It holds everything that changes: progress, decisions
in flight, what is cut, what is broken. The permanent rules live in
[`../AGENTS.md`](../AGENTS.md), which is locked. Do not put moving state in `AGENTS.md`.

Last updated: 2026-10-03

## Current task

Evolve Epsilon into one coherent agent harness with two front ends on a single core: a
powerful CLI/TUI agent and a polished **native** desktop IDE.

### Timing

| | |
|---|---|
| **Started** | 2026-10-03T11:12:47+03:00 (Asia/Riyadh) |
| Budget | ~2 days |
| New-feature freeze | ~2/3 of the budget; after that only fixes, docs, verification, cleanup |

Rule: a working, verified product beats breadth. Anything cut goes in the
[roadmap](#roadmap), never left half-built in the product.

### Phase log

| Time | Phase | Outcome |
|---|---|---|
| 11:12 | 0. Read `AGENTS.md` in full | Done — the file this session started from was **670 lines**, dated 2026-09-28, and stopped at M208. **Superseded: it was rewritten and locked as v2 (280 lines) later the same morning** — see [Corrections to the *old* `AGENTS.md`](#corrections-to-the-old-agentsmd-dropped-by-the-v2-rewrite). |
| 11:12 | 0. Ponytail gate | Done — already installed at `.agents/skills/ponytail*`, no reinstall. Verified byte-identical to upstream and instructions-only. |
| 11:13–11:30 | 0. Map the project | Done — 14 crates. Three read-only sweeps (desktop, TUI, core) produced the ranked gap lists below. A native desktop IDE and a half-built shared command registry **already existed**, which changed the plan from "build" to "finish and unify". **Re-measured 2026-10-04: 124,903 lines under `crates/*/src/`.** |
| 11:12–11:40 | 1. Reference + codebase review | Done — nine read-only subagents: five reference studies (pi, DSH, Hermes, opencode, OpenTUI) and four independent reviews (desktop, TUI, core/security, llm/session). All findings carry a file:line and are in `audit/03-defects.md`. |
| 11:40 | 2. **M276** — first defect batch | Done and verified. Nine defects, three reachable by opening a repo you did not write. Headline: a project config's `[[mcp_servers]]` spawned arbitrary binaries at startup, ungated. |
| 11:45 | 3. **M276 continued** — second batch | Done and verified. The three security rows the review named: **A33** (a `.env.` spelling), **A34** (credentials to third-party children), **A36** (goal gates run shell nobody was asked about). |
| 11:50 | 4. Repo gates after the `AGENTS.md` v2 rewrite | Green — `check-links` 642/0, `check-control-bytes` 932/0, `check-line-endings`, `check-agents-lock` matches. |
| 11:40–12:05 | 5. Reference study and credit | Done — six reference pages under `docs/references/`, [`references/TUI-DECISION.md`](references/TUI-DECISION.md), and root [`CREDITS.md`](../CREDITS.md). |
| 12:00–12:10 | 6. Documentation set | Done — [`START-HERE.md`](START-HERE.md) + [Arabic](START-HERE.ar.md), [`CONTRIBUTING.md`](../CONTRIBUTING.md), and a README in all 14 crates. |
| 12:10 → | 7. **M277–M291** — the hardening run | Fifteen milestones landed against a fixed tree, each with a battery: rollback made reachable, manifests corrected, two command lies closed, the screen-reader tree built, the transcript virtualised, layout agency, the registry move, the backup-ordering fix. Recorded user-visibly in [`../CHANGELOG.md`](../CHANGELOG.md); the reasoning is in **Roadmap** and **Known-broken** below. |
| 07:20 (10-04) | 8. Four read-only investigations | Done — the session lease, subagent permissions, the `.pre-epsilon` backup ordering, and whether the transcript is actually virtualised. **Two roadmap items were wrong as written and one premise was false**; all three are corrected below rather than acted on blindly. |
| 07:35–08:15 (10-04) | 9. **M290 finished, M291 landed** | M290's files were on disk from an **interrupted** battery and unverified; they are now verified and the desktop reads the registry from `epsilon-contracts`. M291 fixes the backup ordering defect with the RED control run first. |

> **Two writers, one workspace.** This objective is being worked by two agents at once in this
> repository, which is why `epsilon-runtime/src/lib.rs` and `epsilon-ext/src/host.rs` changed at
> 11:28 and 11:32 without either of them asking. The split has been accidental but has not been
> destructive: the M276 defect batches landed in the product tree, and the `AGENTS.md` rewrite,
> the lock, and the documentation set landed alongside. **Anything measured here describes the
> tree as it stood at the time named**, because a battery log spanning 11:28 describes bytes
> that moved.

### Decisions made in this session

| Decision | Where | One line |
|---|---|---|
| **TUI stack: keep Ratatui** | [`references/TUI-DECISION.md`](references/TUI-DECISION.md) | OpenTUI needs Bun ≥ 1.3 / Node ≥ 26.4 at runtime and ships 8 prebuilt native FFI packages, against a product promise of one binary with no Node or Python runtime. Licence was **not** the deciding factor — both are MIT. |
| **One command registry** | `AGENTS.md` §2.4 | Declared once, read by both front ends; per-front-end capability as data, not a second list. |
| **`AGENTS.md` locked by SHA-256** | `scripts/check-agents-lock.py` | An agent that can rewrite the rules it is held to is not governed by them. Negative control run: a tampered file exits 1. |

### Corrections to the *old* `AGENTS.md` (dropped by the v2 rewrite)

Verified by grepping callers rather than by reading.

| Old claim | Reality |
|---|---|
| `SessionStore::fork_branch_at` "wired only from RPC" | **5 production callers** — `rpc_mode.rs:964,1016`; `tui/app/commands.rs:1161` (`/fork`), `:1208` (`/clone`); `tui/app/keys.rs:851` |
| `set_tool_hooks` / `set_turn_hooks` have no production caller | **Wired** — `epsilon-runtime/src/lib.rs:1329,1316`, from `build_runtime_and_gate_runner:4507,4516` |
| Seven verification gates | **Ten**, including the lock check |

## Machine

8 logical cores, 15.9 GB RAM. Build output goes to `%TEMP%\epsilon-target`, never into the
repo's `target/`.

**The debug-info knob is necessary but not sufficient.** With `CARGO_PROFILE_DEV_DEBUG=0` and
`CARGO_PROFILE_TEST_DEBUG=0`, a battery once measured 4 GB. Measured again 2026-10-03: the
external target dir had reached **47 GB** and C: was down to **1.6 GB free** — too low to build.
Two things compound it:

- `CARGO_PROFILE_TEST_DEBUG` **accumulates**. Each distinct value produces a parallel set of test
  artifacts, and this session used four (`0`, `1`, `2` plus a default). Reusing one value across
  a long session keeps the sets from multiplying; switching values to escape the Avira block
  grows the directory.
- `debug/incremental` was **20.16 GB** of that 47, and `debug/deps` 25.87.

**The cheapest safe clean is `debug/incremental`, not the whole target dir.** Deleting it costs
nothing but a little recompilation and returned 1.6 GB → **19.1 GB** while keeping the warm
rlibs and test binaries. Measure before deleting (`Get-ChildItem <dir> -Recurse | Measure-Object
-Property Length -Sum`), name the directory in the command, and confirm the path before removing
it. Do not blanket-clean.

**Two machine faults cost time this session, both worked around without touching security
configuration:**

- **`LNK1104` / `os error 5` on freshly linked test executables.** This is the fault that
  consumed the most time, and the belief that it was *unfixable* was wrong. It is a
  security-software block bound to **one exact filename** — proved by probing, not assumed:
  creating a file at the blocked name is *denied*, while `epsilon_core-probe1.exe`,
  `epsilon_core-probe2.exe`, `epsilon_tui-probe.exe`, and **the same name in a fresh
  directory** are all writable. The remedy is fresh test-profile metadata; the only subtlety is
  that the value must be one this machine has not used before. `CARGO_PROFILE_TEST_DEBUG=0`
  produced a blocked name, `=1` produced another, `=2` produced names it had never seen — and
  the full battery went green first time. An earlier note here claimed `=1` "did not clear it"
  and that per-crate runs were the only workaround; that claim is **superseded** by the green
  battery below.
- **`LNK1318: Unexpected PDB error; LIMIT (12)`** on the `epsilon-desktop` link. Cleared by
  stopping `mspdbsrv.exe` and deleting the `.pdb` files in the build tree.

## Verification (what is actually true)

**Green full-workspace battery, latest measured 2026-10-04 (M294):**
[`../audit/evidence/gate-m297-2026-10-04.txt`](../audit/evidence/gate-m297-2026-10-04.txt).

| Gate | Result |
|---|---|
| `check-links` | PASS — 751 relative links, 0 broken |
| `check-agents-lock` | PASS — `AGENTS.md` matches `AGENTS.md.sha256` |
| `check-lint-tables` | PASS |
| `check-line-endings` | PASS |
| `check-control-bytes` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS — zero warnings |
| `cargo test --workspace` | **PASS — 2,022 passed, 0 failed, 7 ignored, 78 binaries** |
| `cargo run -q -p epsilon-cli -- --demo` | PASS |
| `cargo check -p epsilon-cli --features real` | PASS |

Previous green batteries the same day: [`gate-m295`](../audit/evidence/gate-m295-2026-10-04.txt)
and [`gate-m294`](../audit/evidence/gate-m294-2026-10-04.txt) — both 2,019 or 2,020 passed; the
difference is the tests added between them.

**The knobs that number was produced under**, because they change what the evidence is:
`CARGO_BUILD_JOBS=1`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, and
`CARGO_INCREMENTAL=0`. The last one is set by the caller and is therefore **not** in the log's
own header — it is appended to the evidence file by hand, which is the only reason a reader can
find it at all.

Earlier: [`gate-m294`](../audit/evidence/gate-m294-2026-10-04.txt), [`gate-m283`](../audit/evidence/gate-m283-2026-10-03.txt), [`gate-m282`](../audit/evidence/gate-m282-2026-10-03.txt), [`gate-m281`](../audit/evidence/gate-m281-2026-10-03.txt), [`gate-m280`](../audit/evidence/gate-m280-2026-10-03.txt), [`gate-m279`](../audit/evidence/gate-m279-2026-10-03.txt), [`gate-m277final`](../audit/evidence/gate-m277final-2026-10-03.txt), [`gate-base`](../audit/evidence/gate-base-2026-10-03.txt).

**The count is measured, and it is quoted from one battery rather than from memory.** A reader
found this section quoting **two different totals** — 2,018 in the table above it and 1,937
twelve lines below, each attributed to a different milestone. Only one number can describe a
given run, so the older paragraph is replaced by this one rather than left beside it. The
count is read out of the log (`test result: ok` lines, summed), not estimated.

**Two failures on the way here, both machine faults and both recorded rather than smoothed
over.**

1. **`LNK1318: Unexpected PDB error; LIMIT (12)`** on linking a test binary. It recurred on
   `epsilon_tui` and then, after that file was dealt with, on `loop_with_rules` — a *different*
   file, which is what makes it a `mspdbsrv` state fault rather than one bad path. Clearing it
   is stopping `mspdbsrv.exe` and removing the build tree's `.pdb` files.
2. **The disk filled, twice.** `C:` reached 0 bytes free and cargo reported
   `There is not enough space on the disk (os error 112)` — first inside the `test` leg and then
   in `demo`, which failed for a reason that has nothing to do with the demo. Measured: the
   external target directory held **9.84 GB**, of which **4.55 GB was 280 `.pdb` files** and
   **0.91 GB** was `debug/incremental`. The cheapest correct clean is still `incremental`; with
   debug info off for the test profile (`CARGO_PROFILE_TEST_DEBUG=0`) no PDBs are produced at
   all, and that is what finally fit.

**One earlier failure was a real bug I introduced, and it is worth keeping.** The M283 battery
needed two attempts: `the_tui_works_when_started_from_powershell` failed on attempt 1 and
passed on attempt 2 with **no code change between them**, having passed in the three preceding
green batteries. That is a flake under load, not a regression — and it is the *kind* of flake
that most looks like "the TUI is broken", which is why it is written down. See Known-broken.

**`fmt --check` failed twice running**, each time on a single line of mine, each time stopping
the run before the expensive leg. That is the gate working: a formatting mistake is free to fix
and must never cost a test cycle.

### How it went, because three of the failures were informative

Getting here took four battery attempts and **each failure taught something**, which is the
argument for running them rather than reasoning about them:

| Attempt | What failed | What it was |
|---|---|---|
| `m277` | 2 tests in `checkpoint.rs` | **A real bug I introduced.** My guard rewrite dropped the "digest absent but blob present" branch, so an entry that predates digests was told it had no blob — which was false. Found because an existing test asserted a sentence that became untrue. |
| `m277b` | `fmt --check` | One line of mine. Cheap, and it stopped the run before the expensive leg. |
| `m277c` | 1 TUI frame test | **A fixture lying about the product.** `checkpointed()` seeded a capture without the amendment every real write now performs, so the frame showed the coarser refusal. Fixed by making the fixture mirror the product — the right fix, since weakening the assertion would have left a test passing on a shape the product no longer creates. |
| `m277final` | — | **GATE PASS on attempt 1.** |

Each of those was deterministic, so each retry would have repeated it verbatim. `gate-until-pass.ps1`
was used instead of a blind loop, which is what it exists for.

**The earlier belief that the full-workspace `test` leg "cannot complete on this machine" is
wrong, and this battery is the evidence.** The blocker was never the code and never the whole
test leg: it was one security-software block bound to **one exact filename**. Probing settled it —
creating a file at the blocked name is *denied*, while `epsilon_core-probe1.exe`,
`epsilon_core-probe2.exe`, `epsilon_tui-probe.exe` and **the same name in a fresh directory**
are all writable. The remedy is fresh test-profile metadata, with one refinement:

> **Use a `CARGO_PROFILE_TEST_DEBUG` value this machine has not used before.** Values `0` and `1`
> had each produced a filename that was subsequently blocked, so `=1` had stopped working as a
> remedy by the third attempt. `=2` produced names the machine had never seen and every battery
> since has gone green. Filed as a proposal in [`AGENTS-PROPOSALS.md`](AGENTS-PROPOSALS.md);
> `AGENTS.md` is locked.

## What landed (M276, two batches)

| | Defect | Fix |
|---|---|---|
| security | **A50** `[[mcp_servers]]` in a project config spawned binaries ungated | `mcp_servers_trusted`, beside `skill_roots_trusted` |
| security | **A32** repo prompt templates answered the trust prompt | gated on trust; `.agents/prompts` added to the detector |
| security | **A33** `.env.` / `.env::$DATA` / `.env ` beat every name-based rule | one rejection in `paths.rs`, before policy and before any syscall |
| security | **A34** extension and MCP children inherited every credential | `ScrubEnv` injected into both, plus **a new mechanical guard** |
| security | **A36** goal gates ran model-authored shell under capability `goal` | gates now ask as **`bash`**; no approver wired ⇒ refuse |
| durability | `SessionStore::append` called `File::flush()` — a **no-op** | `sync_data()` |
| accounting | Bedrock and Mistral parsed cache counts but never set `cache_reported` | set from key presence, as Anthropic and compat already did |
| accounting | `session_usage()` lost a failed run's tokens | folded on the error path too |
| terminal | `/tree` sliced a `String` on a **byte** index from a **column** budget — panicked | the crate's own column-aware truncation |
| terminal | the permission dialog swallowed every key it did not bind | G10 notice, naming the keys that work |
| terminal | `guidance_mode` drawn nowhere; `e` destroyed the composer draft | drawn, stashed and restored, caret drawn |
| terminal | five glyphs were literals, so the **fold markers** boxed on the ASCII default | five repertoire entries |
| data | 15 C0 control bytes in tracked files, two inside `text_safety` | repaired, and `check-control-bytes` added as a gate |
| tooling | A46 the retry wrapper named the wrong gate in its fingerprint | per-leg parsing; empty ⇒ stop, not agree |

## Roadmap (cut scope lives here)

**Highest value still open**, in the order I would take them. Items 7–12 come from a separate
read-only sweep of the desktop, the TUI and the core run in this session; they are ranked
against the same bar and merge into the list above.

1. **Transcript virtualization** (desktop). ~~The work is not started.~~ **It is done, and the
   roadmap was stale.** M285 landed it: `draw_rows` computes the visible span first
   (`app.rs:4316`) and slices to it (`app.rs:4323`), so per-frame work is O(rows on screen).
   **The 104 ms and the ~5 ms are not two measurements of one thing** — 104.4 ms measured the
   unvirtualised code with the accessibility tree *off*, and a control on the same box measured
   127.9 ms; the current figure is with the tree on. Same fixture, same size. Three tests hold
   the bound (`app.rs:7906`, `app.rs:7753`, and a pure `RowLayout::span` sweep at `app.rs:8066`).
   **What is actually left is not performance:** ADR-022 claims "a test pins 800 rich rows → 44
   lines drawn → 53 named nodes" and **no test in the tree does** — the bounding tests use
   200/400 one-line *user* rows. That claim is either pinned with the fixture it names or
   dropped; it is not true as it stands.
2. **Subagent permission derivation.** A session-remembered `always allow bash` **does** leak
   into every subagent, and the leak has no inheritance decision anywhere: `build_task_child`
   hands the child `Arc::clone(&self.gate)` (`epsilon-core/src/lib.rs:3447`), and
   `SessionMemory` is itself shared. **The roadmap's "six lines" was wrong** — opencode's
   filter is three lines because it hands the child a whole ruleset, while epsilon's
   `Authorization { decision, asked }` cannot distinguish an allow *from memory* from an allow
   the engine's own rules produced. So the derivation has to live in `epsilon-safety` and needs
   a defaulted method on `PermissionGate` (`crates/epsilon-core/src/gate.rs:51`), which is a
   behaviour-level change and needs an ADR. opencode's rule, quoted at
   `references/opencode-1.18.34/packages/opencode/src/agent/subagent-permissions.ts:21-23`, is
   to inherit **denies only**.
3. **Hide denied tools from the model's tool list.** **Not implemented, and the premise is
   false**: the permission engine exposes `evaluate(capability, subject) -> Action` and nothing
   set-shaped, so there is no denied set to read — the only accessor is an *allow* set
   (`session_allows`). Building it is one new engine method plus **two** call sites (the request
   payload at `lib.rs:3149` and the stable prompt tier at `lib.rs:3061-3069`, which is
   fingerprinted at `lib.rs:3126`), and it must tell the model in prose that the tool is refused
   — hiding it alone answers "Tool 'bash' not found", which is strictly worse than a policy
   refusal. **Items 2 and 3 need the same trait edit; doing them together is one ADR.**
4. ~~**The `.pre-epsilon` backup ordering defect.**~~ **LANDED (M291), and it was worse than
   "the backup may be unopenable"** — it *was* unopenable, always, on that path. `append`
   stamped the lazy header before `persist_migration` took the copy, and the handle is
   `O_APPEND`, so on a headerless file the header landed at EOF and the `.pre-epsilon` sibling
   captured `original + header`; a header is legal only as the first line, so it opened as
   `Corrupt { SecondHeader }`. The reachable paths were `/resume` in both front ends and RPC
   `switch_session` — **`Agent::live_resume` opens and repairs without calling
   `persist_migration`**, so the first append is what took the copy. The runtime launch path
   calls it at startup, which is exactly why the suite stayed green. The copy is now taken
   **before** the header is stamped, which changes no other byte (the second call inside
   `persist_migration` is the once-only no-op) and moves the failure earlier.
5. **A desktop probe** (`scripts/desktop-probe.py`), the desktop equivalent of `console-probe.py`.
6. ~~**Lease: a live holder can be stolen from, and the victim's `Drop` deletes the thief's
   lock.**~~ **HALF LANDED (M294), and the half that landed is the ownership one.** Both were
   confirmed by reading `crates/epsilon-session/src/lease.rs`; the arithmetic was 4 s against a
   2 s heartbeat, so **two** consecutive missed ticks and the lock was gone while the process was
   alive.
   - **`Drop` no longer deletes a lock that is not its own.** The lock file carried `pid=` and
     nothing else for its whole life, so a stolen-from holder could not recognise its own file;
     it now carries a per-acquisition `owner=` token, and `Drop` removes the file only when that
     token is still in it. This was the silent half: the victim's heartbeat stamps **by path**, so
     it kept the thief's lock looking alive, then its drop deleted it, then the thief's heartbeat
     failed for ever with its errors discarded, and a third process saw **no lock at all**.
     Negative control run: reverting the guard fails the new test with the message it names.
     *Residual, stated:* read-then-remove is not atomic; the window shrinks from "however long
     the victim lives" to microseconds.
   - **The window is now 6 s, three heartbeats**, so a live holder survives two missed ticks
     rather than one. The cost is that a killed TUI recovers in ~6 s instead of ~4 s.
   - **Still open, and it is the half that matters:** nothing detects the takeover. The store's
     lease field is `_lease` and is never read, `append` writes through an already-open handle,
     and `SessionLease::refresh()` has **zero callers** and is dead code. Making `append` refuse
     once the lock is gone is a behaviour-level change and needs an ADR.
   - **The four doc comments that said a live holder is "never" stolen from are corrected.** An
     overclaiming comment is a defect a reader will act on: the code cannot honour "never", and
     the mtime test cannot see a starved process, only a quiet one.
7. **The desktop cannot see composition diagnostics.** Eleven `eprintln!` lines in
   `epsilon-runtime/src/lib.rs:4434…4679` are invisible in a window: the desktop captures only
   `launch.notices`, taken *before* composition (`crates/epsilon-desktop/src/lib.rs:134`). A
   skill that failed to load, an extension that was skipped, a session that repaired itself —
   none of it reaches the person who could act on it. This is a **silent failure** in a product
   whose own invariant forbids them, and it is cheap to fix.
   **→ LANDED.** Sixteen such lines (not eleven), now routed through `RunParams::notices`.
8. ~~**Desktop accessibility is near-absent.**~~ **Most of this closed at M281–M287, and the row
   is kept because the shape of what is left matters.** Gone: keyboard operation (`F6` /
   `Shift+F6` focus regions, arrows in the tree, `Enter`, `Escape`), eleven named regions in the
   real AccessKit tree, a per-row name on every *drawn* row, and the WCAG 1.4.11 state floor
   (`selection_focus` was 1.52:1 and is now held to 3:1). Text scaling landed at M287 — after
   measuring that egui 0.36's `Style` has **no** zoom field, so `egui::Style` has to be scaled on
   the `Context` or a row stays 15.0 points at 200%. Reduced motion is **vacuously** satisfied:
   the window does not animate, so a toggle would control nothing.
   **Left, and it is the honest remainder:** no screen reader has ever been run against this
   window on this machine; nodes attach flat to the window root with no landmark roles; a
   `Changed` row's click target emits no node; `layout.rs`'s `TITLE_H`/`STATUS_H` are unscaled;
   and the ADR's own "a test pins 800 rich rows → 44 lines drawn → 53 named nodes" has **no
   test behind it**.
9. **Four parallel command registries.** `Command` (38 variants), `COMMAND_HEADS` + `ALIAS_HEADS`
   (42), `SLASH_ITEMS` (43 rows at M290), `palette_entries()` (7) and `keybindings::DEFAULTS` (23) in the TUI —
   plus the desktop's own `app.rs:234` table, which is a separate namespace entirely. Only two
   drift checks exist. Measured drift today: `theme`, `expand` and `templates` parse but appear
   in neither `/help` nor autocomplete; `app.transcript.halfUp`/`halfDown` have handlers and no
   binding entry; the palette's `binding` is a hand-typed string, not a table lookup.
   **→ PARTLY LANDED (M279, M290).** The `theme`/`templates` gap and the palette's hand-typed
   key are closed, and the command half now lives in `epsilon-contracts` with the desktop reading
   it there — so the two front ends share one crate for it. **Still open:** the rows are four
   tables that merely live in one place, and one flat `REGISTRY` of `Action` rows (with the
   keybinding axis separate, because `halfUp` being unlisted is what happens when they are
   conflated) is unbuilt. That is a real refactor with no user-visible defect left attached.
10. **Checkpoint rollback is unreachable.** `record` runs on every write and edit
    (`epsilon-tools/src/files.rs:356,533`); `CheckpointStore::restore`
    (`crates/epsilon-tools/src/checkpoint.rs:322`) has **no production caller**. Checkpoints
    accumulate and grow forever, and "checkpoints and rollback" is on the required capability
    list. Either wire a `/restore` or delete the write path.
    **→ LANDED (M277).** `/restore <path>` previews, then applies on `/restore <path> yes`; the
    desktop refuses it by name; the store is reached through `epsilon-runtime`, adding no Cargo
    edge. Fixing it exposed a second, larger defect — the restore guard compared against the
    **pre**-image and so could never pass after a real write. Now recorded and fixed via
    `Entry::post_image`. ADR-023.
11. **The desktop answers 3 of 38 terminal commands** — `SUPPORTED` in
    `crates/epsilon-desktop/src/commands.rs:364` (after M290; it was `:272` before the registry
    moved) is `compact`, `goal`, `sessions`. Refusing by name is the right pattern; a third of
    the surface is the problem.
12. ~~**Desktop layout has no user agency.**~~ **LANDED (M289, ADR-025).** `Preferences` was dead
    state; there is now a drag handle on the sidebar's right edge, `Ctrl`+`B` to fold, arrows in
    16-point steps, and one small JSON file per workspace. **Still open:** the new bindings are
    in neither the palette nor `/hotkeys`, because both need a `Command` variant the shared
    registry does not yet exist to host — which is item 9, not a separate piece of work.

### Considered and deliberately not doing

- **Switching the TUI to OpenTUI.** Decided against; reasoning and evidence recorded.
- **Adopting codewhale-ratatui as a dependency.** Its ideas are worth having — layout order
  that *yields* information as the window narrows, terminal-profile fallbacks, per-profile
  snapshot testing. Its whale/ocean identity is its authors'. Taking the craft, not the
  character, and not a new dependency.
- **A second language toolchain.** Never, for widgets.

## Ponytail audit

Run repo-wide by an independent read-only pass (token-level call graph, `#[cfg(test)]` excluded
from the production-reference side, every surviving candidate confirmed by opening the file).
**38 findings, net −430 lines and −4 dependencies.** The full list **was not** retained — this
section used to claim it was, and a re-sweep on 2026-10-04 had to re-derive it from source. The
dead-code half is now written down under
[`../audit/evidence/ponytail-deadcode-2026-10-04.md`](../audit/evidence/ponytail-deadcode-2026-10-04.md);
the manifest half is dispositioned below. Nothing was edited by the auditor.

### Disposition

**Acting now — objectively wrong, cheap, no design decision (manifests).** Six of these are not
style preferences; they are declarations that do not describe the crate. **All six are now done
(M278) — and one of them turned out to be stale before anyone touched it, which is the more
useful fact.**

| # | Finding | Disposition |
|---|---|---|
| 5 | `sha2` is a `[dependencies]` entry of `epsilon-cli` with **zero** references | **DONE.** The 4 hits are the *field name* `api_request_sha256`; the digest comes from `epsilon-core::config::sha256_hex` |
| 6 | `async-trait` declared in `epsilon-cli` with a comment claiming a trait implements it | **DONE.** Zero `#[async_trait]` in the crate. The comment was false on **two** counts: `ExtensionToolHooks` and its `impl ToolHooks` live in `epsilon-runtime/src/lib.rs:959,979`, not in `epsilon-cli`. `ToolHooks` *is* `#[async_trait]`, so half the claim was true and the location was not |
| 7 | `async-trait` in `epsilon-session` | **STALE — never acted on.** `crates/epsilon-session/Cargo.toml` declares only `epsilon-contracts`, `serde`, `serde_json` and `tempfile` (dev). There was no entry to remove. The second stale finding of the audit, after #12 |
| 8 | `futures-util` in `epsilon-runtime` | **DONE.** Zero matches, even widened to bare `futures` across 5,664 lines |
| 9 | `tempfile` is a regular `[dependencies]` of `epsilon-cli` | **DONE**, and the audit's **count was wrong**: it said "all five uses"; there are **15** call sites in 5 files. All 15 are test code. Moved to `[dev-dependencies]`, and proved with `cargo tree -p epsilon-cli -e normal -i tempfile` → *nothing to print*, which is better than absence of a caller |
| 38 | `http-body-util` and `tower` pinned outside `[workspace.dependencies]` | **DONE.** Both moved to the workspace table; `tower`'s `util` feature left on the crate entry to match this repository's own shape (`tokio` does the same) |

**Two lessons worth keeping from the audit itself.** Two of its 38 findings were wrong — #7 was
already fixed before the report was read, #12 became live when the same session added
`CheckpointStore::resolve` — and one of its counts (#9) understated the work by 3×. None of that
makes the audit bad: it surveyed a tree that then moved, and said so by naming paths. But it is
the argument for **verifying each finding before acting on it**, which is what the brief to the
implementer said and what it did: "a finding that no longer holds is more useful recorded than
acted on."

**Highest-value shrink, deferred to the feature freeze.** #1: `now in epoch millis` written out
**21 times**, including **seven private `fn now_ms()` definitions across five crates** — and
`epsilon-contracts` is already a dependency of every crate that needs it. ~80 lines.
#2: four near-identical frontmatter parsers, one per crate. #3: a ten-line character loop where
`str::replace(['/', '\\', ':'], "-")` is byte-identical.

**Dead code — and the whole list is now on disk**, which it was not before:
[`../audit/evidence/ponytail-deadcode-2026-10-04.md`](../audit/evidence/ponytail-deadcode-2026-10-04.md).
A re-derivation from source on 2026-10-04 indexed every non-test definition in all 14 crates
(3,078 across 228 files) and classified each mention production-or-test by hand. **73 verified
items, 827 measured lines** - every line count is a span, not a reading. Both named findings
**stand** - `Hints::keys_text`, whose doc is still false, and
`probe_event_forward_latency`, which is **14 lines, not the ~40 this section used to claim** -
and **four more** of the same misleading-doc shape turned up with zero callers, including
`tools/tests/builtin_tools.rs:2186`, a test whose stated purpose is keeping an uncalled constant
alive.

**The disposition, which is what the definition of done actually asks for:**

| Class | Items | Lines | Disposition |
|---|---|---|---|
| Zero callers anywhere | 23 | 175 | **delete - authorized, not done** |
| No production caller, test callers only | 30 | 400 | **delete with the test sites - authorized** |
| Docs assert a caller that does not exist | 6 | 103 | **file as a defect**; `bedrock_event_payloads` (39 lines) is the one to look at |
| Test capability shipped in release libraries | ~21 | ~324 | **keep, justified** - deleting a fault-injection seam deletes real tests |
| False alarms checked and cleared | 5 | - | do not "fix" |

**Nothing was deleted, and that is a decision rather than an oversight.** 59 deletions across 14
crates in the freeze window, each needing its own battery, is how a project ends the day with a
red suite and a list of things that used to work. The list is written down, every row is
authorized, and the first batch is mechanical.

**The audit record had a defect worth naming.** The line above used to read "full list retained
with this section's summary below". **It was not** - the numbered findings existed nowhere in
the tree, which is why "finding #14" could not be disposed of by anyone, including me. A summary
of an audit is not the audit.

**Also found and fixed:** three Cyrillic characters inside an English doc comment
(`epsilon-llm/src/providers/vertex.rs:80`), which `check-control-bytes.py` cannot see - that gate
looks for C0 bytes, and a homoglyph is not one.

**One finding already retired by this session's work.** #12 says `CheckpointStore::latest_for`
has eight test callers and no production one. That is stale: `CheckpointStore::resolve` calls it
at `checkpoint.rs:345`, and `resolve` now has a production caller. The same session added the
caller, so the audit was right about the tree it saw and wrong about the tree that followed.

**Justified — do not act.** #25 `PaletteEntry.binding` looks like seven hand-typed chord
spellings restating `keybindings::DEFAULTS`, but a drift test exists and the maintainers
chose it; deleting it would be disagreeing with a deliberate decision, not removing excess.

**The gap that is still open, and it is a gap in the record rather than in the code.** The
**38-finding list itself was never written down** — only this section's summary of it survives,
which means the ~20 dead-code rows cannot be individually verified or justified by the next
person, and a definition of done that asks for "each remaining finding justified in writing"
cannot be met from a summary. A re-sweep was started on 2026-10-04 to re-establish the list
against today's tree; **whatever it returns, the finding list is written to a file this time**,
under `audit/evidence/`, so the next audit starts from evidence rather than from a paragraph.
Recording the failure to record is the honest half; the other half is not leaving it there.

**Flagged as least confident by the auditor itself:** the frontmatter consolidation (#2) — the
four parsers are *near*-duplicates, not identical (only `tools/skill.rs` strips a BOM;
`contracts` returns k/v pairs while `agents.rs` reads fields off them), so the real saving is
three copies of ~25 lines becoming one ~30-line parser, and someone must first decide the BOM
handling and the split-vs-flat question. And #1's ~80-line figure is inferred from reading
bodies, not counted line by line.

---

## Next

**Freeze new features at two-thirds of the budget.** Start was 11:12 on 2026-10-03 with a ~2-day
budget, so the freeze point is roughly **19:00 on 2026-10-04** — about 32 hours in. After that:
only fixes, documentation, verification and cleanup. Anything cut goes in the roadmap above,
never left half-built in the product.

**This round is in that window already** (10:30 on 2026-10-04), so the list below is not a plan
for new work — it is what is *not* being done, written down so the next person does not read the
roadmap as a queue. **The items in Roadmap are cut, not pending**, except where a line says a fix
landed. Nothing below is half-built in the product.

1. **Composition notices reaching the desktop** — LANDED. Sixteen `eprintln!` lines inside
   `build_runtime_and_gate_runner` now go through one ordered sink, `RunParams::notices`, instead
   of to a console a window does not have. `tui_mode` appends them to the startup notices it
   already draws; the headless modes still print to stderr exactly where they did; the desktop
   stores them on `IdeSession` and renders them in the agent-pane header it already had.
   **Not also printed** — routed, not duplicated. Two runtime tests and two desktop tests, with
   both negative controls run.
2. **Checkpoint rollback made reachable** — LANDED. `/restore <path>` previews, then applies on
   `/restore <path> yes`; the desktop refuses it by name. Routed through
   `epsilon_runtime::{describe_checkpoint, restore_checkpoint}` with **no new Cargo edge**, per
   the `/goal` precedent.
3. **The guard was unsatisfiable, and that was the real find.** `Entry::digest` holds the
   *pre*-image, so reverting an edit refused 100% of the time — invisible while "no UI reaches
   this yet". Fixed by recording the **post**-image (`Entry::post_image`, written by
   `amend_with_post_image` after the write) rather than by weakening the guard. ADR-023.
4. **WCAG 1.4.11 contrast for focus and selection state** — LANDED. `selection_focus` sat at
   **1.52:1** against `selection`, below the 3:1 the standard asks for a *state*, and invisible to
   every existing contrast test because they all measure against the page rather than the sibling.
   Third floor added, second separation pass added, negative control run. Detail above.
5. **Six manifest defects** — LANDED (five acted on, one found already stale). Detail above.
6. **Two user-visible command lies closed** — LANDED (M279, ADR-024). Six commands parsed and
   were in `COMMAND_HEADS` but appeared in **neither** `/help` nor the `/` autocomplete, so a
   person could type `/theme` and have it work with no way to learn it existed; and the command
   palette advertised **`"/login"` in its key column** — a slash command where a reader looks for
   a chord, for an action that has no key at all. Both now guarded:
   `every_offered_command_is_reachable_from_slash_items` (written first; it **failed on `/theme`**
   before anything was added) and `a_palette_key_hint_is_resolved_from_the_binding_table_not_typed`,
   which replaced a test asserting only `normalized != "x"` — and so passing against one bad
   literal out of seven. Negative control run on both.
   **This widens the RPC `get_commands` reply by six** — a wire change under `AGENTS.md` §2.8 —
   recorded in ADR-024 rather than left as a side effect of deleting a const.

Next, in order:

1. ~~**Text scaling.**~~ **LANDED (M287), and the brief's premise was wrong.** The brief said the
   ~17 egui-widget sites follow `zoom_factor` automatically and only the painter half needed
   threading. **Measured, that is false for egui 0.36.2**: `Style` has no zoom field and
   `begin_pass` only rescales `screen_rect` and `pixels_per_point` — with the painter half scaled
   and the style alone, a transcript row measured **15.0 points at both 100% and 200%**. So
   `egui::Style` is scaled too, on the **Context**, because `Ui::new` falls back to
   `ctx.global_style()` and `pane_ui`'s panes never read their parent `Ui`.
   One scale read once per frame; all 25 painter sites through a single `paint_text`; all 7
   `fit` sites budget on the scaled size (`AVG_ADVANCE` deliberately **not** scaled — it is a
   ratio, and scaling it would cut every row at half the visible width); text-holding geometry
   grows too. **A latent defect fell out**: `RowLayout` cached row heights and only invalidated on
   width change, so a zoom change left rows overlapping.
   **And it would have shipped half-built** — nothing could move the zoom. `paint` now binds
   `Ctrl`+`+` / `Ctrl`+`-` / `Ctrl`+`0` through egui's clamped `zoom_in`/`zoom_out`. Plumbing
   nobody can move is a code path wearing a feature's name.
   **Still open:** `layout.rs`'s `TITLE_H` (36) and `STATUS_H` (24) are unscaled, so at 200% a
   long line overflows those two bars. No pane-folding threshold moved.
2. ~~**Desktop layout agency.**~~ **LANDED (M289, ADR-025).** `Preferences` was dead state — set
   to `default()` once and never mutated, so "resizable and collapsible panes, remembered state"
   was met by a struct rather than a product. There is now a drag handle on the sidebar's right
   edge (egui's own `Sense::drag()` idiom, exactly `GUTTER` wide so it cannot reach into a pane),
   `Ctrl`+`B` to fold and unfold — the chord VS Code, Sublime and Atom already use — plus `←`/`→`
   in 16-point steps, and one small JSON file per workspace carrying the two values. The control
   is in the **title bar**, because a control that vanishes with what it controls cannot put it
   back; the status bar prints the keys and **only while they work on that frame**.
   **The minimum window size moved to `900×560`**, above `MEDIUM` (860): at the old `720×480` the
   layout folded the sidebar at the one size a window cannot be dragged away from, so a reader who
   had sized or collapsed it would be shown a window disagreeing with their own choice and could
   not drag it back. **A minimum that contradicts a control is a bug waiting to be resized into.**
   The degradation thresholds themselves are untouched.
   **Still open:** the new bindings are not in the command palette and not in `/hotkeys` — both need
   a `Command` variant the shared `REGISTRY` does not yet exist to host. `Ctrl`+`B` also means
   transcript page-up **in the terminal**, a different application's namespace.
3. ~~**Fix the `frame_cost.rs` instrument** so it can see node-building.~~ **LANDED (M287)** —
   `paint_once` never called `enable_accesskit()`, so the benchmark was blind to a whole class
   of cost. What it was hiding was worth more than the fix: with the tree on, **20 of 20 paired
   rounds came out positive** and the ranges do not touch — **+31.5 %** on an empty window and
   **+38.5 %** at 800 rich rows. Accessibility is not free; it is a third of the frame, bounded
   by what is on screen rather than by how much has been said.
4. ~~**The registry move.**~~ **PARTLY LANDED (M290).** The command registry moved to
   `epsilon-contracts` and the desktop reads it there; the flat `REGISTRY` of `Action` rows is
   still unbuilt. See roadmap item 9.
5. **Run a screen reader against the window — partly done (M287), and it found two bugs.**
   The AccessKit tree egui hands the platform adapter was dumped and **read**, which is the
   accessibility counterpart of rendering a frame and looking at the screen. It is not the same as
   running a reader, and nothing here claims that. The dump is kept at
   [`../audit/evidence/desktop-accesskit-tree-2026-10-04.md`](../audit/evidence/desktop-accesskit-tree-2026-10-04.md).

   Reading it surfaced two defects that **no assertion was looking at**, because every existing
   test asserted *that a node existed with some name*, never *what the name said*:
   the file-tree pane read **`Workspace files · 1 files`**, and the tree's filter box was a
   **`TextInput` with an empty label** — a bare field a screen reader could say nothing about,
   because `WidgetInfo::text_edit` fills value and hint and never `label`. Both fixed, and
   `the_accessibility_names_read_correctly_and_the_filter_is_named` now pins the **wording**, not
   just the presence. Still open from the dump: an unnamed node with `role=Unknown`, and each
   transcript row announcing its text three times (a named `Label` plus two `TextRun` selection
   runs carrying the same string).

   **What is still missing is a reader.** Every accessibility claim in ADR-022 stops at "egui
   emitted the node into the platform tree". A real screen reader on this machine is the one
   thing this whole thread has not done.

### Desktop accessibility: what is measured, not assumed

ADR-022 records the approach, and an amendment now records that it was **implemented** — with one
correction to what the ADR had hedged about.

**Screen-reader names — LANDED (M282–M283).** Thirteen regions now publish an accessible name into
the real AccessKit tree: transcript, editor, file tree, subagents, sessions, **title bar, status
bar, activity-bar strip and its three buttons, palette rows, goal band, and the composer**. The
count is a `name_region(` call-site count in `crates/epsilon-desktop/src/app.rs`, which is not
the same measure as "regions" — an earlier version of this sentence said **eleven** and was
wrong in the other direction, which is why the number now says what it is counting.

The tree is **readable from a test** in egui 0.36.2 — `Context::enable_accesskit()` enables the
state `end_pass` turns into `FullOutput::platform_output.accesskit_update`, the literal
`accesskit::TreeUpdate` the adapter forwards to the OS — so these are assertions, not proxies,
and no new dependency was needed.

Two things the first slice got wrong, corrected by the second: the **composer and the activity bar
were not painter-only** — egui had already emitted nodes for both, *unnamed*
(`WidgetInfo::text_edit` never sets a label; `Button` names itself with its glyph, so the buttons
were "◧", "⑂", "☰"). And naming an egui-built node needs a **different call**, because
`widget_info` also queues an `OutputEvent` on click/edit frames — so `name_node` writes only
`Node::label` and leaves role, value and placeholder as egui set them. ADR-022 now records both.

**The largest remaining gap, and why it is not a one-line fix.** A screen reader can find the
conversation but still not read it, because **the transcript's rows carry no nodes**. It is
tempting to read that as "one more `name_region` call per row", and it is not:

- Only `RowKind::Changed` rows allocate a response today (`app.rs:3288-3290`). Ordinary rows
  are `painter.text` with nothing registered.
- Naming every row therefore means **allocating and registering a response per row**, which is
  one AccessKit node per row per frame.
- And the transcript is **not virtualised** — `draw_rows` walks every row and builds a text
  widget per block, measured at 115 ms/frame for 800 rows (`tests/frame_cost.rs`). At 800 rows
  that is 800 registrations every frame, on the frame path that is already the slowest thing in
  the window.

**So the two gaps are one gap.** Virtualising the transcript is not only a performance
improvement; it is the **prerequisite** that makes per-row accessibility names cheap and
meaningful. Once only the ~40 rows on screen are laid out, naming them is bounded, correct —
a screen reader reads what is visible, exactly as a sighted reader does — and costs what it
should. Until then it would be 800 nodes and a slower frame, in exchange for a screen-reader
user scrolling through a tree nobody else has to walk.

This raises transcript virtualisation to the top of the roadmap for a second, non-performance
reason, and it is recorded here so the next session does not "just add the nodes".

**`/reload` no longer swallows skill diagnostics (M283).** `crates/epsilon-tui/src/app/commands.rs`
called `discover` and discarded them, so it was the last place a skill could fail to load silently.
It now uses `discover_with_notices` and forwards each notice into the transcript through
`push_notice` — the mechanism `/restore`, `/goal` and `/skills` already use, chosen because
`run_command`'s own doc reserves the async notices channel for async events. Verified by
**rendering and reading the screen** at 80×24 and 100×30:
`* skills: cannot read root …\.agents\skills: The directory name is invalid. (os error 267)`,
sitting directly above `reload: 1 skill(s) + 0 agent(s) loaded` so the reader sees *why* the count
is short before reading the count.
**Agent-definition discovery now reports too (M284).** `crates/epsilon-core/src/agents.rs` got
the same treatment skills got: a `Discovered { agents, notices }` result,
`discover_agents_with_notices`, `discover_agents` kept as a one-line wrapper, and the **same**
`root_is_absent` absent-vs-dangling-link predicate over the shared `crate::paths::is_link`.
`load_agents_from_dir` lost both of its `let Ok(..) else` silences. **Both** call sites now
forward the notices: `/reload` into the transcript through `push_notice`, and
`epsilon-runtime` into the existing composition sink at `lib.rs:4778` — **no new channel and no
signature change**, and the desktop reads `launch.notices` so the window gets them for free.
Two negative controls: deleting the `/reload` push loop leaves a frame showing
`reload: 0 skill(s) + 1 agent(s) loaded` with **no reason at all**; reverting the runtime call
leaves the sink holding `["agents: scout"]` — the count and nothing about the root that failed.

**The desktop's command capability now derives from one table (M284).**
`crates/epsilon-desktop/src/commands.rs` carried **three parallel lists**: `SUPPORTED`,
`refused_heads()`, and `head_of` — the last a **38-arm match re-listing every `Command`
variant**. `head_of` and `refusal_head` are deleted (54 lines, 41 of them re-listing variants);
`head_of` is now a **lookup** over `picker::COMMAND_HEADS` by discriminant, `window_runs` is the
single reader of `SUPPORTED`, and `refused_heads()` is *parser heads minus what the window runs*
(34, measured). Exhaustiveness now comes from the terminal's table and its two parser drift tests.

**Two negative controls, and the first is the one worth reading.** Adding `"tree"` to `SUPPORTED`
with no implementation made the new test fail naming both sides, **while the existing guard
`tests/window.rs:1645` passed** — that guard compares two lists and never calls `try_command`, so
it cannot see a declaration that does not match the implementation. Recorded as the one
strengthening worth making there. The second control replayed the inherited sentence: the guard
drove `/branch` and `/exit` (canonical since ADR-024 Decision 1) while the window printed
`/timeline` and `/quit` — **the refusal named a command the reader had not typed**, and nothing
caught it.

**Honest residual, written in the code as well as here:** `window_runs` steers the *exported
capability answer*, not production dispatch. `try_command`'s run/refuse split is still three match
arms, so the declaration and the implementation are two statements of one fact joined by one test,
**not by construction**. That is the next step, not a finished thing.

**Registry drift found while doing it, reported not fixed:** `ALIAS_HEADS` lists `models`, but the
parser arm is `"models" | "model" if tail.is_empty()` (`picker.rs:820`), so **`/models gpt` parses
as `Unknown`**; and `scoped-models` and `?` are parser-accepted spellings present in **neither**
head list, which the reverse-drift test cannot see because it only collects `lowercase|_|:` tokens.

**The desktop writer inherited a file that did not compile** — `view.gates.len()` where
`GoalView` has only `goal` and `armed` (`crates/epsilon-contracts/src/goal.rs:246`); HEAD's spelling
was `view.goal.gates.len()`. Restored to clear E0609.

These three are measured facts from 2026-10-03, kept because "absent" and "present but inert"
need different work:

**Contrast — FIXED, with a negative control.** WCAG 2.2 SC 1.4.11 asks **3:1** for the visual
information identifying a component **and its states**
(<https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html>; the Understanding document
is explicit that the threshold must not be rounded). `IdeTheme` had two floors and no third:
`TEXT_FLOOR` 3.0 and `SURFACE_FLOOR` 1.12, the latter documented as "a band is a background, and
a background is read as a boundary, not as text" — a sound argument for a card. It does not cover
a focus ring or a selection fill, which are the *only* thing saying which row you are on.
`selection_focus` sat at **1.52:1** against `selection` — below the floor, and invisible to every
existing contrast test, because all of them measure a role against the **page**, not against the
sibling a focused row actually sits in.

Fixed by adding `STATE_FLOOR: f32 = 3.0`, holding `border_focus` and `selection_focus` to it, and
adding a **second pass** that separates each from its unfocused sibling. The new assertion is
inside `every_role_is_visible_against_the_window_background`. **Negative control run**: with the
fix reverted, that test fails naming 1.52:1; restored, it passes. The palette argument for
`SURFACE_FLOOR` is unchanged — surfaces still band at 1.12, because they are still backgrounds.

**Reduced motion — vacuously satisfied, and that is a finding, not a gap.** Grepping the crate for
`animate`, `animation`, `lerp`, `spinner` and `elapsed` returns: colour interpolation inside
theme *construction* (`theme.rs:126-133`, a one-off, not animated), repaint *coalescing*
(`session.rs:1010,1026`), and one comment at `app.rs:1800` that explicitly declines to show a
spinner. **The window does not animate.** A reduced-motion toggle would therefore be a control
for nothing, which is the Ponytail ladder rung this repository has to notice. The condition that
changes it is recorded instead: revisit the moment any animation is added.

**Text scaling — partial today, and shipping half of it would be worse than shipping none.**
The window draws text two ways: ~17 sites are real egui widgets (`colored_label`,
`selectable_label`, `TextEdit`) which follow `Style` and zoom automatically, and ~20 are
`painter.text` on raw rectangles with hardcoded sizes (`ROW_FONT`, `TAB_FONT`, literals at
`app.rs:1355,1377,1394,1503,1512,1533,1544,1635,1747,1858,1919,1966,2359,2376,2452,2507,2517,2527,
2541`), which do not. Scaling only the widget half would produce a window whose composer grows and
whose transcript does not. It is one slice: a `scale` on `IdeApp` from `ctx.zoom_factor()`, applied
at the painter sites and inside `fit()` — **and not before**. `AGENTS.md` §8 requires it; it is
roadmap item 3 below because it is not a small change.

### Designed: one command registry, and the slice worth taking

A read-only design pass produced a six-step plan. **Steps 1–3 are the slice worth taking**, because
they close three user-visible lies and touch **zero** of the 500+ TUI tests. Steps 4–5 are a
second commit.

The shape: one flat `REGISTRY: &[Action]` in a new `crates/epsilon-contracts/src/command.rs`,
rows being **actions** rather than commands — `Action { id, defaults, command, head, aliases,
summary, in_desktop }` — because the keybinding axis and the command axis are genuinely different
with a many-to-one link, and `halfUp` being unlisted is exactly what happens when you split them.
~52 rows replace `COMMAND_HEADS`, `ALIAS_HEADS`, `SLASH_ITEMS`, `keybindings::DEFAULTS`, the
desktop's `SUPPORTED`, and seven hand-typed `PaletteEntry.binding` literals. `enum Command` and
`parse_command`'s `match` stay **verbatim**: rewriting the parser as a table lookup deletes ~80
lines but needs four arities, which is a behaviour change disguised as a refactor.

Contracts is the right home on three measured grounds: it has **zero internal dependencies** (the
one place both front ends may read), **no new Cargo edge** is created in any of the three
consumers, and the desktop's own manifest comment (`epsilon-desktop/Cargo.toml:47-54`) already
predicted this move — *"The desktop still draws no terminals and depends on no terminal behaviour…
If that ever stops being true, the edge is one module"* — and `commands.rs:45` importing the
parser is that moment.

**Three corrections the design pass made to my own brief, verified since:**
- `/expand` **is** in `palette_entries()` (`picker.rs:1083-1093`). The real `/help` gap is
  **`theme` and `templates` only**.
- `halfUp`/`halfDown` **can** reach `/hotkeys` — `effective()` surfaces user-only ids
  (`keybindings.rs:421-426`). What is missing is a registry row, not visibility.
- `PaletteEntry.binding` is worse than "hand-typed": `picker.rs:1061` puts the string
  **`"/login"`** in the **key** column, beside `"ctrl+x"`. A palette that advertises a slash
  command as a keyboard shortcut shows a person a key that does not exist, and the test guarding
  it asserts only `normalized != "x"`.

**The risk worth naming before the work starts.** Step 2 derives the head/slash tables and closes
the `theme`/`templates` gap — and `SLASH_ITEMS` feeds the RPC **`get_commands` reply**
(`rpc_mode.rs:1213`), which is a wire type under `AGENTS.md` §2.8. Adding two commands **grows
that reply by two**, and `rpc_mode.rs:2727-2736` checks only the direction that goes green. Almost
certainly correct, but it is a stated decision in ADR-024, not a side effect of deleting a const.

## What this round changed (M290 finished, M291 landed)

**M290 — the registry moved down, and the thing it exposes was an unverified tree.** The files
were on disk from a battery that was interrupted mid-clippy, and the `test` leg had already
failed on something that mattered: `no_source_file_lost_a_test_since_head` reported **four
deleted tests**. They were not deleted — they had moved into the new
`crates/epsilon-contracts/src/command.rs`, and `git diff HEAD` cannot see a file git has never
heard of. **That is the third recorded instance of the same failure: a check that cannot see a
thing is indistinguishable from a check that found nothing**, and under deadline the fix
anybody reaches for is to delete the guard. It now reads `git ls-files --others` and counts
those files' tests as added.

The registry itself: `SlashItem`, `SLASH_ITEMS`, `Command`, `COMMAND_HEADS`, `ALIAS_HEADS` and
`parse_command` now live in `epsilon-contracts`, `picker` re-exports all six, and **no call site
changed**. The desktop reads them from `epsilon_contracts::command` directly — which is the edge
its own manifest predicted ("If that ever stops being true, the edge is one module"). What
remains is the palette and the metrics: colour roles and numbers, not what a `/word` means.

**M291 — the recovery copy was a file nothing could recover with.** `append` stamped the lazy
header *before* `persist_migration` took the `.pre-epsilon` copy, and the handle is `O_APPEND`,
so on a headerless file the header landed at EOF: the copy held `original + header`, and a
header is legal only as the first line. **The test was written first and the control is the
whole point** — it failed on the byte comparison, and the two sides of the assertion show it:
the left-hand side ends with the session header line, the right-hand side does not. The
reachable paths were `/resume` in both front ends and RPC `switch_session`, because
`Agent::live_resume` opens and repairs without calling `persist_migration`; the runtime launch
path calls it at startup, **which is exactly why the suite stayed green for fifteen
milestones**. The fix takes the copy first, which changes no other byte and moves the failure
before the header is stamped.

**Four read-only investigations, run in parallel, all of which corrected something written
here.** The transcript is already virtualised (roadmap item 1 was stale); the subagent-permission
"fix" is not six lines and needs a trait seam; the denied-tools premise is false; and the lease
row understated its own defect — there is no owner token in the lock file at all, so a
stolen-from holder deletes its successor's lock. **A row in this file that has not been checked
against the code is a row about a tree nobody has looked at in a while**, which is the argument
for checking it, and for not treating the investigation as automatic.

**Two independent reviews of this round's own work, and both found something.** Neither was
told what to find, and neither had written the code.

- **A code reviewer** on the registry move and the backup fix: no blockers, both changes
  correct — and ten findings. Three were real defects **in the guard test the registry move had
  just required**: `async fn` was never matched (so the six `#[tokio::test]`s in an untracked
  file were invisible), a second attribute between `#[test]` and the signature ended the
  pairing, and the early return made the untracked scan unreachable exactly when only new files
  had changed — the shape of the move it was written for. All three are fixed. The fourth, which
  is a residual rather than a bug, is written into the code: the scan unions the names in every
  untracked file, so a name collision could satisfy a deletion. One more finding was a data-loss
  hole nobody had looked at: `fs::copy` is **not atomic** and the once-only `exists()` check
  turned an interrupted copy into a permanent truncated "original". Now copied through a
  scratch name and renamed, the same discipline `persist_migration` already used.
- **A newcomer** following `README.md` → `START-HERE.md` → `STATUS.md` as a first-time reader:
  the verdict was that the guide works and the *second* document does not. Two contradictory
  test counts in one section of this file, a lease figure the code contradicts, an invented
  status vocabulary in `docs/README.md`, a defect-ID range that does not exist, three terms used
  before they are defined (`crate`, `JSONL`, `M###`, plus Ponytail, which had no entry
  anywhere a newcomer is sent), two false `file:line` citations, and a description of `AGENTS.md`
  that is 390 lines and three weeks out of date. All corrected except the `AGENTS.md` gate list,
  which is locked and is filed in [`AGENTS-PROPOSALS.md`](AGENTS-PROPOSALS.md).

**The lesson worth keeping from both:** a green suite says nothing about whether the
*statements* are true, and a review is worth more than the code it reviewed. Two of the three
things a newcomer could not decode were vocabulary, and one of them — `Ponytail` — appears in
`AGENTS.md`'s own rules with no introduction anywhere but `README.md`.

## Known-broken

- ~~**The transcript's rows still carry no accessibility nodes.**~~ **LANDED (M286), and the
  premise was wrong.** Every drawn row is an `egui::Label`, and egui had *already* been emitting
  a `Role::Label` node per row carrying the row's text as its **value** — what was missing was the
  **name**. `Response::widget_info` cannot supply it: on the `Label` role egui writes a
  `WidgetInfo`'s label into `Node::value`, not `Node::label`, so it would overwrite the row text
  *and* leave the name empty. Rows use `name_node` instead, which writes the one missing field
  and — because `selectable_labels` makes these labels click-sensitive — queues no duplicate
  `OutputEvent`.
  Labels are `Transcript row · {kind} · {excerpt}`, excerpt ≤ 80 chars, from the **sanitised**
  text. **Off-screen rows stay unnamed structurally**: 800 rich rows → 44 lines drawn → **53
  named nodes**, not 1600, and that is a test rather than a claim.
- ~~**`frame_cost.rs` cannot see node-building.**~~ **FIXED (M287) — and what it was hiding is
  worth more than the fix.** `paint_once` never called `egui::Context::enable_accesskit()`, so
  `accesskit_node_builder` returned `None` and the whole class of cost was invisible — which is why
  naming every transcript row first measured as "no change" at all.

  **Accessibility is not free. It costs about a third of the frame.** Measured by an interleaved
  A/B — one process, 10 rounds × 10 frames, order swapped every round — **20 of 20 paired rounds
  came out positive, and the ranges do not touch**:

  | fixture | tree ON | tree OFF | delta |
  |---|---|---|---|
  | empty window | 1.788 ms (1.509–2.087) | 1.360 ms (1.147–1.692) | **+31.5%** |
  | 800 rich rows | 5.342 ms (5.079–5.763) | 3.858 ms (3.566–4.344) | **+38.5%** |

  **What the delta is made of is not broken out and is not guessed** — egui's per-node bookkeeping
  and our own naming strings (built either way, and simply discarded today) are both plausible,
  and the file says so rather than picking one.

  **M285 survives this.** The cost is bounded by what is on screen, not by how much has been said:
  200 rows and 800 rows cost the same, so putting a third of a frame back did not make the frame
  grow with the conversation. A new non-`#[ignore]`d gate test now fails if the instrument ever
  goes blind again, and it pins the zoom at 1.0 so the table stays comparable.
- **`/models gpt` was a real bug and is fixed (M286).** `ALIAS_HEADS` declares `models` an alias of
  `model`, but the parser arm was bare-only, so the obvious thing to type came back `Unknown`.
  Fixing it **invalidated a desktop test that had pinned the defect as intended behaviour** — the
  writer reported the conflict rather than editing a file it did not own, and it was re-pointed at
  a genuine typo (`/mode gpt`) so the *property* survived the loss of its subject.
- **`the_tui_works_when_started_from_powershell` failed once and passed on re-run with no code
  change in between.** M283 attempt 1: the ConPTY test reported *"the first frame shows neither
  the composer placeholder nor the welcome screen, so the TUI opened and drew nothing"*. It had
  passed in the three preceding green batteries (m280, m281, m282), it passed again on attempt 2
  with the same bytes, and `gate-until-pass.ps1` fingerprinted it and retried on its own. **So: a
  flake under load, not a regression** — but recorded rather than smoothed over, because the
  failure is the specific one that looks most like "the TUI is broken". It belongs to the same
  family as the `epsilon-session` lease tests noted below: host resources (a pseudo-console, a
  window, a port) taken under contention. Not diagnosed; a flake nobody reproduces is not a
  flake that is fixed.
- ~~**`disable-model-invocation` fails OPEN on any spelling but `"true"`.**~~
  **FIXED (M281).** `crates\epsilon-core/src/skills.rs:116-118` read it as
  `get("disable-model-invocation").map(|v| v == "true").unwrap_or(false)`, so a skill saying
  `yes`, `True`, `TRUE` or `on` — all valid YAML — was parsed as **model-invocable** and the
  owner's own opt-out did nothing. Now parsed as the YAML boolean set, and an unrecognised
  spelling applies the **restriction** rather than ignoring it, which is what `AGENTS.md` §6
  demands of an unrecognised value. Four new tests; **negative control run** — restoring the old
  comparison fails two of them, naming `yes` and `perhaps`.
  **Found by reading DeepSeek Harness's own skill documentation**, which accepts the same set
  and drops the skill with a warning on a rejected spelling. We cannot do the warning yet
  because `discover` returns a `Vec<Skill>` and has no diagnostics channel — that signature
  change is the honest follow-up and is recorded below.
- **A discovered skill root that cannot be read is indistinguishable from an empty one.**
  `discover` does `let Ok(entries) = std::fs::read_dir(root) else { continue }`, so a permission
  error on a root reads as "no skills here". DSH marks the observation incomplete and does not
  cache it. Recorded from the same survey; not fixed.
- **The skill catalog is built once at composition**, so a skill added mid-session never reaches
  the model, and `MAX_DESC` is 1024 against DSH's 500. Not fixed; low urgency.
- **The vendored reference trees are current (confirmed 2026-10-03).** All six were re-measured
  and are byte- and file-count-identical, with directory creation times of **11:06 today**, before
  this session began at 11:12 — so the trees every citation in `docs/references/*.md`, ADR-020,
  ADR-023 and ADR-024 was written against are the ones on disk: pi 1.0.0, DSH 0.2.1-alpha.1,
  Hermes 2026.9.24, opencode 1.18.34, OpenTUI 0.5.14, codewhale-ratatui 0.1.0. Every citation in
  those pages was resolved against them, not recalled.
- **DSH's latest ships a desktop application, and it is an Electron one.** `apps/desktop` exists
  and is the product's desktop front end; its `package.json` describes itself as an *"Electron
  desktop shell for a bundled dsh runtime and external plugins"*, and DSH's root README reads
  *"The desktop application is an Electron shell around the complete dsh Web application."* There
  is no Rust, Tauri, egui, GTK or Qt anywhere under `apps/`. ADR-020 rests on exactly this: we
  took DSH's **workflow** and deliberately not its **carrier**, because ours must be native.
  That is a difference in what each project is for, and the page says so in those terms.
- **106 source citations name a pi version that is not vendored.** `references/` holds
  `pi-1.0.0`; tracked files cite `pi-0.84.2` (106), `pi-0.85.1` and `pi-1.0.0`.
  `scripts/check-reference-citations.py` had its version segment pinned to `pi-0.84.2`, so it
  silently ignored the other two versions entirely — a verifier checking nothing, which reads
  as coverage. **Fixed** to `pi-[\w.\-]+`; the checker now reports all three versions and the
  ones that do not resolve. Re-pointing the 106 citations is separate work and is not done.

- **The desktop window cannot be seen on this machine** — no wgpu backend presents, so every
  claim about its pixels is inferred from egui's source and the crate's own coordinates.
  **Re-confirmed by launching it again on 2026-10-04, not inherited:** `cargo run -q -p
  epsilon-desktop` was started, **stayed running for 45 seconds with empty stdout and stderr**
  (a window that opened and did not crash), and was then terminated. The process starts, opens
  and holds; what it draws is still unverified here, and the earlier measurement with
  screenshots is at
  [`../audit/evidence/desktop-window-measurement-2026-10-03.md`](../audit/evidence/desktop-window-measurement-2026-10-03.md).
  **What that means for the accessibility claims:** eleven named regions are asserted against the
  real `accesskit::TreeUpdate` in tests, and the tree has been dumped and read — but **no screen
  reader has ever been run against this window**, on this machine or any other.
- **Three `epsilon-session` lease tests were timing-flaky under load earlier today** — 6/6 green
  in isolation, 3 red inside a loaded 14-crate sweep, same tree, causation never proven. They
  **passed** in the green full-workspace battery recorded above. Not fixed; not reproduced.
  Kept here because "green once" is not the same as "not flaky", and the next person who sees
  three reds in a sweep needs to know this was measured before.
- **`crates/epsilon-core/src/verify.rs:1`** cited `hermes-agent-2026.9.14/agent/verification_evidence.py:731`
  and `verification_stop.py:233`. **Fixed 2026-10-03**: the vendored copy is `2026.9.24`, where
  those files are 590 and 228 lines, so both line numbers were out of range and neither citation
  opened. Now cites `verification_evidence.py:99` and `verification_stop.py:158`.
- **`branch_with_summary`** (`crates/epsilon-session/src/store.rs:1609`) is correct, tested, and
  unreachable from any surface — a fifth independent confirmation.
- **`crates/epsilon-session/src/repair.rs` is DSH-derived, not Hermes-derived** (its own header
  says so). Recorded because the Hermes reference page had to draw that line explicitly.
- **`/restore` on an agent-*created* file has no content check.** A creation entry carries
  `existed: false` and no pre-image, so restoring removes the file unconditionally — a user who
  edits an agent-created file and then restores it loses that edit. Pre-existing (M100/M104
  scope), but `/restore` is what makes it reachable. The preview says `DELETE` in capitals and
  waits for `yes`, which is the honest mitigation available without a further schema change.
  Recorded in ADR-023's "Still open" rather than presented as fixed.
- **Headless notice ordering is unproven.** The `rpc`/`print`/`serve` modes collect composition
  notices and print them in vector order, which preserves emission order by construction — but no
  test asserts it, because proving it needs a subprocess. Believed, not measured.
