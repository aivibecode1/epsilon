# 03 — Defect ledger

Confirmed defects, ranked by what they cost the user. Every row names the line
that has to change, because a defect report that does not end in an edit is a
complaint.

Status vocabulary: **Open** (not fixed), **Open, partly fixed**, **Claimed-fixed
but unverified** (a milestone says so; this audit does not believe it), **Fixed**.

---

# Reconciliation, 2026-09-29 (M206)

**A7 was already fixed when the ledger was written, and the ledger said it was open.** Re-derived
here from the code, which is what the reconciliation passages below exist to do:

| Claim in the row | What the code does |
|---|---|
| "`session/src/lease.rs:88` `STALE_AFTER = 30s`, still live" | True of the *constant*, false of the behaviour. `SessionLease::acquire` has two paths since the fix: a lock whose mtime has not moved across a full `HEARTBEAT` belongs to a process that is not running, and it is stolen after one heartbeat rather than after the window. `a_waiting_acquirer_takes_the_lock_the_moment_it_goes_stale` pins it (200× time-compressed), and its companion `a_waiting_acquirer_still_refuses_a_heartbeating_holder` is what makes the pair worth anything — shrinking the bound without it would be stealing from the living. |
| "the TUI shows nothing (the alternate screen erases the notice)" | The wait **is** rendered: `write_lease_wait` draws `waiting for this session to be released — 4s elapsed, ~26s left` with `\r` and `\x1b[K`, and the lease module's doc carries the frame measured in a real conhost at 100×30. |

Both user-visible consequences the row named are therefore gone: a dead holder no longer costs 30
seconds, and the wait is visible while it happens. The 30-second window survives only for a
**live** holder, where it is the correct behaviour (a second window owns the session).

# A19 — `--continue` waited thirty seconds on a lease it held itself (CLOSED at M212)

**Closed 2026-09-30.** The fix is the reorder plan 021 specified — resolve the resume target before
opening anything, open once, and let the resumed file be the primary — plus the resume block
borrowing the already-attached store instead of opening a second one. Verified with the **installed**
binary in a scratch workspace: two session files before `-c` and **two after** (no third file), and
**zero** lease-countdown lines on screen, with the TUI reaching its composer in about five seconds
of real work. Full record: [`docs/milestones/M212-resume-opens-once.md`](../docs/milestones/M212-resume-opens-once.md).

What stays open from this row, and is now the *only* thing left of it: **no automated regression
test** — nothing drives `build_runtime`, so a future reorder could reintroduce this with every gate
green. That is recorded in M212's Limits as the weakest part of the slice.

**Severity: high, and user-visible on every resume.** Found while trying to verify M210's `-c`
claim through the conhost probe, and it is the reason four attempts to capture that claim failed.

## The measurement

Scratch workspace, two TUI launches (so two session files exist), then **both `.lock` files
deleted** so that no holder existed on disk at all, then the installed binary launched with `-c` and
the console read back at 25 seconds:

```text
waiting for this session to be released — 21s elapsed, ~9s left (another process may sti…
waiting for this session to be released — 22s elapsed, ~8s left (another process may sti…
waiting for this session to be released — 24s elapsed, ~6s left (another process may sti…
```

The countdown is heading for the **full `STALE_AFTER` window (30 s)**, not the four-second
`DEAD_HOLDER_SETTLE` that a lock with no live holder is supposed to reach.

## What was ruled out, by measurement rather than argument

| Hypothesis | Test | Result |
|---|---|---|
| A killed launch left a stale lock, and the wait is for that | delete the lock files before `-c` | **wait unchanged** — ruled out |
| Another live epsilon still holds the lease | count processes right after a probe run, delete the locks, wait 6 s, count again | **0 processes alive, 0 locks reappear** — ruled out |

So nothing else holds it. The remaining and best-fitting explanation is a **self-conflict in the
resume path**: the process acquires a lease for the file it resolves (or for the fresh file M210
now names per launch), then opens the same session *again* for real — and the second acquire finds
its **own** live lease. A holder's heartbeat keeps the lock's mtime moving, so the dead-holder
settle never fires, and the process waits the whole window before the staleness rule lets it steal
from itself.

## Why this matters

`--continue` is the most-used resume path there is, and this predicts **thirty seconds of countdown
on every `-c` launch** with the TUI unstarted until it expires. It also means M210's `-c` claim is
unverified for a reason that is itself a defect, not a probe limitation.

## Located (2026-09-30) — it is two opens of one path, in that order

The sequence is in `crates/epsilon-cli/src/main.rs`, and it is unambiguous:

1. **`main.rs:1813`** opens the primary store at `workspace.join(session_file)` and
   **`:1877`** attaches it to the agent — so this process holds a lease. Since M210 that path is
   a *fresh file named for this launch*.
2. **Only then** does the resume step run: **`:1882-1888`** resolves the path
   (`Resume::Latest` → `latest_session_file(sessions_dir)`), and **`:1891`** opens that path as a
   **second** store.

Two things follow, and together they are the whole defect:

- **With `-c`, the newest session file in the directory can be the file this process just created**
  one step earlier — so the second open acquires a lease on a path whose lease *this same process*
  already holds. The holder is alive by definition (it is us), its heartbeat keeps the lock's mtime
  moving, so the dead-holder settle never fires and the wait runs the whole `STALE_AFTER` window
  before the staleness rule lets it steal from itself. That is the 21/22/24-second countdown, with
  no other process alive and no lock on disk at the moment of the launch.
- **Whether M210 caused it is a separate question, and the code says it probably did not.** The
  order is `open primary → attach → resolve resume`, and before M210 the primary path was the fixed
  `tui.jsonl` — which *is* what `latest_session_file` returns in a TUI-only workspace. So the
  self-conflict plausibly predates M210 by a long way. Do not present this as a regression without
  the `git log -S` that shows it; the mechanism is certain, the age is not.

## And the second half, which is worse than the wait (2026-09-30)

Reading the surrounding lines answered a question I had not asked, and it **withdraws a claim M210
makes**. Two facts, both from the code just read:

1. **The resume block never attaches the store it opens.** `:1891` opens `restored` from `rp`, uses it
   for migration, repair, the unknown-kind notice, `model_view()` and the resumed-model rule — and
   then the block ends at `:1956`. The agent's store is still the **primary** one attached at `:1877`.
2. **Therefore a resumed launch reads conversation N and writes to a different file.** Pre-M210 that
   difference was invisible: the primary was the fixed `tui.jsonl`, and `latest_session_file` in a
   TUI-only workspace returns exactly that file — so reads and writes coincided *by accident of the
   fixed name*.

With M210's per-launch name the accident stops being true, and the consequence is not subtle: **the
file this launch just created is the newest in the directory**, so `latest_session_file` returns
**the primary file itself**. `-c` would then resolve to the file this very process created a moment
ago — the one whose lease it already holds (which is the thirty-second wait) and which is **empty**
(which is why the wait ends in a session with nothing in it).

**So M210's claim that "`--continue` gets better without being touched" is withdrawn**, and it is
withdrawn here rather than quietly edited: the reasoning was sound for the *old* shape and inverted
for the new one. The milestone and the changelog both carry that sentence and both need correcting
when the fix lands.

**Measured, 2026-09-30 — the prediction holds.** Scratch workspace, two TUI launches, then one
`-c` launch, and the directory listed before and after:

```text
after two launches                     after one `-c` launch
2026-09-30T11-39-27-869Z_…8fd0.jsonl   2026-09-30T11-39-27-869Z_…8fd0.jsonl
2026-09-30T11-39-40-505Z_…a590.jsonl   2026-09-30T11-39-40-505Z_…a590.jsonl
                                       2026-09-30T11-39-52-792Z_…a580.jsonl   <- new, from the -c run
```

All three are **147 bytes** — a header and nothing else, which is itself the second measurement:
a launch writes its session header **at open**, before any turn. That is what makes the resolution
order fatal rather than merely untidy — by the time `-c` asks for `latest_session_file`, the file it
created microseconds earlier **is** the newest in the directory. So `-c` resolves to its own empty
file, opens it a second time (the thirty-second self-lease wait, measured separately), and restores
nothing from it.

**So all three halves are now measured, not inferred:** a resumed launch creates a file of its own,
that file is the newest at resolution time, and the wait on it runs the full window. M210's
withdrawn claim is withdrawn for a reason that is now evidence rather than a reading of the code
order.

The fix does not change: [`docs/plans/021-a19-resume-opens-once.md`](../docs/plans/021-a19-resume-opens-once.md).
**This is a prediction from the code order, not a measurement.** The measurement is one launch and a
directory listing, and it is the first thing the fix round should do: run `-c` in a scratch
workspace that already holds two sessions and check (a) whether a third file appears, (b) whether it
is empty, and (c) whether the countdown is longer than the four-second settle. If the third file is
the one being written while the second is the one being read, the resume is writing the new
conversation beside the old one instead of into it.
## The fix shape, in the order worth trying

Execution is specified line by line in [docs/plans/021-a19-resume-opens-once.md](../docs/plans/021-a19-resume-opens-once.md) — the two edits, the three borrow details that would otherwise cost an hour, the verification commands, and the three things not to do.

1. **Resolve the resume path before opening anything**, and when it equals the primary path, open
   **once** — the resumed store *is* the primary. This removes the double open rather than working
   around it, and it is the only version that cannot leave a window where two leases exist.
2. If the reorder is too invasive for one slice: skip the primary open entirely when
   `Resume::Latest`/`Path` resolves to the same path, and construct the agent's store from the
   resumed one.
