# Proposals for `AGENTS.md` — history since the 2026-10-15 unlock

`AGENTS.md` was maintainer-locked (SHA-256 in `AGENTS.md.sha256`, enforced by
`scripts/check-agents-lock.py`) until the owner removed the lock outright.
Corrections now go directly into `AGENTS.md`. Everything below is the record
of what the lock era proposed, kept because the reasoning is still worth reading.

If you think `AGENTS.md` is wrong, or is missing something, **append a proposal here**
and mention it in your final report. The maintainer applies proposals and re-issues the lock.

## How to file one

Keep each entry short. State the current text, the proposed text, and why — in particular
whether you have evidence that the current wording is wrong (a command that does not run, a
file that does not exist, a claim contradicted by the code), or whether it is a preference.

## Open proposals

Five, filed 2026-10-03. The first came from a battery failure; the other four from reading the
reference harnesses' own agent conventions, which is where a rule that a working project has
already had to learn actually comes from.

**Later filings:** 6 (2026-10-04, by a newcomer review), 7 (2026-10-05, by the parity-matrix
inventory), and 9 and 10 (2026-10-06, by
[an independent review of the objective](reviews/2026-10-06-objective-review.md)); **8 is closed
below** because the work was done.

### 1. A recorded gap is not a task

**The gap.** §5 says "Deletion beats addition. A refactor that also changes behaviour cannot be
reviewed as a refactor." That is right, and it has an unintended consequence: an agent finding
something unfinished has every encouragement to *finish* it. Several of our boundaries are
deliberate — the desktop's `existed: false` restore hole is one this session found and recorded
rather than fixed — and nothing tells an agent a recorded gap is *frozen* rather than *queued*.

**Proposed**, as a new §11.1:

> **A recorded gap is not a task.** Some boundaries are deliberate at the moment they were
> written. `docs/STATUS.md` records what is true right now and moves; it is not a backlog to work
> from unprompted. When a gap is deliberate, say where it lives and what would change the
> decision. Do not "fix" a boundary a previous decision recorded on purpose.

**Why.** The single most useful idea in the eighteen opencode agent guides, and we have nothing
like it. One sentence, and it is the difference between a recorded decision and a standing
complaint.

### 2. Multi-session git safety

**The gap.** §12 assumes subagents **we** orchestrate, with file ownership. It does not model two
*independent* sessions in one tree — which is exactly what happened this session, when another
agent edited `epsilon-runtime/src/lib.rs` and `epsilon-ext/src/host.rs` at 11:28 and 11:32 while a
battery ran against them.

**Proposed**, appended to §11:

> **Destructive git commands destroy other sessions' work.** `git reset --hard`,
> `git checkout .`, `git clean -fd`, `git stash`, `git add -A` and `git commit --no-verify` are
> not safe here. Stage by path. Review another session's work with `git show <ref>:<path>` or
> `git diff <ref>`, never by checking it out.

**Why.** `git stash` and `git add -A` are on none of our lists, and both destroy a concurrent
writer's uncommitted work.

### 3. Ask before removing functionality that looks intentional

**The gap.** §11 covers deleting *files* — "check `git status` that it is untracked and
regenerable". Removing a **capability from working code** is a different act with no rule, and
§5's "Deletion beats addition" pushes the other way.

**Proposed**, one line in §11:

> Removing a capability from code is not the same as removing a file. If something looks
> deliberate, ask before removing it, however dead it looks.

**Why.** A terminal feature that appears unused is often a pi-parity decision recorded in a
comment — `app.transcript.halfUp`/`halfDown` are exactly that, and deleting them because no
binding references them would remove a documented decision.

### 4. When a preview is blank, **stop** visual verification

**Proposed**, one bullet in §5's evidence discipline:

> A successful read or render does not prove the frame contains content. A uniformly blank or
> fully transparent output is a renderer failure, not evidence about the thing being rendered —
> stop visual checking for it immediately rather than re-running the check with variations.

**Why.** Sharper than "check whether the frame is the bug": it adds *stop*, not just *check first*.
Directly on point for the egui/wgpu desktop path, where a frame can legitimately come back empty
and a retry loop then burns a lot of time proving the same thing. From DSH's Office skills, whose
exact phrasing is worth keeping: *"a fully transparent or uniformly blank image is a renderer
failure, not evidence that the workbook needs fixing."*

### 5. `CARGO_PROFILE_TEST_DEBUG` — name the *value*, not just the knob

**Filed:** 2026-10-03, by the session that produced the first green full-workspace battery.

