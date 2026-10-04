# Proposals for `AGENTS.md`

`AGENTS.md` is **locked**: maintainer-owned, SHA-256 in `AGENTS.md.sha256`, enforced by
`scripts/check-agents-lock.py` in both the gate battery and CI. **No agent or subagent may edit
it.** An agent that can rewrite the rules it is held to is not governed by them.

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

## Closed proposals

_Moved here when a proposal was applied and the lock re-issued. Kept, because deleting a
record of what someone once proposed is its own kind of dishonesty — and because the next
person to propose the same thing deserves to know it was already considered._

_None yet._