3. Do **not** "fix" it by releasing the primary lease just before the second open. That works for
   the same-path case and silently permits a genuine second writer whenever the paths differ —
   which is the corruption the lease exists to prevent.

The verification is the check A19 interrupted: a capture past the wait that shows the resumed
transcript, plus a launch timing that does not include a thirty-second countdown.
## Not yet established — the two things a fix needs

- **Which file it waits on.** The evidence is a countdown with no holder, not a `Leased { holder }`
  error naming a path; a run with `-e`-style logging or a breakpoint-free `eprintln!` of the two
  acquire calls would name it in one launch.
- **Whether M210 introduced it or merely inherits it.** The pre-M210 TUI also resolved
  `latest_session_file` for `-c` while naming its own fixed file, so the same double-open plausibly
  existed then — which would make this an old defect that M210's per-launch naming does not fix
  rather than one it caused. **Do not assume either way; the fix is the same and the history is one
  `git log -S` away.**

The fix is almost certainly to drop the first lease before the real open (or to resolve the file
*without* acquiring), and the verification is the check this row interrupted: a capture past the
wait that shows the resumed transcript.
**A18 measured again in the real artifact (2026-09-30), and it is not fully closed.** Driving the
installed binary through the conhost probe: two launches, then `epsilon -c`. The resume run went
straight into the lease wait — because the two launches had been *killed* by the probe, and a killed
process leaves its `.lock` behind, so the newest session file was also the locked one. The countdown
was drawn on screen (M207's premise, confirmed): `waiting for this session to be released — 1s
elapsed, ~29s left`, then 2s, then 3s.

Two facts follow, and the second is new. First, the wait **is** legible — A18's fix works. Second, the wait for a *dead* holder is bounded by `DEAD_HOLDER_SETTLE` — and the
constants, read rather than remembered, are `HEARTBEAT = 2s`, `DEAD_HOLDER_SETTLE = 4s`,
`STALE_AFTER = 30s`, `WAIT_POLL = 100ms` (`lease.rs:88/97/102/129/134`). So a workspace whose TUI
was just killed starts after about **four seconds**, with the countdown drawn throughout — which is
what the probe's 1s/2s/3s frames were counting toward.

**Do not "optimize" that four seconds away, and here is why, so nobody has to re-derive it.**
`DEAD_HOLDER_SETTLE` is deliberately 2× `HEARTBEAT`: a *live* holder is visible as an mtime that
keeps moving, and the margin exists so that a live process which stalls for one heartbeat — a
scheduler hiccup, a busy disk, a debugger — is not mistaken for a dead one. The steal is the only
path by which two writers can share a session file, so shortening the settle toward the heartbeat
trades a four-second wait for the corruption the lease exists to prevent. The two exact
alternatives are both already priced: a pid probe needs `unsafe` (forbidden here) and is wrong
under pid reuse, and OS advisory locks (`fs4`/flock) would be exact but need a dependency and an
ADR — a trade the lease module's own docs recorded when the mtime-steal was chosen.

**A consequence of M210 that nobody has designed for:** with one file per launch, each killed
session leaves its own `.lock` beside its own `.jsonl`, so a sessions directory grows two entries
per launch, half of them stale. Discovery filters `*.jsonl` and ignores them, so nothing breaks —
but nothing removes them either, and they are what makes the *next* launch wait.
**A16 is decided, and the decision goes against this harness.** The row asked whether `tui.jsonl`
is one rolling session per frontend or one per launch, and noted that the code ships both answers
(`reset_leaf` exists for the second and is called only from tests). **The reference settles it**:
pi names one file per session — `repo.ts:42-45` builds
`sessions/--<cwd>--/<ISO-timestamp>_<id>.jsonl` — and resolves that path on *create*
(`resolveNewSessionPath`, `repo.ts:77/171/273`). Which is why `--continue` ("newest session for this
workspace", `latest_session_file_in(dir, cwd)` here) is a meaningful phrase in pi and an empty one
in a harness that writes one fixed file per frontend.

What the current shape actually does, stated plainly: three TUI launches append three conversations
to **one lane**, so `-c` hands the model all three as a single context with no boundary; the header
is written once, so its `id` and `cwd` name the first conversation while every later one is a
stowaway in it; `/fork`, `export` and the sessions browser all assume file ↔ conversation; and the
file grows without bound, putting a full fold in front of every `goal()` and label read. M205's
"this session already holds N entries" notice was the *symptom* of this shape, not a feature of it.

Execution, with the per-cwd directory in the same slice because `-c` across workspaces depends on
it: [`docs/plans/020-one-session-per-file.md`](../docs/plans/020-one-session-per-file.md). The plan
carries the costs too (a file per launch with no pruner; no migration of existing files), and its
step 3 is the disposition of `reset_leaf` itself — **delete it unless the new shape finds a caller**,
since an uncalled primitive is how this row started.

**A11 is stale on both halves, and that is the second row this sweep has found written from prose rather than from code.** The row claims "delegated-subagent and compaction usage has a reader and no writer". Re-derived:

| Half | Writer | Reader | Tests that fail if the writer goes |
|---|---|---|---|
| compaction | `core/src/lib.rs:2775` — the compaction entry carries `usage` **and** `model_id` (M145/F22, with the reasoning inline: one entry, not two, because `pop_last_message` would otherwise separate a summary from its bill) | the TUI's cost rows | `a_recorded_compaction_bill_reaches_the_sessions_token_rows`, `a_summarizer_billed_to_another_model_keeps_its_own_pricing_row`, `a_compaction_that_billed_nothing_adds_nothing` |
| delegated subagents | the `task` tool's result carries `usage` + the child's model | `record_tool_usage`, called on the **live** path (`state.rs:1002`) *and* the resume/hydration path (`state.rs:1123`) — the second exists because the first was once the only one | `a_subagents_tokens_reach_the_session_total`, `a_subagents_tokens_survive_a_resume`, `a_tool_usage_without_a_model_is_billed_to_an_unpriced_row` |

So the money the row was worried about is counted, on both paths, with tests on each. **A7 and A11 are two instances of one method failure**: a ledger row is a claim about code, and both were written from a *neighbouring* fact — A7 from the constant `STALE_AFTER = 30s` rather than from what `acquire` does with it, A11 from the fields existing rather than from who fills them. The countermeasure is the one the top of this file already states and that has now paid for itself twice: **re-derive before acting, and name the file:line that would have to change.**
**A14/A15 are documented; the decision is still the owner's.** Both rows end in the same place —
one paragraph in `docs/user/permissions.md` — and it now exists: the two ways to get a file in
front of the model are tabulated (``@`` refuses by name, `read` does not), the refusal list is
stated and verified against `references.rs` (ten names, checked on the spelled *and* the resolved
path), the silence-versus-visibility distinction that makes the asymmetry purposeful is written
down, and the one thing a reader most needs to know is stated plainly: **the session file is not
scrubbed**, so a `cat .env` through `bash` stays in it in cleartext. What remains is not
documentation but a choice — whether the durable store should scrub, with the trade named (a
scrubber hides evidence and a wrong pattern either misses a secret or mangles content) — and that
choice is the owner's, not an agent's. Until it is made, the guide says to treat the session file
as you would treat your shell history.
**A12's decision is made, execution planned.** [`docs/plans/019-one-config-reader.md`](../docs/plans/019-one-config-reader.md)
records the measured finding — `global_dir()` written byte-for-byte twice (`core/config.rs:37`,
`llm/auth.rs`) and one global `config.toml` parsed by three readers — and the decision the
architecture law forces: `epsilon-llm` may not depend on `epsilon-core`, contracts is
types-and-traits-only, and a new crate needs an ADR, so the reader stays above the seam and parsed
values cross as parameters (the pattern `register_builtins_with_skills` and `ToolContext::task_host`
already use). The plan is four independently committable steps with per-step evidence, the first of
which deletes the duplicate path rule.
**The remnant, which is real and is not fixed — A18, added here:** the notice goes to the console
and the alternate screen then erases it, so a launch that waited 25 s and found the lock released
starts with no record of why it paused. Closing it needs the wait's duration to reach
`TuiConfig::startup_notices`, which is built before the store is opened — plumbing rather than a
sentence.

---

# Reconciliation, 2026-09-29 (M202)

One defect closed (found by driving the installed binary, not by a test), and two rows added
that M202's investigation exposed. Milestone:
[`docs/milestones/M202-older-sessions.md`](../docs/milestones/M202-older-sessions.md).

| # | Row | State |
|---|---|---|
| — | **A session written before M194 could not be opened at all.** `Usage::cache_reported` was added without a serde default, and `Usage` is inside every assistant message, so the whole file was refused and `epsilon` would not start in a workspace that had one. | **Fixed** — `#[serde(default)]`, the refusal now quotes the decoder (`missing field \`cache_reported\``) and names the file. Negative control in [`audit/evidence/m202-negative-control.md`](evidence/m202-negative-control.md). |
| **A16** | **`SessionStore::reset_leaf` has no production caller** — seventh instance of the repository's own "faithful port, no production caller" class. A plain `epsilon` launch opens the existing `tui.jsonl`, appends to it as a child of the previous launch's tail, and starts with an empty in-memory history; `reset_leaf` is documented as *the* way to make the next append a new root and is called only from tests. | **Open — a decision, not a patch.** The question is what `tui.jsonl` *is*: one rolling session per frontend (today's behaviour, and what `-c` implies) or one session per launch (what `reset_leaf` is for). Neither is wrong; the code has both answers and ships the first. |
| **A17** | **A plain launch never says that a session exists.** `--resume` seeds history and a plain launch does not, so a user who quits and relaunches sees the first-run screen and no hint that `-c` (or `/resume`) would bring the conversation back. The header reports `ctx 4%` beside it — the system-prompt-and-tools baseline, **not** hidden history, verified at `main.rs:1924` — which makes an empty screen look like a contradiction. | **Open — small.** The welcome panel has room for one line, and the fact it needs (a prior session exists, here is the command) is known to the CLI before the TUI starts. |

---

# Reconciliation, 2026-09-29 (M201)

Seven of the thirteen rows open at M199 were closed by M201, plus a defect that no row
named. Every one is the same class — **a screen that is wrong in a way no state-level test
can see** — and every one was verified by rendering a frame and reading it, not by reading
the module. Frames: `audit/evidence/frames/m199-baseline/` (before) against
`audit/evidence/frames/m201/` (after). The milestone doc is
[`docs/milestones/M201-frame-truth.md`](../docs/milestones/M201-frame-truth.md).

| # | Closed by | Verified by |
|---|---|---|
| A1 | `wrap::wrap_line` counted the separator it was about to flush after deciding the word fit, so a row could be one column over budget and the draw path clipped its last cell. | `no_row_is_wider_than_the_width_it_was_given` (widths 1..=len) and `no_row_is_wider_than_the_width_across_styled_spans`; the 60- and 80-column frames regained "then" and the full stop. |
| A2 | `header::metric_spans` budgets with `layout::room_beside` and drops whole gauges; the model and its `offline` qualifier are pinned by a field on `MetricPart`, not by role. | `a_narrow_terminal_drops_whole_gauges_rather_than_cutting_one`, `a_narrow_terminal_keeps_the_model_and_the_context_gauge` and `an_offline_session_keeps_the_model_and_its_qualifier_when_the_row_narrows`. The frame at 60 reads `idle      ?/1.0M · ↑0 ↓0 · Faux (offline scripted) · offline`. |
| A3 | `footer::draw` budgets the hints and emits `key action`, `key`, or nothing; the gap is the one `side_by_side` reserves. | `no_footer_row_ever_prints_half_a_hint_or_touches_the_location`, swept at every width 20–120 against the legal renderings. |
| A4 | The welcome panel stopped printing the model and the context; the header owns them. | `the_welcome_screen_does_not_restate_the_header`, and the frame. |
| A5 | One left edge, one instruction in a body weight, affordances nothing else states. | `every_welcome_row_shares_one_left_edge`, and the frame. |
| A6 | The composer draws `glyphs.prompt`, not `glyphs.bullet`. | `the_composer_prompt_is_not_the_notice_bullet`, run against both repertoires. |
| A13 | The legend moved to `ui::metrics::LEGEND`, beside the formatters; `/help` prints it. | `the_legend_decodes_every_token_the_header_can_print` generates each placeholder from one fixture and asserts the legend quotes it. Its first run failed: the legend said `ctx 42%/200k*` and the header prints `ctx 42%*/200k`. |

**Not a row, found while fixing A2.** The first correction dropped leading fragments and
kept `R0 W0 CH n/a` while losing `ctx ?/1.0M`; the second pinned the model by
`MetricRole::Model`, which is `Warn` in an offline session, so the pin stopped matching in
exactly the row long enough to need it. Both were invisible to every assertion in
`header.rs` and visible in one rendered frame. That is the M198 rule again, and the reason
the frames are committed as evidence rather than described.

**Still open, unchanged:** A7 (a killed TUI blocks the workspace for 30 s), A8 (`epsilon-tools`
depends on `epsilon-lsp`), A9 (no `.gitattributes`), A10 (eight identical `#[allow]`s), A11
(delegated usage has a reader and no writer), A12 (configuration is read in three crates),
A14 (no secret scrubbing in the durable store), A15 (the `read`/`@` asymmetry is undeclared).

---

# Reconciliation, 2026-09-29 (M199)

Every row below was **re-derived from the code at `c975d51`**, not inherited. The
method, because the method is the part that has gone wrong before:

* A row is marked **Fixed** only on a check that would fail if it were open. Where a
  measurement was possible it is quoted.
* **`unsafe` outside `epsilon-safety` is 3 hits and all three are a comment or a test
  string.** The repository's one safety law is genuinely true. (`grep` counted
  hyphenated crate names in source, where Rust uses underscores, and briefly showed
  every `epsilon-server` dependency as unused — D2 is a retraction, and the
  identifier was the error, not the code.)
* Two greps that would have produced false rows are recorded here as **method notes**,
  not as findings: a grep that returns nothing proves nothing if the identifier is
  wrong, and a count that includes `#[cfg(test)]` proves nothing either.

## Verified open, ranked by what it costs the user

| # | Defect | Cost | Evidence that would close it |
|---|---|---|---|
| **A1** | **The draw path drops a character at the wrap boundary.** A 60-column frame renders "…in the workspace and **the**" and drops the `n` of "then". | The transcript is the conversation: a user re-reads their own message, and the model reads the same rows. Silent text corruption, no error. | Measured: `wrap_line` is **content-intact at widths 56–61** (a scratch probe over the exact string), so the wrapper is innocent and the *draw* path wraps one column wider than it draws. `app/view.rs:432` and `app/panels.rs:290` both compute `width - GUTTER`; the drawn row 1 is the 59-wide wrap clipped to 58. Fix by making the two agree, and pin it with a frame test at 60 columns. |
| **A2** | **The header loses its `↑` at 60 columns**, printing `idle 0 ↓0 · R0 W0 CH n/a …`. | A bare `0` where a labelled pair was. A *partial* badge is worse than a missing one, because a wrong number is not noticed the way an absent one is. | `idle 0 ↓0` vs `idle … ↑0 ↓0` at 100 — in `audit/evidence/frames/m199-baseline/*.60x20.txt`. The model picker must drop whole segments, never the marker that gives a number its meaning. |
| **A3** | **The working directory is appended to the key hints with no gap.** `… pageup scroll back · pageup …\Epsilon\crates\epsilon-tui`. | The reader cannot tell where the key list ends and the path begins, so a truncated hint run reads as one more hint. | `ui/footer.rs:326-331`: `side_by_side(left, right, …, Priority::Right, whole_span)` reserves no minimum gap, so the truncation ellipsis abuts the right side. Reserve ≥1 column and pin it at 60. |
| **A4** | **The empty state states what the header already states.** `model Faux (offline scripted)` and `?/1.0M` appear in the header **and** in the body, two rows apart, on a screen with 20 blank rows under them. | The first thing a new user sees spends its whole budget repeating itself. | `audit/evidence/frames/m199-baseline/brand-new.100x30.txt`. Decide which surface owns the model and context, and delete the other. |
| **A5** | **The empty state's alignment has three left edges and no hierarchy.** `E P S I L O N` at col 1, `a coding agent…` at col 1, `model`/`context` at col 3, `ready — …` at col 3 in the *same style as the data above it*. | An instruction rendered as a datum; nothing on the screen says which line is the call to action. | Same frame. An instruction wants its own style, and the block wants one left edge. |
| **A6** | **The composer's placeholder is drawn as a notice.** `• ask anything` carries the same `•` gutter glyph as a system notice. | The one glyph that means "the system said this" marks a placeholder. This is the M198 family — a glyph carrying the wrong meaning — and it is on screen at every width, in every state. | `brand-new.100x30.txt` vs the real notices in `streaming.100x30.txt`. |
| **A7** | **D20 — a killed TUI blocks the workspace for 30 s.** | A relaunch straight after End Task refuses to start. | `session/src/lease.rs:88` `STALE_AFTER = 30s`, still live. The CLI shows a countdown; **the TUI shows nothing** (the alternate screen erases the notice). |
| **A8** | **D3 — `epsilon-tools` depends on `epsilon-lsp` directly.** | A documented decoupling that buys nothing, which is worse than not documenting it. | `crates/epsilon-tools/Cargo.toml:13`. The trait exists so the dependency is not needed. |
| **A9** | **D5 — no `.gitattributes`; the tree is genuinely mixed CRLF/LF.** | Permanent diff noise, host-dependent formatting. Every `git add` in this session printed an LF→CRLF warning. | `ls .gitattributes` → absent. |
| **A10** | **D9 — eight identical `#[allow(unreachable_patterns)]` in one file.** | Unknown, and that is the problem. | `app/commands.rs`, 8 occurrences. |
| **A11** | **D17 / F22 — delegated-subagent and compaction usage has a reader and no writer.** `usage_accounts()` and `add_usage_accounts` exist and are read by `app/session.rs:227`; nothing constructs the entry. | A summarization is a real provider call whose cost nothing counts. It is about money, it is silent, and it is unbounded. | `core/tests/compaction_scenarios.rs:837` *asserts the writer does not exist* — the suite pins the absence. Shipping the writer is a design decision (which model is a summarization billed to?), not a patch. |
| **A12** | **D14 — configuration is read in three crates** (`core/config.rs`, `llm/custom.rs`, `llm/oauth_login.rs` + three in the CLI). | Two config surfaces with no single owner, rising as config grows. | Name the owner. |
| **A13** | **The header has no legend for `?`, `CH n/a`, or the trailing segment.** | The owner asked for visibility into the harness's own operation, and the three least meaningful tokens on screen are the three nobody can read. | Carried from `next-hardening.md` and still open. Two lines in `/help` close it. |

## Security and privacy — verified, and two of them are judgements rather than defects

Neither is a bug. Both are recorded because **an undocumented judgement is an
undocumented judgement**, and this repository's own history is a list of features
that were correct-by-accident and nobody wrote down.

| # | Finding | What was checked | The judgement to record |
|---|---|---|---|
| **A14** | **No pattern-based secret scrubbing anywhere.** The only `redact` in the tree is extension-driven (`runtime.rs:1983` — an extension rewriting content it chose to); the only masking is `mask_key`, which hides a key on screen during `/login` (`main.rs:2753`). Nothing scrubs tool output before it is appended to the session JSONL. | `grep -rl redact crates/*/src` → 2 files, neither the TUI nor diagnostics. `grep` for key/secret/token patterns joined to a scrub function → nothing. | **Is that right?** Transparency is defensible — the user asked for the file, and scrubbing hides evidence — but a `cat .env` lands in a durable on-disk transcript in cleartext. Whichever way it goes, the policy belongs in `docs/user/permissions.md`, and the answer is probably "scrub the durable store, not the live view". **A decision, not a patch.** |
| **A15** | **`ReadTool` applies no sensitive-name check, and that is not written down.** `is_sensitive` has exactly one caller: the `@`-reference expander. So `@.env` is refused while `read .env` is not. | `grep -rn is_sensitive` → `references.rs` only (4 sites). | The asymmetry looks deliberate — `@` attaches content *silently* into a prompt, `read` returns visibly in the transcript — but that reasoning appears in no document. It also means A14's answer depends on it. **Record it in `docs/user/permissions.md`.** |
| — | **Command spawns are covered by a mechanical guard, and it is a good one.** | `no_production_spawn_leaves_stdin_unset` walks every `Command::new` under `crates/`. It caught a **new** omission the first time it ran against M199's `mklink` helper. | Working as designed. Worth noting it now covers a `#[doc(hidden)] pub mod` in a library crate, which is the case a per-site test would have missed. |

## Verified fixed at `c975d51`

| # | Claim | How it was verified, not assumed |
|---|---|---|
| D1 / D21 | `epsilon-lsp` did not inherit the lint law | `crates/epsilon-lsp/Cargo.toml` has `[lints] workspace = true`. `scripts/check-lint-tables.py` reports 11 crates inheriting, 1 hand-copied and matching. |
| D6 / D19 | A panic leaves the terminal unusable | `app.rs:1173` `impl Drop for TerminalGuard` and `app.rs:1206` `std::panic::set_hook`, with the restore factored out so it is testable without `set_hook`. **D6 and D19 are the same defect listed twice** with different severities — see the note under method. |
| D7 / D18 | The TUI deep-clones the transcript every frame | **The repo's own tripwire, run:** `11.978 µs at 20 messages -> 11.899 µs at 200 messages` — 1.0x cost for 10x the transcript. It was 12.9x. `context_mirror_cost_does_not_track_transcript_length` passes for the right reason, and `context_mirror_stays_inside_the_frame_budget` passes beside it. |
| D13 | `docs/README.md` renders as body text | The file opens with `# Documentation` and a `| File | What lives there | Read it when |` header. |
| D15 | The TUI test never types while a turn is in flight | `crates/epsilon-cli/tests/terminal.rs:429` `typing_still_works_while_a_turn_is_streaming`. |
| D16 | Four root scratch files | None present. |
| — | `unsafe_code = "forbid"` is true workspace-wide | 3 hits outside `epsilon-safety`, all a doc comment (`contracts/message.rs:99`, `core/lib.rs:1822`) or a test string. |

## Method notes — the ledger's own defects

* **D6 and D19 are one defect.** D7 and D18 are another. Four rows, two findings,
  two different severities for each. A ledger that ranks duplicates cannot be read
  as a ranking.
* **D2 is retracted, correctly, in place.** It claimed `epsilon-server` linked two
  crates it never calls; the only uses are inside its `mod tests`. It is *not* — the
  retraction is in the row. That is the ledger working as intended and is the reason
  the reconciliation above re-derives rather than trusts.

---

## D1 — `epsilon-lsp` does not inherit `unsafe_code = "forbid"`

**Severity: high. Cost of leaving it: a silent hole in the repository's one
stated safety law.**

`crates/epsilon-lsp/Cargo.toml:21-22` declares

```toml
[lints.rust]
missing_docs = "warn"
```

with no `workspace = true`. **A package `[lints]` table replaces the workspace
one; it does not merge with it.** So this crate compiles with neither
`unsafe_code = "forbid"` nor `clippy::all = "warn"`. `crates/epsilon-safety/Cargo.toml:28-38`
shows the correct pattern for a deliberate override.

There is no `unsafe` in that crate today — verified by reading all seven source
files. The defect is that nothing would stop the first one. `AGENTS.md` states
`unsafe_code = "forbid"` as a repository-wide law with per-crate exceptions
requiring justification; this crate has an exception that nobody declared.

**Fix:** add `workspace = true` to the crate's `[lints.rust]`, keeping
`missing_docs = "warn"` as the local override. One line, and it makes the law
true again.

---

## D2 — `epsilon-server` links two crates it never calls

**Severity: medium. Cost: the shipping server binary carries a provider stack
and the entire tool set for nothing, and the architecture document's
responsibility table is wrong because of it.**

`crates/epsilon-server/Cargo.toml:12,14` declare `epsilon-llm` and
`epsilon-tools` in `[dependencies]`. The only uses are at
`crates/epsilon-server/src/lib.rs:461` and `:863-864`, both inside the
`mod tests` block that opens at `:457`.

**Fix:** move both to `[dev-dependencies]`. The architecture table stays true and
the release binary stops linking ~27,000 lines it cannot reach.

> **RETRACTED 2026-09-28 — this claim is wrong.** I checked whether those two
> uses are the only ones, and they are not:
>
> ```
> $ grep -rn "epsilon_llm\|epsilon_tools" crates/epsilon-server/src/
>   lib.rs:461:  use epsilon_llm::{FauxProvider, FauxStep, PanicGuard};
>   lib.rs:863:  let registry = Arc::new(epsilon_tools::builtin_registry(
> ```
>
> The claim was that every use sits inside `mod tests`, which opens at `:457`.
> Line 461 is inside it; **line 863 is 400 lines further down and is not.** The
> server builds a real tool registry at runtime. Both crates are genuinely used
> by production code, so the proposed fix — moving them to `[dev-dependencies]` —
> would **break the build**.
>
> This is recorded rather than deleted because the failure mode is instructive:
> a claim with a file:line citation reads as verified, and this one was not. Every
> other row in this ledger is cheap to re-run the same way before acting on it.

---

## D3 — `epsilon-tools` depends on `epsilon-lsp` directly, contradicting the reason the trait exists

**Severity: medium. Cost: a documented decoupling that buys nothing, which is
worse than not having it — a reader trusts the comment and reasons about an edge
that exists anyway.**

`crates/epsilon-contracts/src/diagnostics.rs:3-5` says the `DiagnosticsProvider`
trait exists "so `epsilon-tools` depends on a promise … instead of on
`epsilon-lsp`". But `crates/epsilon-tools/src/lib.rs:119,125` import
`epsilon_lsp::ServiceConfig` and `epsilon_lsp::DiagnosticsService` directly.

Either the trait should be the whole story, or the comment should be corrected to
say what the seam actually does. Keeping a comment that describes an
architecture the code does not have is a defect of the same class as a stale
status table: it is a claim that survives after it stops being true.

**Fix:** decide, then make the comment true. The lean is to move the two
`epsilon_lsp` types behind `epsilon-contracts` (they are configuration and a
service handle, both nameable there) and delete the direct edge.

---

## D4 — Five false claims and four stale ones in the architecture guide

**Severity: medium. Cost: every new contributor is routed here by
`docs/README.md:61-64`, and four of the false claims are about code paths that
are the most confusing ones in the repository.**

`docs/design/epsilon-architecture.md` gets the dependency table exactly right
(`:28-41`, verified against all twelve manifests). Its execution-path section is
not reliable:

| Line | Claim | Reality |
|---|---|---|
| `:54` | `core/src/lib.rs:900–934` admits a run | `lib.rs:901-904` is `subscribe()`, `:906-911` `steer()`. Off by ~380 lines. |
| `:54` | `:1037–1261` alternates provider turns and tool batches | `lib.rs:1038-1056` is `persist()`. |
| `:56` | "frontend acknowledgments do not yet share atomic admission" | Closed. `epsilon-server/src/lib.rs:326-329` does `run_depth.fetch_add` + `CONFLICT`; `rpc_mode.rs:100` has `admit_prompt`. |
| `:63` | "RPC currently skips lagged broadcasts" | Closed. `rpc_mode.rs:376` handles `Lagged`; `:227` emits a `lagged` notice; tested at `:497`. |
| `:149` | "Admission and dropped-event recovery remain open" | Both closed, same evidence. |
| `:148` | "`app.rs` orchestrates editor/picker/markdown/state modules" | `app.rs` is 1,314 lines; the loop is `app::event::drive`; ten submodules now live under `app/`. |
| `:150` | "RPC: prompt, steer, followup, approvals, approve, ping" — six ops | Nine. Also `abort` (`:334`), `get_state` (`:338`), `set_thinking_level` (`:344`). |
| `:60-61` | `providers` and `fallback` are listed as if unconditional | Both are `#[cfg(feature = "real")]` (`llm/src/lib.rs:21-22`, `:15-16`); `retry_stream` is a private `mod` (`:25`). An offline build has neither. |
| `:24` | "per-crate `tests/` directories" | True as a convention; three of twelve crates have none. |

There is also a **second architecture file**, `docs/design/epsilon-architecture.draft.md`,
358 lines, referenced by nothing. It is *more* accurate than the indexed one — it
independently confirms D1, D2 and a false doc comment in `epsilon-contracts` — but
it is now itself partly stale (it describes `app.rs` as 4,238 lines; it is 1,314).
Shipping an accurate-but-unindexed draft beside an inaccurate-but-indexed guide
is the worst of the three available options.

**Fix:** re-derive the execution-path section from the code, fold in what the
draft got right, delete the draft, and re-point every `path:line` anchor. Then add
a gate that checks the anchors resolve, so this cannot rot again.

---

## D5 — No `.gitattributes`; the tree is genuinely mixed CRLF/LF

**Severity: medium. Cost: permanent diff noise, and host-dependent formatting.**

There is no `.gitattributes` anywhere in the repository. Line endings split along
refactor history — older files CRLF, newer LF — and are mixed *within*
directories: in `crates/epsilon-tui/src/app/`, `view.rs` is CRLF and
`commands.rs` is LF; in `crates/epsilon-core/tests/`, `guard_scenarios.rs` is
CRLF and `verification_scenarios.rs` is LF. Every future diff against an older
file carries whole-file line-ending churn, and `cargo fmt --check` behaviour
becomes a property of the host rather than the repository.

**Fix:** add `.gitattributes` with `* text=auto eol=lf`, normalise once, and add
`*.bat text eol=crlf` for anything that genuinely needs it (nothing does today).

---

## D6 — A panic anywhere in the TUI leaves the console unusable

**Severity: high, and it is indistinguishable from the reported freeze.**

`crates/epsilon-tui/src/app.rs:909-924` enters the alternate screen and raw mode,
calls `drive`, and on the *normal* return path calls `ratatui::try_restore()` at
`:922`. There is **no panic hook and no `Drop` guard**. `color-eyre` is not in the
dependency tree at all (verified against `Cargo.lock`).

A panic in the event loop, a key handler, or a widget therefore leaves the
console in raw mode, in the alternate screen, with the cursor hidden, and prints
the panic message *into* the alternate screen. The process is still running. From
the user's side that is a frozen TUI, and it is indistinguishable from a hang.

This violates a non-negotiable contract the repository's own skill states:
"Do not hand-roll raw-mode + alt-screen + panic restore unless the project already
does it" (`.agents/skills/ratatui-tui/SKILL.md:61`). The project hand-rolls it and
does not restore on panic. The skill is right and the code is wrong.

**Fix:** a `TerminalGuard` holding the terminal whose `Drop` restores it, plus a
`std::panic::set_hook` that restores first and then prints. Small, and it converts
an unexplainable failure into a readable one.

---

## D7 — The TUI deep-clones the entire transcript on every drawn frame

**Severity: high. Cost: the UI thread does O(session) work at up to 30 Hz, and it
gets worse exactly when the user is doing something long.**

`crates/epsilon-tui/src/app/event.rs:169` calls `mirror_context_size` on the frame
boundary. That calls
`Agent::estimated_context_tokens` (`crates/epsilon-core/src/lib.rs:1934-1937`),
which calls `self.history()` — and `history()` is
`self.history.lock().map(|h| h.clone()).unwrap_or_default()`
(`lib.rs:1114-1116`). **A full deep clone of every message in the conversation,
per frame**, purely to produce one integer for a progress gauge.

The comment above it (`crates/epsilon-tui/src/app/view.rs:296-299`) claims the
call is "cheap enough for that position". It is not: it allocates and copies the
whole transcript, thirty times a second, while the run that is growing the
transcript is still going.

The fix is not an optimisation, it is a correction: `anchored_tokens`
(`compact.rs:46-53`) needs the history only to *read* it. Holding the lock and
borrowing removes the clone entirely and leaves the walk.

**Fix:** borrow under the lock instead of cloning. Three lines in
`epsilon-core/src/lib.rs:1934-1937`.

---

## D8 — The render cache contradicts its own stated cost model, and the
streaming tail is wrapped twice per frame

**Severity: medium-high.**

`crates/epsilon-tui/src/render_cache.rs:310-313` documents steady-state reconcile
as "O(1), independent of transcript length". It is not.
`render_cache.rs:350-357` does a prefix scan and `:373-374` builds a `HashSet`
over every reconciled id and retains against it — **per frame, over every line of
the transcript**.

Separately, the in-flight answer is wrapped in full, twice, on every frame:
`app/view.rs:549-562` counts its rows and `app/panels.rs:264-292` builds the same
rows to draw them. Both go through `wrap.rs:120-143`, whose `tokenize` allocates a
`String` **per whitespace-delimited token**, and `push_text`
(`wrap.rs:91-105`) allocates **per span**. A 40 KB answer is roughly 28,000
allocations per frame; at 30 fps that is on the order of 10⁶ allocations a second
on the UI thread.

**Fix:** (a) only build the `HashSet` when something was actually removed;
(b) compute the tail's row count once per frame and share it with the draw, or
count without materialising; (c) make `tokenize` return borrowed slices — it
allocates a `String` per token to immediately re-borrow it.

---

## D9 — Eight identical `#[allow(unreachable_patterns)]` in one file

**Severity: unknown, and that is the problem. Read all eight before editing
anything else in that file.**

`crates/epsilon-tui/src/app/commands.rs:288, 314, 408, 553, 652, 764, 1001, 1312`.

Each suppression is either redundant (the arm should be deleted) or, worse, a
**live arm is shadowed** — which is a behaviour bug the lint is pointing at. A
slash command whose handler is unreachable is a command that silently does
nothing, and this repository's own history (M162, "slash commands by subject, and
a tripwire for misrouting") is exactly that class of defect.

**Fix:** read all eight, delete or un-shadow, remove the suppressions.

---

## D10 — Seven stale `#[allow]`s and the dead items they guard

**Severity: low individually, medium as a pattern.** Dead code that carries a
justification comment is worse than dead code, because the next reader believes
the comment.

| Suppression | What it guards | Reality |
|---|---|---|
| `epsilon-tui/src/picker.rs:595` | `ALIAS_HEADS` | **Used** at `picker.rs:667`. Stale suppression — remove. |
| `epsilon-llm/src/http.rs:81` | `status_error` | No caller. Comment says "wired into the retry-classified error path"; it is not. A *live* `status_error` exists at `epsilon-tools/src/webfetch.rs:72`. The comment is false. |
| `epsilon-llm/src/retry_stream.rs:233` | `VecStream::single` | No caller. |
| `epsilon-safety/src/sandbox/jobobject.rs:187` | `terminate()` | "Test/diagnostic hook"; the test module at `:347` does not use it. |
| `epsilon-mcp/src/lib.rs:136` | `invalidate_health` | `#[cfg(test)]` *and* dead — not even the test uses it. |
| `epsilon-tools/src/files.rs:712-714` | `_path_witness` | Lint scaffolding. Its comment ("until `Path` gains a direct use in later milestones") is stale — that milestone is long past. |
| `epsilon-tui/src/app/view.rs:32` | `App::theme()` | Speculative: "a future integration test, a screenshot harness". |

**Bigger dead blocks**, both already noted in the earlier ledger and neither
addressed:

- Four `SessionStore` tree methods with no production caller —
  `epsilon-session/src/store.rs:1329 navigate_to`, `:1352 reset_leaf`,
  `:1372 branch_with_summary`, `:1486 fork_branch_at`, plus the private
  `branch_to` at `:1421`. About 250 lines that only tests call.
  **Caution:** these are pi's session-tree feature
  (`session-manager.ts:1374-1395`). Deleting them removes parity. They are
  "unreachable from the product", not "not implemented" — see
  [02](02-port-fidelity.md) before touching them.
- Three `SessionLease` constructors — `epsilon-session/src/lease.rs:103 acquire`,
  `:111 acquire_now`, `:117 acquire_with` — where only `acquire_with_progress`
  (`:141`) is reachable. `docs/milestones/M136-the-lease-waits-instead-of-refusing.md:120`
  claims "It now calls `acquire_now`". **That claim is false of the tree.**

**Fix:** delete the suppressions and their dead items, except the session-tree
methods, which need a product decision rather than a cleanup.

---

## D11 — Two false doc comments

**Severity: low. Cost: a reader who trusts them is wrong.**

- `crates/epsilon-tui/src/lib.rs:27-29` — two stacked `///` lines before
  `pub mod search;`. The first, *"What a tool card says, per tool. See the module
  docs."*, describes `renderers` (declared at `:25`). Orphaned when `renderers`
  was split out. It is attached to the wrong module.
- `crates/epsilon-contracts/src/lib.rs:4-5` — *"It contains zero business
  logic"*. It contains `stream::StreamFolder` (stateful folding,
  `stream.rs:66-214`), `session_entry::SessionLine::parse` validation, and
  `agent_event::TurnEndReason::merge_sticky` (`agent_event.rs:32`). `AGENTS.md`
  records this drift as a known exception; the crate's own doc comment still
  denies it. The same line cites "`design/epsilon-architecture.md` §2" — that file
  has no numbered sections.