**Current text** (`AGENTS.md` §4, "This machine"):

> **`LNK1104` on a freshly linked test executable is a machine fault, not a defect.** Avira
> flags some paths and the link fails on a file that does not exist. Never change security
> configuration from an agent session. Rebuild the test profile under fresh metadata
> (`CARGO_PROFILE_TEST_DEBUG=1`) and disclose the knob in the log.

**Proposed text:**

> **`LNK1104` on a freshly linked test executable is a machine fault, not a defect.** Probing
> settles it: creating a file at the blocked name is denied, while other names in the same
> directory are writable. Never change security configuration from an agent session. Rebuild the
> test profile under fresh metadata — **using a `CARGO_PROFILE_TEST_DEBUG` value this machine has
> not used before** — and disclose the knob in the log.

**Why.** The rule as written stopped working after two attempts, and the reason is the
interesting part. The block binds to one **exact filename**, not to a pattern: on this machine,
`CARGO_PROFILE_TEST_DEBUG=0` produced a blocked name, `=1` produced another blocked name, and
`=2` produced names the machine had never seen — and the battery went green on the first attempt
(1,922 passed, 0 failed; `audit/evidence/gate-base-2026-10-03.log`).

So the rule is right and incomplete. "Set the knob" reads as "set it to 1", and once 1 has been
used the remedy silently degrades into a failed gate that looks exactly like a code defect.

---

## 6. The gate list in §4 names nine gates and omits the lock check

**Found by.** An independent newcomer review, 2026-10-04, following `README.md` → `START-HERE.md`
→ `STATUS.md` and checking every claim against the tree. The first defect it found was in the file
newcomers are told to obey.

**The gap.** §4 says:

> **The battery runs all of it plus four repository gates:**
>
> Gates, in order: `check-links`, `check-lint-tables`, `check-line-endings`,
> `check-control-bytes`, `fmt`, `clippy`, `test`, `demo`, `real-check`.

`scripts/gate.ps1:181-214` and `scripts/gate.sh:54-84` both run **ten**, and `check-agents-lock`
is one of them (`gate.ps1:185`). It is the check that enforces the lock this file's own header
describes, and the section that lists the gates does not name it.

**Proposed**, replacing the paragraph above:

> **The battery runs all of it plus five repository gates:**
>
> Gates, in order: `check-links`, `check-agents-lock`, `check-lint-tables`, `check-line-endings`,
> `check-control-bytes`, `fmt`, `clippy`, `test`, `demo`, `real-check`.

**Why.** `README.md` and `docs/STATUS.md` both already say ten and enumerate them. A newcomer who
reads `AGENTS.md` first — which is what §7 tells them to do — is told the lock is not part of the
battery, while the same file's header tells them CI enforces it. One number, three documents, two
answers; the locked one is the outlier.

**The same review found a second stale pointer**, which needed no proposal because the file it is
in is not locked and has been fixed: `scripts/gate.ps1:9` sent the reader to `AGENTS.md`'s
"Windows notes", and v2 calls that section **"This machine"**.

---

## 7. Provider credentials are on disk in plaintext, and §6 forbids that

**Found by.** A read-only source inventory of `crates/epsilon-tui` for the parity matrix,
2026-10-05. Reported through [`STATUS.md`](STATUS.md) per §14 rather than acted on.

**The contradiction, exactly.** §6 says:

> **Never write a credential to disk in plaintext.** Secrets belong in the OS credential
> store. Never accept one on a command line.

The product writes every provider credential — API keys **and** OAuth token pairs — to
`<global config dir>/auth.json`, i.e. `~/.config/epsilon/auth.json` on this platform:

- `crates/epsilon-llm/src/auth.rs:1-23` — the module doc states the design plainly, and
  names its model: *"opencode's `auth.json` pattern (M22)"*.
- `crates/epsilon-llm/src/auth.rs:6` — the path.
- `StoredCredential::Api { key, .. }` and `StoredCredential::OAuth { access, refresh, .. }`
  (`auth.rs:47-60`) are both `Deserialize` from that file, so a refresh token is stored the
  same way an API key is.
- Measured, not assumed: **`keyring`, `wincred`, `secret-service` and `keychain` appear
  nowhere in `crates/**/*.rs`, and `keyring` is not in `Cargo.lock`.** There is no OS
  credential store in this project at all.