**Fix:** correct both. The contracts one is the more important: it is the crate
whose entire purpose is to be a narrow waist, and it is currently documented as
narrower than it is.

---

## D12 — Duplicated formatting, with two different policies

**Severity: low, but it is the exact defect class this repository has been
fixing for ten milestones.**

| Thing | Two implementations |
|---|---|
| Byte sizes | `epsilon-tools/src/util.rs:429 format_size` → `50.0KB`; `epsilon-tui/src/picker.rs:1045 format_bytes` → `1.4 MB` |
| Durations | `epsilon-tui/src/ui/format.rs:75 format_duration` → `5m01s`; `epsilon-tui/src/picker.rs:1058 age_label` → `5m ago` |
| HTTP status text | `epsilon-llm/src/http.rs:82 status_error` (dead); `epsilon-tools/src/webfetch.rs:72` (live) |
| LSP `Content-Length` header | built twice in one file, `epsilon-lsp/src/framing.rs:33` and `:98` |

Four independent bounded-line readers for newline-delimited protocols, each with
its own cap semantics: `epsilon-tools/src/util.rs:382`, `epsilon-ext/src/host.rs:348`
and `:408`, `epsilon-lsp/src/framing.rs`, `epsilon-llm/src/oauth_login.rs:1107`.
`docs/milestones/M103-extension-host-protocol.md` records a real defect that came
from one of them getting its cap wrong — the convergence is not academic.