**What the code does get right**, so the decision is not one-sided. The store takes a
cross-process `auth.lock` with stale-lock stealing, persists through a same-directory temp
file plus atomic rename, enforces `0600` on unix, and **refuses to overwrite a corrupt file**
so a failed-open read cannot become a wipe (`auth.rs:15-19`). Those are the right disciplines
for a file that holds a secret. The problem is that it is a file.

**Why no agent fixed it.** Three reasons, stated so the maintainer can weigh them:

1. It is a **behaviour-level change** with a wire format and a migration, which §5 says wants
   an ADR.
2. It needs a **new dependency** — an OS keychain crate — which §5 also gates behind an ADR.
3. The objective's own desktop standard repeats the requirement (*"secrets in the OS keychain,
   not plaintext"*), so this is not a rule only the terminal happens to bend.

**Two ways to close it. The maintainer should pick one.**

**(a) Change the rule to match the product**, as a statement of what is actually enforced:

> Provider credentials live in `<config dir>/auth.json`, in the opencode and Pi shape.
> They are written `0600` on unix, via atomic rename, under a cross-process lock, and a
> corrupt file is never overwritten. **The OS credential store is the target, not the
> current state**; until it is adopted, this file is a known plaintext store and every
> document that says otherwise is wrong.

**(b) Change the product to match the rule**: adopt an OS keychain, keep `auth.json` as an
import/export format for CI, and migrate on first read. This is the better answer on the
merits — a 0600 file on a machine with malware, a backup, or a synced home directory is not a
secret store — and it is what the objective asked for. It is also a day of work, and it should
be scheduled as its own milestone rather than folded into a hardening round.

**Recommendation: (b), with (a)'s wording adopted for the interim** so that the locked file
describes the product truthfully in the meantime. Filing a rule that the tree violates is
worse than either fixing the tree or fixing the rule.

---

## 8. ~~§2.4's "one command registry" describes keybindings the tree does not have~~ — **CLOSED by the work, 2026-10-05 (M315, ADR-031)**

> **Status: resolved in the tree, so the proposal is withdrawn rather than answered.** The
> registry now exists, the window reads it, and `§2.4` is true of the code for the first time.
> The entry is kept below because a later reader who remembers the gap deserves to know it was
> real, and because the *shape* of the fix is a decision someone may want to revisit.

**Found by.** A source survey of `crates/epsilon-desktop` for the parity matrix, 2026-10-05,
confirmed in the integrator's session against the source rather than relayed.

**§2.4 says:**

> **One command registry.** Slash commands, palette entries and keybindings are declared once
> and read by every front end, with per-front-end capability declared as data rather than as a
> second hand-maintained list.

Measured:

| | Shared? |
|---|---|
| Slash commands | **yes** — `epsilon-contracts`, read by both front ends |
| Palette entries | **partly** — the desktop's eight are declared in `crates/epsilon-desktop/src/app.rs:806-838`, not in the shared table |
| **Keybindings** | **no** |

`keybindings`, `KeybindingsManager` and `key_text` appear **nowhere** in `crates/epsilon-desktop`.
Every chord is a literal egui `consume_key` — `Ctrl+P` at `app.rs:880`, `Ctrl+B` at `:1733`,
`F6`/`Shift+F6` at `:1815-1816`, the tree arrows at `:1850-1855`, zoom at `:2561-2564`, submit
and abort at `:5165-5168`, allow/deny at `:5353-5358` — and the on-screen hint comes from a
hand-written constant, `SIDEBAR_TOGGLE_KEY: &str = "ctrl+b"` (`app.rs:759`).

**The code is honest about it.** The comment above that constant says the registry *"does not
exist yet"* and that the two front ends are separate namespaces. **§2.4 is the thing that
overstates**, because a reader who trusts it concludes one table governs both front ends.

**Proposed**, replacing the last sentence of §2.4:

> …with per-front-end capability declared as data rather than as a second hand-maintained
> list. **This holds for slash commands today and not yet for keybindings:** the terminal reads
> `epsilon-tui`'s binding table and the desktop window's chords are literals in `app.rs`. Do
> not write "the binding table is honoured" about the window.

**Why.** The locked file is what every agent reads first, and §13 asks an agent to treat it as
the contract. A rule that is true of one front end and false of the other will be cited as if it
covered both — which is how the window's shortcuts came to be un-rebindable without anyone
noticing that was a choice rather than an accident.

**If instead you would rather build it**, the honest scope is a shared table in
`epsilon-contracts` plus migrating 17 read sites, with a capability flag per front end so the
terminal's `ctrl+b` (page-up) and the window's `ctrl+b` (fold sidebar) can differ. That is its
own milestone, not a cleanup.

### What was actually done, and the one place the answer differs from this proposal

**ADR-031**, M315. The grammar (`Chord::parse`) moved to `epsilon-contracts`; the terminal's
table moved with it; and the window's seventeen literals became a second table in the same module.
**No capability flag** — the proposal suggested one, and ADR-031 argues against it: two adjacent
tables are easier to read than one table with a conditional on every row, and `§2.4` asks for
per-front-end capability "declared as data", which two tables in one format are.

**The window's shortcuts are now rebindable from the same `keybindings.json` the terminal reads.**
That is the capability this proposal identified as missing, and it is real.

**What is still missing, so the withdrawal is not overclaiming:** nothing *writes* the file (so it
is not atomic or 0600, because it is not written), and there is **no recorder** — a user can set a
binding by hand but cannot record one with a keystroke, which is the reference's S-122 and the
largest remaining piece of this row.

---

## 9. §13's definition of done has no clause about the *deliverable*

**Filed:** 2026-10-06, by [an independent review of the objective](reviews/2026-10-06-objective-review.md).

**The gap.** §13 is a good *engineering* definition of done — a fresh clone builds, the battery is
green, changed behaviour has a test, docs are current, credits are complete. Every one of those was
true of this tree while the objective it was built for was **11.5% and 2.5% complete** by the
project's own parity matrices. Nothing in the locked file asks what the deliverable *is*, how
complete it has to be, or who decides that it is finished.

**Proposed**, as a new first checkbox in §13:

> - [ ] **The deliverable is named, and its completeness criterion was agreed with the owner before
>   the work started.** A green battery proves the tree is sound; it does not prove the thing asked
>   for exists. When the task is a port, a migration or a replacement, the criterion is a count
>   against the thing being ported — not a list of defects closed.

**Why.** The single largest structural cause of this project not meeting its objective was that the
objective was never operationalised: the only instrument that could measure it (the parity matrices)
was written on the last day, and it is still untracked in git. One checkbox would have forced that
instrument to exist on day one. It is also cheap: the matrices are already the right shape.

---

## 10. §9's "every link must resolve" is enforced for links and not for paths

**Filed:** 2026-10-06, by the same review. **Measured:** 36 repo-relative paths written in backticks
under `docs/` resolve to nothing, and `check-links.py` cannot see any of them because it matches
markdown links only. Live examples: `docs/plans/README.md` sends a reader to
`audit/04-roadmap.md` (the file is `audit/history/04-roadmap.md`, and `audit/history/` is excluded
from the gate on purpose), and `docs/AGENTS-PROPOSALS.md` cites `gate-base-2026-10-03.log` for a
battery whose file is `.txt`.

**A second measurement, made while filing this.** The same naive matcher reads a *link written
inside a code span* as a real link: quoting the syntax in this very proposal failed the gate on the
word `target`, and `docs/reviews/2026-10-06-objective-review.md` failed it on an empty-label
example. The gate is therefore wrong in both directions — blind to code-span paths, and credulous
about code-span link examples. Both are the same defect: it parses characters without knowing
whether they are prose, code or a path.

**The current text** (§9):

> **Every command in the docs must have been run. Every link must resolve.**
> `scripts/check-links.py` is a gate; the commands are on you.

**Proposed**, replacing the second sentence:

> **Every command in the docs must have been run. Every link must resolve, and so must every
> repository path written as a code span** — the gate matches markdown links and cannot see a path
> in backticks, which is how a moved file stays plausible for months. A path that
> names a file which does not exist yet (a proposal, a roadmap item) says so in the same sentence,
> so the check can tell a proposal from a stale pointer.

**Why.** This is §5's own failure mode — *a check that cannot see a thing is indistinguishable from
a check that found nothing* — and it is measurable, which is what makes it worth a rule: the
checker is a dozen lines, and 36 dead pointers is what its absence cost.

---

## 7b. Measured addition to proposal 7: on Windows the file has no explicit protection at all

**Added 2026-10-06 by the same review, because it sharpens the choice rather than changing it.**

Proposal 7 says the store *"enforces `0600` on unix"*. Read against the source, the qualifier is
doing more work than it looks: the permission call is `#[cfg(unix)]`
(`crates/epsilon-llm/src/auth.rs:775-780`), the test that pins the mode is `#[cfg(unix)]` too
(`:1133`), and the code's own doc comment states the Windows position plainly — *"Windows relies on
the user-profile ACL, which is already user-scoped"* (`:345-346`).