Six "truncate" helpers with four different meanings: `tools/util.rs:448`,
`tools/util.rs:790`, `tools/search.rs:321`, `core/prompt.rs:453`,
`tui/state.rs:1423` and `:1427` (one implementation, two names), and
`session/store.rs:976` (file truncation, not string).

**Fix:** one owner per concept, in the crate that owns the concept. The line
readers are the ones worth unifying first.

---

## D13 — The `docs/README.md` table that renders as body text

**Severity: low. Cost: eighteen of the most recent milestones are invisible to
anyone reading the documentation index.**

`docs/README.md:2-19` is an eighteen-row table (M178 down to M161) with no
header row, no leading `|` on any row, and no heading, wedged between the
`# Documentation` H1 and the intro paragraph. It renders as a paragraph. Those
eighteen milestones appear in no other index — the "Current hardening work" list
further down starts at M145.

**Fix:** give it a header and pipes, or fold it into the list below. All the
links in that file resolve; this is the only structural problem in it.

---

## D14 — Two config surfaces with no single owner

**Severity: low, rising.** Configuration is read in three crates —
`epsilon-core/src/config.rs:20,55` (global), `epsilon-cli/src/runtime.rs:29`
(`ProjectConfig` and trust), `epsilon-tui/src/app.rs:37` (`TuiConfig`) — and both
`config.rs:20-29` and `runtime.rs` hand-roll "read a string key, trim, treat
empty as absent". The behaviour is currently identical; nothing keeps it that
way.