**So on the platform this project is developed and run on, the plaintext secret file has no
protection beyond the ACL it inherits from the profile directory.** That does not change the
recommendation — (b), the OS credential store, with (a)'s wording as the interim — but it does mean
the interim wording must name the platform, not just the mode:

> Provider credentials live in `<config dir>/auth.json`, in the opencode and Pi shape. On unix they
> are written `0600` via atomic rename under a cross-process lock, and a corrupt file is never
> overwritten. **On Windows the file inherits the user-profile ACL and has no protection of its
> own.** The OS credential store is the target, not the current state.

---

## 11. The gate list in §4 is stale again: eleven legs, and proposal 10 is half-answered

**Filed:** 2026-10-06, full-port round 22.

**Current text** (§4):

> Gates, in order: `check-links`, `check-agents-lock`, `check-lint-tables`, `check-line-endings`,
> `check-control-bytes`, `fmt`, `clippy`, `test`, `demo`, `real-check`.

**The gap.** Both gate scripts now run eleven legs: `check-parity-citations`
(`scripts/check-parity-citations.py`) was wired in after `check-lint-tables`
(`gate.ps1`, `gate.sh`), because the export move deleted 53 lines and silently
re-pointed nineteen matrix citations in round 14 — all repaired by hand, the
twentieth such move should fail the build instead. The script checks 464
citations across both matrices (workspace paths with `:line`/`:start-end`;
reference-side paths excluded like `check-links.py` excludes `references/`).

**Proposed:** add `check-parity-citations` to the §4 list in the same position.

**Relation to proposal 10.** Proposal 10 measured 36 dead backticked paths under
`docs/` and asked that every code-span path resolve. This gate answers a subset:
citations *with line numbers* in the *two matrices*. It does not touch bare
paths, other documents, or the proposal-vs-stale-pointer distinction — proposal
10 stays open in full. Noted here so the maintainer can see the overlap.

**Update, full-port round 65.** Now thirteen legs: `check-verb-counts` (round 49)
and `check-parity-counts` (round 50) joined since. Same fix, longer list.

---

## 12. Name the objective's scope: §1 (a), (b), or (c)

**Filed:** 2026-10-07, full-port round 65.

**The gap.** Plan 025 §1 asks the owner to pick one of three readings — (a) ratify
plus must-have subset, (b) actually port DSH Desktop, (c) freeze and release —
and assumes (a) throughout. Sixty-five rounds later no answer is recorded, and
the tree kept growing under the assumption: the window answers 36 of 37 heads,
Pi stands at 49 / 118 / 151, DSH at 17 / 92 / 282. Every further round spends
against an unratified scope.

**Proposed:** record the pick in plan 025 §1 (one letter plus the must-have row
list for (a), or the invariant change for (b)), and close or re-scope the
matrices accordingly. Until then the matrices stay the scoreboard and (a) stays
the working assumption — stated here so the assumption is visible.

**Answered 2026-10-07, round 65: (b) — actually port DSH Desktop — under the
native-window invariant (§2.3 stands).** Every portable DSH surface is in scope;
Electron-only chrome stays `missing` permanently, to be rescoped row by row.
Recorded in plan 025 §1.

## 13. Empty window with no session: choose the behavior

**Filed:** 2026-10-07, full-port round 65.

**The gap.** A window opened with no session file (fresh launch, all sessions
deleted) shows the agent pane's empty state and a working composer whose sends
are refused for lack of an agent. Nothing is broken, but no decision is
recorded: offer a session, start one unasked, or leave the empty state as the
answer. The longer it stays undecided, the more code guesses around it.

**Proposed:** pick one — (i) empty state stands (current behavior, document it),
(ii) auto-start a session file on first send, or (iii) a first-run offer —
and pin it in a test. Recommended: (i); the state is honest and the composer
says why it refuses.

**Answered 2026-10-07, round 65: (i).** Pinned by
`an_empty_window_says_so_and_still_asks` (empty state shown, close still asked,
nothing invented and nothing auto-started).

## 14. Release scope and vehicle

**Filed:** 2026-10-07, full-port round 65.

**The gap.** Plan 025 §I (release) is open: one tag (`v0.1.0`), no artifact,
install is `cargo install --path`. Sixty-five green batteries later there is
still no thing a person can install, and no recorded scope for one — preview
with the matrices as the gap list, or wait for (a)'s must-have list.

**Proposed:** pick the vehicle (cargo-install notes vs. attached binaries) and
the scope (preview now vs. after the must-have list), then §I proceeds. If
preview now, the round that builds it also runs the clean-install transcript
§I demands.

**Answered 2026-10-07, round 65: preview now, vehicle is cargo-install notes.**
Next: write the install + scope notes and run the clean-install transcript.
**Next focus, same round: permission presets** (plan §C gap 2) — design first,
no owner input needed.

## 15. AGENTS.md still describes the removed desktop

**Filed:** 2026-10-12, revised-goal rework after the completion audit.

**The gap.** ADR-039 removed `crates/epsilon-desktop`, but `AGENTS.md` — which holds
"only what stays true" — still states it: §1's "one core and two front ends" with the
`epsilon-desktop` bullet and "The desktop must stay a real desktop application"; §2.1
("the terminal and the desktop are front ends on it"); §2.3 (the native-desktop
stack rule, ADR-020, now superseded); §2.4 (palette entries "derived from each front
end's `SUPPORTED` subset" — the desktop `SUPPORTED` table is gone; the terminal reads
`COMMAND_HEADS` directly) and the `TERMINAL` / `WINDOW` tables (the `WINDOW` table's
only reader is gone, retained deliberately for the TUI IDE per its doc comment);
§3's `epsilon-desktop` repository-map row; §4's 1.95 toolchain split, the MSRV-job
split, and `cargo run -q -p epsilon-desktop`; §8's "applies to the desktop IDE first".
Evidence: the crate is gone (`git log 8992fec`), the workspace is 13 crates on uniform
1.88, the battery is green without it. A newcomer reading the lock file first — as
`START-HERE.md` instructs — meets a product that contradicts the tree on the first page.

**Proposed**, as a maintainer edit with lock re-issue (agents must not touch the file):
restate §1 as one core + one terminal IDE (+ headless/HTTP/RPC modes), fold §2.1's
sentence to modes, mark §2.3 superseded by ADR-039 with its revisit trigger, repoint
§2.4 at the `TERMINAL` table (+ `WINDOW` retained-for-IDE note), drop the §3 row, and
restore §4/§8 to a single toolchain and the terminal. `docs/START-HERE.md` (EN+AR),
`docs/README.md`, `docs/HANDOFF.md` and the root `README.md` already read that way —
the lock file is the last document still describing two front ends.

---

## 16. §2.4 says the `WINDOW` keybinding table is read by every mode; nothing reads it

**Filed:** 2026-10-14, production-readiness audit. **Measured on the tree, not recalled.**

**The current text** (§2.4):

> Slash commands (`COMMAND_HEADS`) and palette entries (derived from `COMMAND_HEADS`) and
> keybindings (the `TERMINAL` table, plus the retained `WINDOW` table) are declared once in
> `epsilon-contracts` and read by every mode, with per-mode capability declared as data rather
> than as a second hand-maintained list.

**The measurement.** `WINDOW` (`crates/epsilon-contracts/src/keybind.rs:457`) has **no reader
outside its own module**: `grep -rn "WINDOW" crates/epsilon-tui/src crates/epsilon-runtime/src
crates/epsilon-cli/src` returns nothing (the hits in `epsilon-core/src/util.rs`,
`epsilon-safety`, `epsilon-tools` are the word inside other identifiers —
`CASCADIA_HOSTING_WINDOW_CLASS` and friends). Its only consumers are the `contracts` re-export
at `crates/epsilon-contracts/src/lib.rs:34` and its own self-tests
(`keybind.rs:599,617,637`). `TERMINAL` is genuinely read by `epsilon-tui`'s
`KeybindingsManager`.

**Proposed**, replacing the parenthetical:

> keybindings (the `TERMINAL` table — the `WINDOW` table is retained as the record of the
> removed desktop's chords, read by nothing today)

**Why this is a proposal and not an edit.** §2.4 is invariant law and this is a factual
claim about the code, not a preference — the same shape as proposal 15, whose text
("retained deliberately for the TUI IDE") this sharpens: retained it is, read it is not.
**Recommendation:** either delete `WINDOW` outright (the desktop is gone, ADR-039; the table
is 40-odd rows of dead data with self-tests holding it) or keep it deliberately as history
with the wording above. Deletion is the smaller tree; the second is the smaller diff.

---

## Closed proposals

_Moved here when a proposal was applied and the lock re-issued. Kept, because deleting a
record of what someone once proposed is its own kind of dishonesty — and because the next
person to propose the same thing deserves to know it was already considered._

_None yet._