**Fix:** one reader, in the crate that owns configuration, with the other two
calling it. This is a restructure task, not a bug fix.

---

## D15 — The TUI integration test never types while a run is in flight

**Severity: high, as a measurement gap.** Not a defect in the product; a defect
in the thing that is supposed to catch product defects.

`crates/epsilon-cli/tests/terminal.rs` has seven tests. Three type into the
composer. **One** submits a turn (`terminal.rs:393-428`), and its typing happens
against an idle TUI with an empty transcript, *before* Enter.

So the one state the owner is describing — a run in flight, the user typing into
it — is never exercised. Additional blind spots, all in the same file:

- The faux provider answers in well under a second (`default_steps()` at
  `main.rs:1603-1611` is `bash echo epsilon-ready` + `Run complete.`), so the
  transcript is three rows and **`streaming_text` is empty for the entire run**.
  The two O(tail) wraps in D8 and the `MessageUpdate` path are never reached.
- Every test uses a fresh temp dir (`:285-289`), so the per-frame transcript
  clone in D7 is invisible.
- There is no latency assertion anywhere — `SETTLE = 7 s`, `STEP = 600 ms`
  (`:38,40`). A frame that takes three seconds passes.
- CPU is never sampled, so a pegged main thread and a blocked one look identical
  to `screen.contains(...)`.
- ConPTY emulates the console *inside* the terminal application, so nothing here
  can observe a real conhost window or console mode.

**Fix:** a test that types a second line *after* Enter, while the run is in
flight, and asserts the echo appears within a stated wall-clock bound. That single
test would have caught the reported freeze. It needs a provider step slow enough
to type into — which is what `SlowProvider` is for, and what the faux path
currently does not use.

---

## D16 — Four root scratch files

`.tmp-msg-m106.txt`, `.tmp-msg-m107.txt`, `.tmp-msg-m108.txt`, `.tmp-rows.txt`
are sitting in the repository root. They are untracked — `.gitignore:6` covers
`/.tmp-*` — so git is clean, but they are consumed scratch left where the next
reader will trip over them. **Fix: delete.**

---

# Added 2026-09-28

Defects found in this pass. Each was verified with a command named in the entry,
because D2 above is a standing reminder that a file:line citation is not evidence.

## D17 — Delegated subagent tokens are never counted in the cost

**Severity: high. It is about money, it is silent, and it is unbounded.**

The `task` tool returns its child's aggregated usage on `ToolOutput.usage`
(`crates/epsilon-tools/src/task.rs:231,281`). `crates/epsilon-core/src/exec.rs:736`
carries it into the `ToolResultMessage`. And then nothing reads it:

```
$ grep -n "usage" crates/epsilon-tui/src/state.rs
  → the Message::ToolResult arm (state.rs:1022-1061) never reads t.usage
  → the hydrate arm (state.rs:909-934) never reads it either
  → the only reader of per-message usage in epsilon-tui is app/session.rs:205,
    which is the /resume path
```

Every other usage in this codebase flows through `record_usage`
(`state.rs:573-579`). This path bypasses it. A session that fans out into
subagents under-reports its cost by an arbitrary multiple, with **no `+` marker**,
because `is_floor` fires only for unpriced *models* and *buckets* — never for
missing *tokens*.

**Fix:** read `t.usage` where the sibling usage is already read, and route it
through `record_usage`. One reader, in one place. The test is a fan-out that
asserts the bill rose by the child's usage.

## D18 — The per-frame context mirror deep-clones the whole transcript

**Severity: medium, and rising with session length. Measured.**

`crates/epsilon-tui/src/app/event.rs:169` calls `App::mirror_context_size` on
every drawn frame — up to 30×/second, on the event-loop thread, which is the
same thread that must read the keyboard. It reaches
`Agent::estimated_context_tokens` (`core/src/lib.rs:1934-1937`), which calls
`self.history()` — a full deep clone of every message (`lib.rs:1114-1116`) — to
produce one integer.

`--tui-bench` structurally cannot see this: it calls `draw_frame` directly and
its `bench_app` holds no `Agent`, so the mirror never runs.

Measured on this machine (commit `47ca904`,
`crates/epsilon-core/tests/context_mirror_cost.rs`):

| Transcript | Cost per mirror |
|---|---|
| 20 messages | 16.9 µs |
| 200 messages | 216.7 µs |

**12.9× cost for a 10× larger transcript** — the signature of a whole-transcript
scan rather than the anchored walk it intends to be. The test's second assertion
fails today, deliberately; it is the tripwire for this fix.

**Honest limit:** at 216 µs this is ~0.7% of a 33 ms frame. It is a real defect
and it is O(session) on the input thread, but it is **not** established as the
reported freeze. See [04](04-tui.md).

**Fix:** `anchored_tokens` already takes `&[Message]`; the clone is the caller's
choice, not the function's. Hoist or memoise the mirror so a frame does not pay
for a session-length clone.

## D19 — A panic leaves the terminal unusable

**Severity: medium. A panic is indistinguishable from a freeze.**

`crates/epsilon-tui/src/app.rs:906-926` calls `ratatui::try_init()` and
`ratatui::try_restore()` with no `Drop` guard and no panic hook. A panic unwinds
past both, leaving raw mode, the alternate screen and a hidden cursor — the
symptom is a screen that never updates, which is exactly what the owner reported
and impossible to diagnose from the outside.

**Fix:** a panic hook plus a `Drop` guard that restores unconditionally. Small,
and it converts the next occurrence of an unexplained freeze into a readable
error.

## D20 — A killed TUI blocks the workspace for 30 seconds

**Severity: medium. Confirmed by hitting it repeatedly.**

A TUI killed with `taskkill /F` leaves `.epsilon/sessions/tui.lock` behind. The
next launch then waits on `STALE_AFTER`/`ACQUIRE_WAIT` (`lease.rs:50,59`) for 30
seconds, showing a truncated "waiting for this session to be released" frame with
no working input. The recorded pid is deliberately never probed for liveness.

I hit this on every probe run: `scripts/console-probe.py` ends with `taskkill /F`,
so each run needs `rm .epsilon/sessions/tui.lock` first. A user who closes a
terminal window pays 30 seconds per launch until they find the file.

**Fix:** probe the recorded pid, or shorten the steal window once it is known
dead.

## D21 — `epsilon-lsp` compiles without the workspace lint law

*(Carried forward from D1, which was correct; repeated here because the severity
is easy to lose in a long ledger.)*

`crates/epsilon-lsp/Cargo.toml:21-23` declares a `[lints.rust]` table with
`missing_docs = "warn"` and **no `workspace = true`**. A package `[lints]` table
*replaces* the workspace one rather than merging with it, so this crate compiles
with neither `unsafe_code = "forbid"` nor `clippy::all = "warn"`. Verified:

```
$ sed -n '18,25p' crates/epsilon-lsp/Cargo.toml
  [lints.rust]
  missing_docs = "warn"
```

There is no `unsafe` in the crate today. The defect is that nothing would stop
the first one, against a law `AGENTS.md` states as repository-wide.

**Fix:** add `workspace = true`, keeping `missing_docs = "warn"` as the local
override. One line.

---

# Status, 2026-09-28

Five defects opened this audit are now closed. Each was fixed *and* measured; the
measurement is the point, because the whole reason this ledger exists is that the
previous pass asserted causes it had not reproduced.

| # | Defect | Commit | How it was verified |
|---|---|---|---|
| D17 | Subagent tokens never counted in the cost | `12ae518` | 4 unit tests; a negative control removing both accounting calls made 2 fail for the right reason |
| D18 | Per-frame transcript clone | `fc9ed3b` | 144.3 µs → **276 ns, flat** across 10× the transcript |
| D19 | A panic bricks the terminal | `a4e9fa2` | Real conhost: control **FAIL**, fix **PASS** |
| D20 | A killed TUI blocks for 30 s | `5d4e205` | Real conhost: 30 s wait → no wait notice at all |
| D21 | `epsilon-lsp` outside the lint law | `dda2b54` | `clippy -D warnings` clean; workspace sweep shows one remaining exception, and it is the recorded one |

## The headline result, and what it is not

**The reported typing-freeze does not reproduce.** `PacedProvider` (`f318bc5`) made a
turn last seconds, which nothing in this repository had ever done, and then the TUI was
typed into *while it streamed*: 42 characters appeared, the cursor advanced, output kept
growing — cmd and PowerShell, 100×30, debug build. `f9f1d25` and the permanent test in
`b3fa012`.

That is a narrow claim, and it is deliberately not a larger one. It does **not** mean the
owner has not seen it. Two candidates remain untestable from here: a stale installed
binary, and the owner's own terminal and real provider.

## What this ledger was wrong about

- **D2 was retracted** — `epsilon-server` uses `epsilon-tools` in production, not tests.
- **The inherited TUI diagnosis was wrong twice over.** It blamed a permission dialog
  that renders `y/a/n/e` and handles `y`; and it blamed the transcript clone, which is
  real and now fixed but measured at ~0.4% of a frame. Both are recorded because
  inferring a cause from a symptom is exactly what this pass was hired to stop doing.

## The class, not just the instance: seams that pass tests and are never wired

`branch_with_summary` and `ToolHooks`/`TurnHooks` were the same defect wearing different
clothes. Worth writing down as a class, because the symptom is invisible by construction:

- The primitive is ported faithfully and reads well against the reference.
- It has **passing tests**, because the tests call it directly.
- Nothing on the user path calls it.
- A reader checking for the capability sees green tests and concludes it works.

`set_tool_hooks` / `set_turn_hooks` appeared **10 times in the repository and every one was
in `crates/epsilon-core/tests/loop_scenarios.rs`**. The traits were the right shape for pi's
mutating extension events and the agent loop invoked them correctly — and a real extension
child could reach neither, because `send_event` returned `()` with no reply to carry a
decision. Found 2026-09-28; see
[`evidence/delegation-port-gaps-2026-09-28.md`](evidence/delegation-port-gaps-2026-09-28.md).

**Closed 2026-09-28** across `e9d5b94` (ADR), `ece3af4` (transport), `9f73a5f` (tool
before-arm), `2fb5219` (tool after-arm) and `570819e` (turn hooks). All four `ToolHooks` and
all four `TurnHooks` methods are now reachable from a real extension process, through
`install_extension_hooks` and `install_extension_turn_hooks` in `main.rs`. Each is proved
against a live child, and each has a negative control that fails for the reason it names.

**The detection rule that follows, and it still stands:** for any capability a reference
harness has and this one claims, count the callers by *path*, not by name. Zero non-test
callers on a primitive with tests is a defect, not a proof of depth.

**One instance of the class remains:** `branch_with_summary` (`session/src/store.rs:1372`).
Confirmed three times over this session. It is a different fix — no protocol is missing, just
no caller — and it is the honest next candidate for this rule.

## Still open

- **0.6** promote `docs/design/epsilon-architecture.draft.md`.
- **146 broken links**, all in `audit/history/` — deliberately unrepaired, because a
  blind rewrite points readers at the wrong document.
- **Phase 4** is in progress: `/tree` re-projection landed, then ADR-019 through transport
  (`ece3af4`) and the production bridge (`9f73a5f`). **Phase 5** (repository structure) not
  started. Remaining in Phase 4: `AfterToolCallResult` wiring (result rewriting), and
  ~~TurnHooks still test-only.~~ **Closed** in `570819e`: `ExtensionTurnHooks`
  is the first production `set_turn_hooks` caller. `transform_context`/`convert_to_llm`
  remain pass-through **by decision** and are not a defect.
- ~~**D22 — the installed release binary predates ADR-019.**~~ **Closed** by reinstall at
  05:27 (15m 09s release build) and verified by marker plus two live probes — see
  [`installed-binary-2026-09-28-post-adr019.md`](evidence/installed-binary-2026-09-28-post-adr019.md).
  All six ADR-019 and session markers are present, typing during a paced stream rendered
  **40 characters** in the editor, and the crash probe reports
  `PASS: the terminal was restored and the crash was reported`.
- **D24 — `docs/README.md` listed `user/` generically, orphaning every user guide
  added in a hurry.** `docs/user/extensions.md` (M183) and `docs/user/prompt-templates.md`
  (M184) were both reachable only by knowing the filename, which is the "new documentation
  files must be added to the docs map" rule in AGENTS.md unenforced. **Closed** in `363a0ee`
  by naming both in the product-documentation row. Not a code defect, but it is the same
  failure as a dead link: a document nothing points at is a document nobody finds.
- ~~**D23 — `docs/history.md` is corrupt below M160.**~~ **CLOSED** in `f6636ff`.

  The corruption was 18 consecutive lines (196–213) each carrying a spurious `M161`–`M178`
  prefix glued onto whatever followed: `M163| M144 | **Read side only…`, and
  `M167## Decision-log index` swallowing a section heading. **The 18 milestone rows were lost,
  not merely glued.** Verified the pattern before editing — exactly 18 lines, M161–M178 in
  order, each a prefix — then stripped the prefixes, which recovered the three displaced rows
  intact, and reconstructed all 18 rows **from the dev logs** (every one of M161–M178 has
  one, so no description is guessed). M154, M186, M16, M80, M179 and M180 recovered the same
  way, each row naming the record it came from.

  Also repaired: three rows truncated mid-sentence by an eaten `|`, the M185 row that had
  swallowed two lines of trailing prose, and a duplicate M6.

  Verified afterwards: timeline table 105 rows, acceptance table 85 rows, **0 out-of-order,
  0 duplicates, 0 unclosed, 0 glued lines**, and every milestone 1–186 has a row somewhere.
  M6/M7/M8/M10 appear in both tables — legitimately, since they are different tables.

  **A verification of mine was wrong first, and is worth recording.** I sliced the table to
  lines 9–130, found 79 "missing" milestones, and began reconstructing them — until M96
  turned up at line 143 under a *different* heading. `## Original acceptance criteria` is a
  second table by design and M96–M178 belong to it. The missing list was an artefact of my
  slice, not a defect. The check that answers the real question spans both tables.

- **D26 — the RPC `follow_up` op was reachable under a name pi does not use. FIXED** in M188.

  pi documents `{ type: "follow_up" }` (`rpc-types.ts:30`). Epsilon accepted only
  `followup`, because serde's `rename_all = "snake_case"` turns the variant `Followup` into
  `followup` and nothing corrected it. A pi-shaped client sending the documented name got
  `unknown op: follow_up` — a refusal of the exact name the reference implementation says to
  use, on a surface whose purpose is pi compatibility.

  **It survived because the test read the wrong side of the boundary.**
  `ops_deserialize_with_snake_tags` deserialized the enum and compared serde's output against
  the variant list, so it agreed with the code by construction. Nothing in the suite ever
  dispatched the op and read the reply, which is the only thing that exercises a wire name.

  Found by a precondition assertion I added while writing M188, and diagnosed by a probe
  that printed its own reply:
  `{"command":"follow_up","error":"unknown op: follow_up","success":false}`.

  **Fixed with an explicit `rename`, not an `alias` alone.** `#[serde(other)] Unknown` on the
  same enum absorbs any unrecognised name before serde reaches the alias check, so
  `#[serde(alias = "followup")]` compiled, looked right, and did nothing. The working form is
  `#[serde(rename = "follow_up", alias = "followup")]`.

  Two tests now close it, and the rule they encode is general: **a protocol test must go
  through the wire.** Deserializing a type proves the type parses; only dispatching a frame
  and reading the reply proves the protocol accepts what a client will send.

- **D25 — a history row numbered `| 018 |`, which predates the D23 corruption and is still
  not explained.** Present in `1873cc8~1`, the README-era history, so it is not this session's
  work. It reads as an `M1…` with the `M` eaten, and M11 is the model-picker milestone whose
  log matches the row's text — but *confirming that is a historical question, and guessing
  would put an unverified claim in the permanent record*. Left verbatim and flagged. A
  mechanical check (`every | M<n> | row is a well-formed milestone number`) will catch it.

- **The batteries ran, and they passed** — `audit/evidence/gate-m180-1.md` and
  `gate-m180-final.md`; the latter reports 1316 results across 63 targets, 0 failures. (The
  "1253 tests" figure that used to sit here was derived by subtracting a per-crate sweep
  from a battery total, which does not survive parsing the log; the correction is in
  `gate-m180-1.md`.) Two things the battery does not cover: `--no-default-features` (which
  found a real pre-existing compile failure, now fixed in `57812ca`), and anything whose
  symptom is a broken console rather than a wrong value.
- The installed `C:/Users/Hp/.cargo/bin/epsilon.exe` **is** the current release build, verified
  for the session/TUI work at [`installed-binary-2026-09-28.md`](evidence/installed-binary-2026-09-28.md)
  and for ADR-019 at
  [`installed-binary-2026-09-28-post-adr019.md`](evidence/installed-binary-2026-09-28-post-adr019.md).
  One caveat is now recorded rather than left for the next reader to rediscover: passing
  `--exe-args "--crash"` to `console-probe.py` yields a **false PASS**, because the probe sets
  `EPSILON_TUI_CRASH` in the child environment itself and the argument only makes the CLI exit
  normally. The console is healthy precisely because nothing crashed.

- **D27 — M189 shipped `get_tree` with the WRONG SHAPE: a flat array where pi returns a
  nested forest. FIXED in M190.** Found while verifying M189's installed binary, by
  reading pi's `session-manager.ts` rather than its type declarations.

  pi's `getTree()` (`core/session-manager.ts`) builds a **recursive** structure over **every**
  entry: a node is `{ entry, children: SessionTreeNode[], label?, labelTimestamp? }`, roots
  are entries with `parentId === null` **or a broken parent chain** (orphans become roots),
  and each level's children are sorted by `entry.timestamp` ascending. `SessionEntry` itself
  is `{ type, id, parentId, timestamp }` — camelCase, and `timestamp` is an **ISO string**,
  not epoch millis.

  What M189 shipped: `tree` as a **flat array of the current branch**, with
  `{id, parentId, seq, timestampMs, kind}`. Three separate mismatches — flat vs nested,
  branch vs whole file, and `timestampMs: number` vs `timestamp: string`. A client written
  against pi would render no tree at all: `node.children` is `undefined`, so the walk that
  draws a tree throws or draws nothing.

  **It passed 1,446 tests including two controls**, because the tests asserted against my
  own shape. This is D26's lesson in a new form: *reading a reference's TYPE proves the
  field names, and reading its IMPLEMENTATION proves the structure.* `rpc-types.ts` says
  `tree: SessionTreeNode[]` and I checked the element type; that array is trees, not entries,
  and only the implementation says so.

  Also found in the same reading, same severity class: **pi's `getMessages()` returns
  `AgentMessage[]` directly** (`rpc-client.ts`: `this.getData<AgentMessage[]>(...).messages`),
  and `getCommands()` returns `RpcSlashCommand[]` — M188 wrapped both in an object
  (`{messages: [...]}`, `{commands: [...], count}`). `getEntries`/`getTree`/`getSessionStats`
  are the ones that really are objects in pi. So three of the M188/M189 replies carry a
  wrapper pi's client does not unwrap.

  **FIXED in M190** (`d4df88e` fix, `b2b06c5` D27 record, `968dd15` gate evidence).
  `get_tree` is rebuilt as a recursive forest over `read_all()`, roots on no-parent,
  self-parent and orphan, children sorted by timestamp at every level, and — beyond pi —
  a parent that is an ancestor of the child is promoted to a root so a hand-edited
  `a → b → a` cycle renders both entries exactly once instead of losing them.
  `entry_as_pi` emits `parentId` camelCase and an ISO-8601 UTC string with millis and a
  `Z`, matching JavaScript's `toISOString()`. `get_messages` and `get_commands` now return
  bare arrays.

  **Two controls, both failing on the defect they name:**
  - **D** — `get_tree` reverted to M189's flat shape → **2 failed**:
    *"every node needs a children ARRAY (pi's SessionTreeNode)"*, and an entry present in
    the file but missing from the tree.
  - **E** — `get_messages`/`get_commands` restored to M188's object wrappers → **4 failed**.

  `epsilon-cli --lib`: 114 passed. Focused rpc: 49 passed.

  The gate rule this earned: **read the reference's IMPLEMENTATION, and read the client
  that CONSUMES the payload, not only the type declaration.** A declaration gives field
  names; only the implementation and the consumer give structure.

## D28 — the parity count was wrong, and nothing could have caught it

**Status: closed in M192.** Not a code defect: a *measurement* defect, which is worse in one
respect and better in another. It was wrong in every document that repeated it, across
several milestones, and no test, clippy, or gate stage consulted it.

**The claim:** "18 of pi's 33 RPC client commands are implemented."

**The truth before M192:** 19. **`follow_up`'s primary wire name lives in
`#[serde(rename = "follow_up", alias = "followup")]`**, so the Rust variant is `Followup`
and the string `follow_up` never appears as an identifier. Any count derived by grepping
the `Op` enum for wire names misses it — precisely the blindness `AGENTS.md` records under
"Protocol surfaces" for D26, where the same attribute pair is why a marker search is
useless. I had written that rule down and then did the thing it warns about, to measure
parity.

**How it was found:** writing M192, I needed the *remainder* to know what was left, and
found that "18 plus four" did not reconcile with a count derived from the reference's own
`RpcCommand` union. The union has exactly 33 members, so the denominator is exact and the
numerator is not.

**The correct method**, now written into `agents/memory/01-plan-execution.md`:

1. Take the denominator from the reference's own type union — 33, exact, not estimated.
2. Match each member against the implementation **variant by variant**.
3. Resolve every implementation name through **serde**, not through the Rust identifier.
4. Never carry a count forward. Re-derive it; a count that requires trusting the previous
   count is a count that drifts.

**Result:** **23 of 33** after M192, ten named in
`audit/02-port-fidelity.md` and `audit/evidence/gate-m192b.md`.

**The generalisation.** A number in a document is a claim with no reviewer. It survived
M186, M188, M189, M190 and M191 unmodified because every milestone that repeated it also
*believed* it, and belief is not measurement. This is the same failure as D27 in a
different costume: **an assertion that agrees with its own author by construction**. The
defects that survive longest are the ones nothing is positioned to disagree with.

## D29 — one reference type, two wire shapes, and 1,458 tests agreed with neither

**Status: closed in M193 (`6a19af1`).** D27's twin, found one function over, and the
second time the same root cause has cost this project real work.

**The defect.** `get_entries` returned epsilon's internal `SessionEntry` structs while
`get_tree` returned pi's. Measured on the installed binary, same session, same entry:

```json
// get_entries -- WRONG
{"id":"ent_1a0e7861a211",
 "kind":{"custom_type":"session_name","data":{"name":"…"},"type":"custom"},
 "parent_id":null,"seq":0,"timestamp_ms":1790590720545}

// get_tree -- RIGHT
{"id":"ent_1a0e7861a211",
 "parentId":null,"timestamp":"2026-09-28T10:18:40.545Z","type":"custom"}
```

pi returns the same `SessionEntryBase` behind both
(`session-manager.ts`; `rpc-mode.ts:638-648`), so a client reading `entry.parentId` from
`get_entries` got `undefined`, and `entry.timestamp` was epoch millis where pi sends an
ISO-8601 string.

**Why it survived M190 and M192B.** 63 targets, 1,458 results, 0 failed. The M190 tests
asserted `get_entries` against **epsilon's own output** — raw and converted agreed by
construction, so the assertion could not fail. D27's exact mechanism, and I had already
written the lesson down: *"a type declaration gives field names; only the implementation
and the consumer give structure."* The structure was right in one function and wrong in
its neighbour, and no test compared them.

**How it was found.** Driving the **installed** release over real stdio and reading raw
JSON. Not a stricter unit test — a different kind of check. Every in-process assertion was
comparing epsilon to epsilon; only a wire probe compares epsilon to the reference.

**The fix, chosen for the class rather than the instance.** Both ops now call
`entry_as_pi`. One conversion, two callers, so a second shape is *impossible* rather than
merely absent. Adding a test that pins the correct shape would have left the third op free
to invent one.

**The tests are deliberately two kinds:**

- `get_entries_and_get_tree_agree_on_every_entries_shape` **compares the two ops** field by
  field and by JSON value type. A fixed literal would be my guess again, and a guess is
  what produced both defects. What cannot be wrong is that one reference type cannot be
  served two ways — and that is checkable without knowing the right answer.
- `every_get_entries_entry_has_pis_four_fields` pins the four names against the reference
  line, and asserts `parent_id`, `seq`, `timestamp_ms` and `kind` are **absent**. The
  defect was their presence, so their absence is what pins the fix.

**Control I** (raw structs restored) fails both, printing the defect:
`left: [id, kind, parent_id, seq, timestamp_ms]` vs `right: [id, parentId, timestamp, type]`.

**The generalisation, now twice earned.** An assertion that agrees with its own author by
construction is not a test — and neither is a *pair* of such assertions. When two code paths
serve the same reference type, the test that matters is the one that compares **them to each
other**, because that is the only comparison whose expected value does not come from the
implementation.

**A third finding, not a defect but the same lesson:** a new session was named
`ses_….jsonl` while a fork was named `2026-09-28T10-28-30-472Z_ses_….jsonl`, in the same
directory. Nothing failed; every test passed. Found by *listing the directory* after the
wire probe. Two spellings in one place sort into two groups and there is no visible reason
why. Fixed by moving the convention into `SessionStore::new_session_path()` — a second
caller cannot invent a third name — in `1b79f8a`.
