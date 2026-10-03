# Documentation

For users and contributors finding current guides, design intent and historical records.
The implementation is the authority when documents disagree; older design and milestone
claims are not proof that a capability works today.

## Product documentation

| File | What lives there | Read it when |
|---|---|---|
| [`../README.md`](../README.md) | the entry point: what Epsilon is, install, quick start | first |
| [`user/`](user) | product guides: install, configuration, auth, permissions, sessions, [goals](user/goals.md), [extensions](user/extensions.md), [prompt templates](user/prompt-templates.md), troubleshooting | you are using or configuring Epsilon |
| [`../CHANGELOG.md`](../CHANGELOG.md) | user-visible changes, milestone by milestone, newest first | you want to know what changed |

## Contributor documentation

Agent recovery state lives in [`agents/memory/`](../agents/memory/README.md):
curated open-work notes, evidence pointers and safe resumption steps, not product docs.

| Folder / file | What lives there | Read it when |
|---|---|---|
| [`../AGENTS.md`](../AGENTS.md) | the rules for anyone writing code here: gates, conventions, disk hygiene | before your first change |
| [`design/`](design) | **how and why**: the architecture tour, the ADR log, the skills design | you are changing behaviour |
| [`qa/`](qa) | design conversations turned into notes: why Rust, the brick methodology, the harness primer, TUI keymaps, OAuth client ids, provider extension points, the repository layout, **which code belongs to a frontend and which to the composition** | you need the convention behind a choice |
| [`plans/README.md`](plans/README.md) | historical plans and their recorded statuses, not fresh authorization | you want to trace earlier scope |
| [`../audit/`](../audit) | the dated assessment: usability review, defect ledger D1–D32, port plan, verification log, reference comparison, roadmap | you want to know what is broken or what to do next |

## Historical record

| Folder / file | What lives there | Read it when |
|---|---|---|
| [`history.md`](history.md) | the milestone timeline and acceptance notes | you want the project's story in one table |
| [`milestones/`](milestones) | one dev log per milestone: what was built, decisions, bugs found, verification output. **The annotated list below covers recent work; older milestones are covered by this directory row and by [`history.md`](history.md)**, so a file that is not named individually is not an orphan — a consistency check that flags M1–M160 is testing the wrong convention | you are touching an area and want its history |
| [`research/`](research) | source-level studies of the reference harnesses and the gap analyses (dated evidence, not current status) | you are deciding what to build next |
| [`handoff/`](handoff) | archived handoff prompts from earlier milestones (historical, not active tasks) | you are tracing how a slice of work was delegated |
| [`glossary.md`](glossary.md) | the vocabulary — harness, turn, tool call, compaction, lease, … | you hit an unfamiliar term |
| `../references/` | the four reference harnesses' sources (not committed; research input) | you want to check a port against its origin |

## Reading paths

**New to coding agents?** [`qa/06-agent-harness-primer.md`](qa/06-agent-harness-primer.md) with
[`glossary.md`](glossary.md) open beside it, then the
[architecture guide](design/epsilon-architecture.md) — which now carries a dated header
saying which parts were verified when, so read a claim there as a hypothesis to check.

**New to the codebase?** Root README → [`design/epsilon-architecture.md`](design/epsilon-architecture.md)
→ the crate that owns what you are touching → its milestone log in `milestones/` → the tests in
that crate.

**Writing a frontend, or changing one?** [`qa/11-frontend-ownership.md`](qa/11-frontend-ownership.md)
first. It is the rule that decides whether a change belongs in `epsilon-tui`,
`epsilon-desktop`, or `epsilon-runtime`, and it exists because the first attempt at this
would have needed fourteen steps copied into a second place.

**Reviewing a change?** `AGENTS.md` (gates), then the ADR log
([`design/epsilon-decisions.md`](design/epsilon-decisions.md)) for the behaviour it changes,
then the milestone log that introduces it.

**Deciding what to build next?** [`../audit/03-defects.md`](../audit/03-defects.md) —
every row now carries a status (OPEN / CLOSED / CLOSED with a named residual / FIXED pending
verification), and the status is only as good as the milestone log it names —
[`../audit/02-port-fidelity.md`](../audit/02-port-fidelity.md) and
[`../audit/07-plan.md`](../audit/07-plan.md), then the relevant study in `research/`.

## Current hardening work

Newest first. Each entry is a two-line pointer; the linked log has the reasoning, the
decisions and what the slice could not do. The logs marked *(reconstructed)* were
written after the fact from the commits, because those milestones landed without one —
they say so themselves, and their `## Verification` sections name what could not be
recovered rather than inventing it.

**This list is not the timeline.** Every milestone has a row in [`history.md`](history.md) except
M80 — whose checkpoint work is recorded inside [`M81`](milestones/M81-observation-ownership.md)
rather than reconstructed — and almost every one has a log in [`milestones/`](milestones). Where
no log was written the pointer below goes to the timeline row, which is then the whole record:
M0, M29–M32, M120, M122–M124 and M128. This list is the subset a reader is most likely to want
first, newest at the top, and it is kept current from M83 forward.

> **The pointer list had stopped at M160 for forty milestones**, while the sentence above
> claimed it was current — the drift this repository is built to catch, in the file that is
> the map. M161–M201 are listed now, and the log-less ones say so on their own line rather
> than being silently absent. The list is generated from the `H1` of each log, so an entry
> cannot describe a milestone the log does not.

- [M231 — a 202 ms startup scan, counted instead of summarized](milestones/M231-counting-instead-of-summarizing.md)
- [M233 — a native desktop IDE, and the composition crate it needed first](milestones/M233-desktop-ide-and-runtime-crate.md)
- [M234 — the second half of ADR-021: launch resolution is a library function too](milestones/M234-launch-resolution-is-a-library-function.md)
- [M235 — the agent is in the window](milestones/M235-the-agent-is-in-the-window.md)
- [M236 — the file tree and a read-only editor](milestones/M236-file-tree-and-a-read-only-editor.md)
- [M237 — reviewing what the agent changed, and the window that was never idle](milestones/M237-reviewing-what-the-agent-changed.md)
- [M238 — the assistant's markdown, and a message that arrived alone](milestones/M238-the-assistant-s-markdown.md)
- [M239 — the prompt cache was marking the one place that never moves](milestones/M239-the-prompt-cache-was-marking-the-wrong-place.md)
- [M240 — a goal's gates: "done" stops being something the model says](milestones/M240-goal-gates.md)
- [M241 — declaring a gate](milestones/M241-declaring-a-gate.md)
- [M242 — a switchable sidebar, and the agent's own subagents](milestones/M242-a-switchable-sidebar.md)
- [M243 — a sessions view that can actually switch sessions](milestones/M243-a-sessions-view-that-switches.md)
- [M244 — the compaction instruction that let an agent forget what it was told](milestones/M244-the-compaction-instruction-that-let-an-agent-forget.md)
- [M245 — when a session ran, and a test passing for the wrong reason](milestones/M245-when-a-session-ran-and-a-test-passing-for-the-wrong-reason.md)
- [M246 — the subagent states nobody checked](milestones/M246-the-subagent-states-nobody-checked.md)
- [M247 — a row that does not fit now says so](milestones/M247-a-row-that-does-not-fit-now-says-so.md)
- [M248 — the window shows the conversation](milestones/M248-the-window-shows-the-conversation.md)
- [M249 — the window follows the agent, and the keyboard goes where you put it](milestones/M249-the-window-follows-the-agent.md)
- [M250 — the review the window was pointing at](milestones/M250-the-review-the-window-was-pointing-at.md)
- [M251 — a keyboard route to everything](milestones/M251-a-keyboard-route-to-everything.md)
- [M252 — the window was opened, and looked at](milestones/M252-the-window-was-opened-and-looked-at.md)
- [M253 — a finding recorded instead of a feature shipped, and a palette you can click](milestones/M253-a-finding-recorded-and-a-palette-you-can-click.md)
- [M254 — a goal round consults the goal's gates](milestones/M254-a-goal-round-consults-its-gates.md)
- [M255 — the branch that is the security argument, finally executed](milestones/M255-the-branch-that-is-the-security-argument.md)
- [M256 — an unchanged workspace stops paying for the same failure](milestones/M256-an-unchanged-workspace-stops-paying.md)
- [M257 — the gated-goal test that had never existed](milestones/M257-the-gated-goal-test-that-had-never-existed.md)
- [M258 — the window has a goal now](milestones/M258-the-window-has-a-goal-now.md)
- [M259 — the window can control a goal, and a fifth reference arrived](milestones/M259-the-window-can-control-a-goal.md)
- [M260 — a command cannot be displayed as something it is not](milestones/M260-a-command-cannot-be-displayed-as-something-it-is-not.md)
- [M261 — the permission dialog cannot be lied to, and the footer stopped agreeing with it](milestones/M261-the-permission-dialog-cannot-be-lied-to.md)
- [M262 — the search view could end the session](milestones/M262-the-search-view-could-end-the-session.md)
- [M263 — a band painted across the file tree, and a budget in the wrong unit](milestones/M263-the-wrong-unit-and-the-wrong-pane.md)
- [M264 — `/compact` worked once, and the window never showed what a tool printed](milestones/M264-compact-and-tool-output.md)
- [M265 — the workspace walk followed a link out of the workspace](milestones/M265-the-walk-followed-a-link-out-of-the-workspace.md)
- [M266 — the desktop frame path had never been measured, and it is not what the review said](milestones/M266-the-desktop-frame-path-had-never-been-measured.md)
- [M267 — the window had no slash commands, and the goal round channel was two channels](milestones/M267-window-commands.md)
- [M268 — one `/goal`, two frontends](milestones/M268-one-goal-command.md)
- [M269 — the window's primary content was invisible to every measurement](milestones/M269-prose-was-already-wrapping.md)
- [M270 — Enter did nothing, and two predicted defects were not real](milestones/M270-silent-enter.md)
- [M232 — a gate for the failure that lied about itself](milestones/M232-a-gate-for-the-failure-that-lied.md)
- [M230 — a subagent's tokens reach the session's numbers](milestones/M230-a-subagent-s-tokens-reach-the-numbers.md)
- [M229 — a subagent's tokens are stored, persisted, and shown nowhere](milestones/M229-a-subagent-s-tokens-are-shown-nowhere.md)
- [M228 — two rows that were wrong about the code, and one of them wrong about the reference](milestones/M228-two-stale-rows.md)
- [M227 — two load-sensitive tests: one dependency removed, one mystery intact](milestones/M227-two-load-sensitive-tests.md)
- [M226 — the last uncalled duplicate, and a plan step retired with its reason](milestones/M226-the-last-uncalled-duplicate.md)
- [M225 — toml leaves epsilon-llm, and a config file is read once per process](milestones/M225-toml-leaves-llm.md)
- [M224 — the OAuth table is read once, by the crate that owns the file](milestones/M224-one-oauth-read.md)
- [M223 — one implementation of the global directory, found by reading the dependency graph](milestones/M223-one-global-dir.md)
- [M222 — the notice and `-c` now agree about what exists](milestones/M222-notice-and-continue-agree.md)
- [M221 — /new starts a new session file, and the primitive it needed is deleted](milestones/M221-new-starts-a-new-file.md)
- [M220 — every drawn character a CP437 console can actually draw](milestones/M220-every-drawn-character.md)
- [M219 — the character repertoire, and the default that got it wrong](milestones/M219-the-character-repertoire.md)
- [M218 — a list row that reads as one word](milestones/M218-a-list-row-that-reads-as-one-word.md)
- [M217 — a workspace's sessions get their own directory](milestones/M217-session-per-workspace-directory.md)
- [M216 — a line-ending policy, and the tree that was drifting from it](milestones/M216-line-endings.md)
- [M215 — the tool registry stops building a language server](milestones/M215-tools-lsp-inversion.md)
- [M214 — the notice M210 obsoleted, asked the question that survived](milestones/M214-earlier-sessions.md)
- [M213 — the invariant A19 lacked, with a test on it](milestones/M213-the-resume-invariant-tested.md)
- [M212 — a resumed launch opens its session once](milestones/M212-resume-opens-once.md)
- [M211 — reaping locks whose holder is gone](milestones/M211-reaping-dead-locks.md)
- [M210 — a launch writes a session file of its own](milestones/M210-a-session-file-per-launch.md)
- [M209 — the session-naming rule, transcribed from the reference](milestones/M209-session-naming-rule.md)
- [M208 — one timestamp formatter, moved before it was duplicated](milestones/M208-one-timestamp-formatter.md)
- [M207 — a wait you can still read after the screen is replaced](milestones/M207-the-wait-you-can-read.md)
- [M206 — the goal on the wire](milestones/M206-goal-on-the-wire.md)
- [M205 — the harness says what it knows](milestones/M205-what-it-knows.md)
- [M204 — the round driver: an armed goal continues itself](milestones/M204-round-driver.md)
- [M203 — `/goal`: one persisted objective, readable by the human and the model](milestones/M203-goal.md)
- [M202 — an older session is old, not damaged](milestones/M202-older-sessions.md)
- [M201 — every part of a status row is whole, and the first screen says what to do](milestones/M201-frame-truth.md)
- M200 — no log was written; the [timeline row](history.md) is the record.
- [M199 — a link is not a name: workspace containment follows symlinks](milestones/M199-symlink-containment.md)
- [M198 — the frame path, measured](milestones/M198-the-frame-path-measured.md)
- M197 — no log was written; the [timeline row](history.md) is the record.
- [M196 — GFM markdown, one session forest, branch labels, and a `/tree` that sees branches](milestones/M196-gfm-labels-and-the-session-tree.md)
- [M195 — the `@`-walk, and the fourth control that passed for the wrong reason](milestones/M195-at-walk-cache.md)
- [M194 — eleven defects, and the two questions nobody had asked](milestones/M194-accuracy-and-honest-keys.md)
- [M193 — D29: the same defect, one function over](milestones/M193-d29-entry-shape.md)
- [M192 — fork, clone, new_session, switch_session](milestones/M192-rpc-session-swap.md)
- M191 — no log was written; the [timeline row](history.md) is the record.
- [M190 — D27: the reference's type is not its behaviour](milestones/M190-d27-tree-shape.md)
- [M189 — Three store-backed RPC commands, and two controls that taught the same lesson](milestones/M189-rpc-store-commands.md)
- [M188 — Four more RPC commands, and a probe that found a wire name pi documents and epsilon refused](milestones/M188-rpc-session-commands.md)
- M187 — no log was written; the [timeline row](history.md) is the record.
- [M186 — Six pi RPC commands, and a control that found I had tested no success path](milestones/M186-rpc-commands.md)
- M185 — no log was written; the [timeline row](history.md) is the record.
- [M184 — Prompt templates](milestones/M184-prompt-templates.md)
- M183 — no log was written; the [timeline row](history.md) is the record.
- M182 — no log was written; the [timeline row](history.md) is the record.
- [M181 — Extensions intercept tool calls (ADR-019, phases 4.3 and 4.5a/4.5b)](milestones/M181-extension-interception.md)
- M180 — no log was written; the [timeline row](history.md) is the record.
- M179 — no log was written; the [timeline row](history.md) is the record.
- [M178 — putting the shell in the suite, and a stale test that had been failing for 27 milestones](milestones/M178-the-shell-in-the-suite.md)
- [M177 — the shell the owner named, measured for the first time](milestones/M177-powershell-measured-for-the-first-time.md)
- [M176 — driving a picker in a real console, and the key the probe could not send](milestones/M176-driving-a-picker-in-a-real-console.md)
- [M175 — a crate whose test count depended on which crates were in the command](milestones/M175-a-test-count-that-depended-on-the-command.md)
- [M174 — every source-shaped test, and whether each one bites](milestones/M174-every-source-shaped-test.md)
- [M173 — the "acceptable trade" that was not one](milestones/M173-the-trade-that-was-not-one.md)
- [M172 — the list the parser owns, and the seven commands it found](milestones/M172-the-list-the-parser-owns.md)
- [M171 — turning "sixteen uncovered commands" into a test that runs forever](milestones/M171-every-command-executed-at-least-once.md)
- [M170 — the real-console check, and a tool gap it exposed](milestones/M170-the-real-console-check.md)
- [M169 — the correction M168 needed, and a hazard I could not test](milestones/M169-the-correction-m168-needed.md)
- [M168 — `/tree` was a copy of `/timeline`, and closing the gap found a crash](milestones/M168-the-duplicate-timeline-and-the-crash-it-hid.md)
- [M167 — paying down a debt I created in M162, and the 18 commands no test runs](milestones/M167-paying-down-a-debt-i-created.md)
- [M166 — the 929-line `impl App`, in three](milestones/M166-the-929-line-impl-in-three.md)
- [M165 — two self-contained seams, and a coverage hole a control found](milestones/M165-two-seams-and-a-coverage-hole.md)
- [M164 — the read side of `app.rs`, and a test that named a file](milestones/M164-the-read-side-and-a-test-that-named-a-file.md)
- [M163 — the catch-all stops being a monolith](milestones/M163-the-catch-all-stops-being-a-monolith.md)
- [M162 — slash commands by subject](milestones/M162-slash-commands-by-subject.md)
- [M161 — the first seam out of `app.rs`](milestones/M161-the-first-seam-out-of-app-rs.md)
- [M145 — a bill lives on the entry that incurred it](milestones/M145-a-bill-lives-on-the-entry-that-incurred-it.md)
  — **closes F23 and the compaction half of F22**. M144's write side was reverted because a *separate* accounting
  row becomes the file's last entry, and `pop_last_message` pops the last entry to discard a failed turn — so a
  rollback could separate a summary from its cost. pi has no such entry: `Compaction.usage` sits on the compaction
  itself. Epsilon now does the same, plus the charging model, because its `Usage` is four token counts with no
  precomputed cost. One entry cannot be half-removed. The regression test reproduces the pop and asserts the bill
  goes with it, and a negative control reverts to M144's shape for a decisive RED.
- [M146 — delivery is not passage](milestones/M146-delivery-is-not-passage.md)
  — **closes F05**. A shell command exiting nonzero arrives as a *successful* tool call, so the
  verification record counted a failing test suite as a passing check; and ordering was compared in
  turn numbers, so a check before an edit in the same batch vouched for it. Two negative controls,
  one per half, so neither fix could hide the other.
- [M147 — one deadline for the whole confined run](milestones/M147-one-deadline-for-the-whole-run.md)
  — **partially closes F11** (bwrap half). The backend discarded its own
  `wall_timeout_secs`, bounded only the child wait and not the pipe drains, and leaked the
  tree on cancellation. The three tests that prove it are `cfg(unix)` and **did not run on
  the Windows host** — the status banner says so rather than implying coverage.
- [M148 — a descendant cannot hold the shell open](milestones/M148-a-descendant-cannot-hold-the-shell-open.md)
- [M149 — a short result stops saying ok](milestones/M149-a-short-result-stops-saying-ok.md)
- [M150 — the third path, and the fix that was eating the output](milestones/M150-the-third-path-and-the-eaten-output.md)
- [M151 — the welcome screen stops repeating the footer](milestones/M151-the-welcome-screen-stops-repeating-the-footer.md)
- [M152 — a running tool shows its output](milestones/M152-a-running-tool-shows-its-output.md)
- [M153 — every tool card says something specific](milestones/M153-every-tool-card-says-something.md)
- [M154 — the probe could not submit](milestones/M154-the-probe-could-not-submit.md)
- [M155 — one tool call, one card](milestones/M155-one-tool-call-one-card.md)
- [M156 — the frame test that would have caught M155](milestones/M156-the-frame-test-that-would-have-caught-m155.md)
- [M157 — a search that moves the view](milestones/M157-a-search-that-moves-the-view.md)
- [M158 — the contradiction a comment could not settle](milestones/M158-the-contradiction-a-comment-could-not-settle.md)
- [M159 — the repair was tested; the durability was not](milestones/M159-the-repair-was-tested-the-durability-was-not.md)
- [M160 — `/theme`, and the cache that would have hidden half of it](milestones/M160-theme-and-the-cache-that-would-have-hidden-it.md)
  — **closes F11's unconfined half.** The pipe drains were awaited outside the caller's timeout, so a
  surviving descendant hung a command that had already exited. Bounded, and the shortfall is
  declared. The control settles in 10.46s without the fix — the descendant's own lifetime.
- [M144 — a summarization is a real bill](milestones/M144-a-summarization-is-a-real-bill.md) — **read side
  only**. Compaction resends the whole history and is charged for it, and the response was in scope at the write
  site while the entry said `usage: None`; pi counts this charge explicitly so totals "reflect what was actually
  billed". Epsilon can now read, group and total such charges per model, and a resumed session folds them in.
  **The write side was reverted** — see F23, where `pop_last_message` can eat a bill, and **M145**, which fixed it
  by deleting the second entry rather than defending it. Carries the fourth way a
  negative control misleads: this slice's tests all exercised the *reader* with hand-built rows, so reverting the
  *writer* left 249 tests green.
- [M143 — the session says where the money went](milestones/M143-the-session-says-where-the-money-went.md):
  the cost breakdown accessors were computed, tested, documented, and called by nothing. Also F22, found
  while checking pi's parity: compaction usage is stored on the entry and read by nobody, so summarization
  cost is real money that nothing counts.
- [M142 — proving the accumulation, not just the pricing](milestones/M142-proving-the-accumulation-not-just-the-pricing.md):
  M141's tests all fed a hand-built map to the pricing function, so a bug in the code that *builds*
  that map would have passed every one of them. No product code changed here.
- [M141 — a session's bill is priced per model](milestones/M141-a-sessions-bill-is-priced-per-model.md):
  the second half of M140. A token's price belongs to the model that consumed it, so the session
  keeps its counts split by model. The error followed the direction of the switch and was invisible.
- [M140 — the header counted the cached prefix twice](milestones/M140-the-header-counted-the-cached-prefix-twice.md):
  the three numbers the owner asked for were present but wrong on most providers, in three different
  directions. Present-but-wrong is worse than absent, because a missing number gets noticed.
- [M139 — switching models survives closing the session](milestones/M139-a-model-switch-survives-a-restart.md):
  the entries were written faithfully and read by nobody. Also the third rule of pi's
  `getSessionContextSettings` that a `model_change`-only port drops: an assistant message names
  the model too, so a session that never switched still comes back on the model it ran.
- [M138 — the lease wait is visible, not a blank screen](milestones/M138-the-lease-wait-is-visible.md):
  M136 replaced a 30-second *refusal* with a 30-second *wait* — correct, and still wrong, because
  the user saw nothing while it happened. One redrawn line, a countdown, and the reason. The next
  step is the TUI alternate screen, which still erases it.
- [M137 — several SSE frames in one network chunk no longer lose all but the first](milestones/M137-every-frame-in-one-chunk-reaches-the-user.md):
  a short answer delivered in one chunk showed only the first frame and was reported as a
  **successful** completion. Also: why it survived (the stream layer had no test seam at all), and
  why `cargo test -p epsilon-llm` without `--features real` silently runs 38 fewer tests.
- [M136 — reopening a session you just closed no longer fails for 30 seconds](milestones/M136-the-lease-waits-instead-of-refusing.md):
  kill the TUI, open it again, and the harness said another instance owned the session and exited —
  because the process you just killed is precisely the one that was *not* heartbeating its lock. The
  heartbeat already answers "is anyone still there?" without a platform call, so the lease now waits
  the crash window out and takes the session the moment the heartbeat stops. A **pid-liveness probe
  was considered and rejected**: it needs `unsafe` in this workspace to learn strictly less than the
  heartbeat already reports. The trade is stated rather than hidden — the only case that costs
  anything is one where the holder really is alive, and there waiting is correct.
- [M135 — an `edit` no longer hands the model the rest of a partly-read file](milestones/M135-edit-does-not-license-a-whole-file-write.md):
  the F01 guard had a second door. `edit` recorded its success through the same function as `write`,
  whose contract is true of one and false of the other, so **editing one line of a file the model had
  read twenty lines of granted a licence to replace all of it**. A hole written into a milestone's
  `Limits` is a hole nobody measures; the record here is the fix, the six tests, battery `m135b`
  (1185 passed, 0 failed), and the negative control that reproduces the overwrite with the fix
  reverted — [`audit/evidence/m135-negative-control.txt`](../audit/evidence/m135-negative-control.txt).
  The [independent review](../audit/evidence/m135-review.md) earned its keep twice over: it caught
  that control being asserted in four documents before its log existed, and it found that the
  `capped` exemption lets a model that has seen **zero** lines destroy a whole file — a pre-existing
  hole, now pinned by its own test instead of living in nobody's notes.
- [M133 — `serve` mode stops discarding a run it refused](milestones/M133-serve-mode-reports.md):
  the last place a durable-write failure was still silent. The server's boot task did
  `let _ = agent.run(message).await;` with nothing subscribed to the agent's notices, so the error
  and core's own `Notice` both vanished. **The protocol genuinely cannot carry it** — a `Notice` is a
  broadcast event, not a replay log, and the boot task starts before the listener binds — so the
  server's log is the right channel rather than a second one being invented.
- [M131 + M132 — the review's seven findings, and the auth lock that reported a permission error](milestones/M131-M132-review-findings-and-auth-lock.md):
  a footer comment claiming a survival order the code did not have (and the **measurement** that
  contradicted the review's own account of it), one convention implemented twice where an agreement
  test provably cannot see the duplication, two glyph-table entries with no reader anywhere, and a
  lock that reported `Access is denied` for a transient Windows sharing violation — wrong twice over,
  since it is neither a permission failure nor permanent.
- [M130 — the review's four remaining blockers, and the production bug the third one hid](milestones/M130-review-blockers.md):
  the light palette's ANSI-16 quiet roles were the dark palette's `DarkGray` under a comment forbidding
  it — in the mode that is the *default* on a console that does not advertise itself, which is the user
  `--light` exists for; an unrecognised `EPSILON_THEME` fell through to the config file rather than to
  dark; `parse_diff`'s doc contradicted its own pinned test; and the split diff at ≥120 columns had
  never been drawn by a test, which is how the two halves were found landing on top of each other.
  Answers [`audit/evidence/m126-m127-review.md`](../audit/evidence/m126-m127-review.md).
- [M129 — OSC 52 for `/copy`, the glyph fallback that was only promised, and a real card duration](milestones/M129-osc52-and-glyph-fallback.md):
  over SSH, `/copy` printed a success notice while copying nothing; `theme.rs` promised a band-less
  gutter fallback that no branch implemented, so under `NO_COLOR` an answer and its reasoning were the
  same pixels; and every card reported `0ms` because the result stamped its start and end with the
  same `now`.
- [M128 — each reconciliation run gets its own directory](history.md):
  the test that failed once and passed on every re-run had a cause: a **fixed shared path**, so two
  runs at once renamed and deleted each other's fixtures.
- [M127 — a light theme, and a transcript that says when it is frozen](milestones/M127-light-theme-and-frozen-view.md):
  `Color::White` as body text is the one thing this project's own standard forbids, and on a light
  terminal the prose was near-invisible. A selectable light palette (a switch, never background
  sniffing) plus the frozen-view signal: a position badge and a jump-to-live key bound through the same
  table as its hint, so the two cannot disagree.
- [M126 — tool cards, a real diff, and four rounds spent on one assertion](milestones/M126-tool-cards-and-diffs.md):
  a tool call was one flat line and an edit's diff was prose — the artefact a coding agent shows most
  often, rendered unreadable. Cards with a row-budgeted body, unified below a named width and split
  above it, line numbers only above a column budget, and colour never the only signal. The stubborn
  assertion turned out to be the *fixture's* arithmetic, not the product's.
- [M125 — an honest footer, a true context number, and an approval box that fits](milestones/M125-tui-honesty.md):
  the footer advertised `? help`, which is not a bound key at all — it is text the composer parses
  into a command, so the hint named a key that did nothing until the reader pressed it *and* Enter —
  while PageUp/PageDown, history and Ctrl+O went unmentioned. Hints are now resolved from the **live
  binding table**, so a dead hint is not representable. The header showed the last call's prompt size
  as window occupancy — a true number and the wrong one, agreeing on turn 1 and diverging flatteringly
  after — and now shows the session context the compaction trigger itself reads.
- [M124 — the test that hung for ever, and the seam that could not express the fault](history.md):
  the battery stopped on a test that ran past 420 seconds. `set_write_fault_for_tests`'s third
  argument was a **count of failures, not an occurrence**, so it could not say "fail the second
  append"; and since M122 a failed prompt persist ends the run before the provider is entered, so the
  test waited for ever on a watch that could never flip. The seam gained an occurrence, the test now
  asserts its own sequence, and five joins are bounded so a wait that cannot resolve fails instead of
  hanging a round.
- [M123 — the transcript decoration existed, was tested, and had never been drawn](history.md):
  `ui::transcript::decorate` and `row_style` — the two-column gutter, the user marker, the per-role
  glyphs, the filled bands — had **no production caller**, only tests in the same file, while the real
  draw path emitted plain per-kind text through `RenderCache`. The suite reported decoration as
  covered and the user saw none of it. Tests for anything visual now read **cells from the real draw
  path**, the standard M127 and M129 were then held to.
- [M122 — the turn loop stops discarding durable-write failures](history.md): F07 — two store
  mutations threw their results away, so a failed durable write left memory and the model view
  silently divergent: the loop believed a turn was recorded while the transcript the next request is
  built from did not contain it. Both now report, so the user learns the session may be missing a
  turn. Written by a delegated author that never reported, so its inventory of the other discarded
  sites is incomplete rather than exhaustive.
- [M121 — carry pi's thinking signature, and cap `glob`'s output](milestones/M121-thinking-signature-and-glob-cap.md):
  `ContentBlock::Thinking` had no slot for pi's `thinkingSignature`, so 49 occurrences in one fixture
  and 1 in another were dropped on import; it now rides the **one** content-part conversion both
  readers use, which is why the importer needed no change — the payoff of M119 reusing the healing
  path instead of writing a second converter. `glob` gained the 50 KiB cap M118 left it, and decides
  the opposite way from `find` about which bound speaks: when the **walk** stopped, the byte notice's
  claims are false about files it never visited.
- [M120 — reconcile an import against its source, and settle M119's stray number](history.md):
  M119's evidence left one figure unexplained (373 tool results in the source, 391 in the import) and
  said so rather than smoothing it over, which is what made the follow-up possible. A real
  reconciliation proved a fresh import is **entry-for-entry identical** to its source — 1,018 =
  1,018, zero unmatched in either direction — and that the stray figures came from a file `--resume`
  had already repaired: it appends 18 synthetic tool results plus a `TurnEnd` before printing its
  summary. The lesson outlives the number: `--resume` mutates the session it resumes.
- [M119 — import pi's current session files](milestones/M119-pi-v2-import.md): a *current* pi file,
  whose entries carry the `id`/`parentId` envelope pi's own `migrateV1ToV2` adds, was refused at line
  2 — M114/M115 had made only the v1 files readable. `epsilon --import <pi-file>` converts one into
  `<file>.epsilon.jsonl` **beside** the source, leaving the source byte-identical, and the refusal it
  replaces now prints the exact command to run: a refusal a user cannot act on is half a feature. One
  converter, not two — it reuses the v1 healing path, because a second converter is how the two drift
  and one of them starts emptying messages again.
- [M118 — make the advertised bounds real, and announce the silent one](milestones/M118-tool-output-bounds.md):
  one rule applied three times: a tool must not have a bound it does not announce, and must not
  announce a bound it does not have. `find`'s description promised "1000 results or 64KB" and enforced
  neither byte — the cap is now real (50 KiB, pi's own `DEFAULT_MAX_BYTES`) and the description says
  50KB; `glob` stopped at the walker's budget in silence; `ls` gained pi's byte cap. Each notice names
  **only the bound that actually fired**, because sending a reader to raise a limit that is not what
  stopped them is worse than saying nothing.
- [M117 — `find` announces what it withholds, and `ls` gains pi's limit](milestones/M117-find-and-ls-bounds.md):
  the two *defects* inside G20's divergence, separated from the one deliberate choice (this harness's
  active tool set is its own). `find` stopped at its cap and returned a list that looked complete; it
  now appends pi's notice with the count of what is hidden. `ls` had no bound it could report and now
  takes pi's `limit` (default 500) and says so. Both notices are silent when nothing was withheld — a
  notice that fires when nothing is hidden trains the reader to ignore it.
- [M116 — preservation became loud, and a typed field stopped lying](milestones/M116-preserved-notice-and-typed-model-change.md):
  the notice `preserved_unknown()` had always been able to produce was printed by nobody, so a session
  with a foreign entry opened with a quiet gap and no explanation; both frontends now say it in one
  sentence owned by the store. The same round found the `model_change` arm was half-typed: the guard
  required only `modelId`, so a provider-less line healed to `ModelChange { provider: "" }` — an empty
  string in a typed field, which looks like data. Both fields must now be strings, or the line heals to
  `Custom`.
- [M115 — converting an older session keeps its header and its previous bytes](milestones/M115-header-and-backup.md):
  the other two halves of G19. A rewrite used to drop every header field this build does not
  name (`provider`, `modelId`, `thinkingLevel`, `branchedFrom`) — the M113 preservation rule,
  one layer up — and it rewrote the user's file with no way back. Now the header keeps what it
  does not model, and the previous bytes are copied to a `.pre-epsilon` sibling once, never
  overwritten, with the path named in the message.
- [M114 — the healer that read pi's messages and emptied them](milestones/M114-pi-legacy-healing.md):
  the migration path that makes pi's session files readable read `message.content` as a
  string while pi writes an **array of parts**, so every message came back empty, every role
  became a *user* turn, and every ISO timestamp became "now". Found by running it: resuming
  pi's own fixture reported success and turned a 668-character prompt into nothing. Now roles,
  parts and both timestamp spellings are read, and anything unmappable is preserved.
- [M113 — an entry kind this build does not know is kept, not used to close the session](milestones/M113-unknown-kinds-preserved.md):
  a session file carrying a `kind.type` from a newer build used to be **refused whole** — one
  foreign entry cost the entire conversation. It is now parsed as an opaque object, replayed
  on the next write, and left out of the model view: kept, not applied. The rule is lenient
  about the tag and strict about the body, because preserving a *known* kind with an
  unreadable body would open the session with a silent gap instead.
- [M112 — the cost model gets a shape, its dead twin leaves, and the header gets a legend](milestones/M112-cost-shape-and-legend.md):
  `CostBreakdown` returns the four components with the unscaled products private, so the
  total divides once (a refactor had quietly restored the four-truncations bug, and a test
  caught it); a model whose every rate is zero was about to print `$0+` and no longer does;
  the dead `CostMicro` type left `epsilon-contracts`; and `/help` now explains what the
  header's figures — including its `?`, `—` and `+` — actually mean.
- [M111 — the cost estimate prices what it was leaving out](milestones/M111-cost-accuracy.md):
  the header's cost figure added up two of the four disjoint token buckets a provider
  reports, so cache reads and cache writes were charged at **zero** — not cheap, free, in
  the one number a user checks to see what they are spending. The catalog now carries both
  cache prices (regenerated from models.dev), the arithmetic lives in a tested module
  instead of the draw path, and a session that moved tokens the registry cannot price
  reports a floor (`$0.42+`) rather than a total.
- [M110 — the `@` popup went blank, and the search behind it read everything](milestones/M110-at-popup-stall.md):
  the fix for a regression M109 shipped, found by the console probe. `@` completion loaded
  the ignore rules once per keystroke, and loading them searched gitignored trees too — in
  this repository, 404 MB of reference checkouts per character. One- and two-character
  queries worked; five or six showed the composer's *placeholder*, i.e. the app had not
  drawn since before the keys arrived. Fixed by caching the rules in the frontend and by
  pruning the search in the matcher, which makes every tool call cheaper as well.
- [M109 — `@` completion stops offering files the tools would refuse](milestones/M109-at-file-ignore.md):
  the `@` list had its own walker and its own hardcoded skips, so it disagreed with every
  read tool about which files exist. It now shares `epsilon_core::ignore`, and the slice
  found that the hidden-directory "escape hatch" both walks promised was dead logic —
  `is_ignored` cannot tell "re-included" from "never mentioned", so both now say so.
- [M108 — a complete line this build cannot read is refused, not deleted](milestones/M108-unknown-final-line.md):
  the session store dropped a final line it could not parse. That is right for a crash
  fragment and wrong for a *complete* line in an unknown shape — the first is debris, the
  second is the user's newest turn or a newer Epsilon's entry. Position used to decide
  whether data survived; `is_whole_json` decides now, and a refusal leaves the file
  byte-identical.
- [M107 — deleting the second footer a redesign left behind](milestones/M107-dead-footer.md):
  `epsilon-tui/src/footer.rs` was a whole superseded footer with one live function, and
  `clippy -D warnings` is structurally blind to it because `pub` items in a library crate
  are exempt from `dead_code`. It carried a second `format_tokens` with different rounding
  and a second copy of the cache-formula bug below.
- [M106 — the cache-hit gauge measures one prompt, not a session](milestones/M106-cache-hit-accuracy.md):
  the header divided the session-total cache reads by the latest prompt size, so the
  figure inflated every turn and pinned at `cache 100%`. The test named for the right
  property passed because it modelled the gauge wrongly — the numerator now comes from
  the same call as the denominator.
- [M105 — a repair keeps the session's identity, and a pop keeps its cache](milestones/M105-session-repair-integrity.md):
  two silent session-store defects, both reachable only after something has already gone wrong. A
  repair cut a crash-truncated file back to the last *indexed* entry — zero bytes when nothing was
  indexed — while the in-memory header still answered `Some`, so the next append skipped the header it
  believed was there and the session reopened headerless: fresh ids, no cwd, and for a delegated child
  no parent link. Separately, `pop_last` — which the loop calls for real to discard a message that
  overflowed the window — popped the index but not the read-through entry cache, and because the active
  branch resolves its leaf through those reads, the branch came back **empty**.
- [M104 — a restore verifies the file before overwriting it](milestones/M104-restore-verifies-the-file.md)
  *(reconstructed)*: reverting a checkpoint over a file the user edited after the snapshot
  silently discarded their work. Restore now refuses when the bytes on disk are not the
  bytes that were captured, and says which file and why.
- [M103 — the extension host protocol](milestones/M103-extension-host-protocol.md)
  *(reconstructed)*: `epsilon-ext` went from 5 tests to 30 by driving the real fixture
  process instead of a mock, which found a live defect — the frame cap only bounded a line
  whose newline had not arrived yet, so one oversized line came back as valid.
- [M102 — a summary cut off by the output limit is not committed](milestones/M102-length-stopped-summary.md)
  *(reconstructed)*: a compaction summary that hit the model's output ceiling is a truncated
  summary, not a summary; committing it replaced the transcript with a mutilated version of it.
- [M101 — the context files the harness was already detecting](milestones/M101-context-files.md):
  three ways the harness told the user something untrue. `SYSTEM.md` / `APPEND_SYSTEM.md` raised a
  trust prompt and never loaded; `AGENTS.override.md` was missing from the candidate list *and*
  `doctor` kept a second, shorter list of its own; ancestor context files were never read. All
  three fixed, with the important half being semantics rather than names — **one context file per
  directory, first existing wins**, because adding the override to the old loop would have made it
  additive and it would have stopped overriding. The audit was wrong in two places and this
  milestone records both: `.epsilon/prompts/` is slash templates, not system-prompt text, so it
  stays gated and unloaded; and the `skill` tool it said did not exist has existed since M97.
  Negative control: each fix reverted in place, 9 named tests fail. Landed df338b7.
- [M100 — a revert that cannot restore a file must not delete it](milestones/M100-restore-must-not-delete.md)
  *(reconstructed)*: `restore` read "no captured bytes" as "the file did not exist, so remove
  it", which is also what a *skipped* capture looks like. Deleting a file is the worst outcome a
  revert path can produce, and the entry already carried the field that distinguishes the cases.
- [M99 — `.gitignore` is honoured instead of merely promised](milestones/M99-gitignore.md)
  *(reconstructed)*: `find`'s description claimed it and nothing read one. The matcher lives in
  `epsilon-core::ignore` so the tools and the TUI share one answer — one half of which is still
  not true; see the `@`-file note in [`../audit/history/11-current-assessment.md`](../audit/history/11-current-assessment.md).
- [M98 — the six-second stall that read as a hang](milestones/M98-windows-shell-stall.md)
  *(reconstructed)*: every Windows `bash` call ran `powershell -Command`, which loads the user's
  interactive profile first — 6.2 s for a one-word `echo` against 1.06 s with `-NoProfile`. From
  inside the TUI that is indistinguishable from a deadlock, and it is what the owner's "it
  freezes when I send a prompt" actually was.
- [M97 — the `skill` tool the contract always promised](milestones/M97-skill-tool.md)
  *(reconstructed)*: the agent contract told the model to load a skill body "via the `skill`
  tool" and no such tool existed, so the model could see a skill and never open it.
- [M96 — three port gaps the fidelity audit proved](milestones/M96-compaction-and-overflow-fidelity.md)
  *(reconstructed)*: the overflow classifier matched a spaced spelling no provider sends, so the
  compaction-and-retry branch never fired for a real overflow; and the compaction prompt asserted
  a template it never sent, so every summary came back in a different shape.
- [M95 — paged reads](milestones/M95-paged-reads.md): SURFACE-05 — `read` finally pages the way
  its own description promised: pi's offset/limit window over the whole file (`read.ts:275-323`)
  with pi's continuation strings, past-end error, empty-file Ok and the `sed` fallback for an
  over-cap single line, streamed with whole-line boundaries (no U+FFFD cut, no partial line) and
  faithful real line counts. The same change moves the stale-read guard to a coverage model so a
  page of a large file can never grant a wholesale overwrite (F01), and `grep`'s 256 KiB cap is
  announced instead of silent. Focused runs green with a 7-test pre-fix RED; the read-only review
  returned one blocker in the same F01 class (an over-cap line *mid-page* replaced the page and
  recorded `capped`), now fixed — the over-cap branch is gated on the page's first line, with its
  own RED-screened test. Battery `m95b`: 876 passed / 0 failed / 0 ignored; landed 27cd677.
- [M94 — TUI responsiveness](milestones/M94-tui-responsiveness.md): the owner-reported
  unresponsiveness in cmd/pwsh, reproduced in a real console (~30 chars/s echo) and fixed in the
  run loop (drain input before drawing, coalesce agent events, ~30 fps ceiling with a due-frame
  wake, input/approval priority) — measured at ~one frame per burst afterwards; lands the
  in-tree TUI enhancement together with its `config.rs` dependency and the M36 pins.
- [M93 — task state when resuming](milestones/M93-task-state-on-resume.md): F18 — a resumed
  child is claimed `Running` in one lock hold before the background/foreground split, so the
  refusal, the abort cascade and eviction all see it; both run bodies are contained so a panic
  still lands a terminal record. Verified: battery `m93b` 798 passed / 0 failed, two
  negative-control probes (disabling the claim reds seven tests; disabling the containment reds
  both panic tests).

- [M92 — RPC protocol breadth](milestones/M92-rpc-protocol-breadth.md): the RPC-02 slice —
  correlated replies in pi's envelope while keeping the first wire's `ok`/`error` keys, an
  extracted testable dispatch path, `abort`/`get_state`/`set_thinking_level` ops, a lagged frame
  matching serve mode's, `Agent::model_id`, and `POST /abort` + `GET /state` over the shared
  `SessionState` struct with `request_id` echo and OpenAPI registration; verification pending.

- [M91 — review hardening](milestones/M91-review-hardening.md): the follow-ups the M89/M90
  reviews recorded as owed — session-reader guards against hand-corruption (a parent cycle and a
  `seq`/line-order mismatch, neither of which may hang or panic), the three missing CORE-02 loop
  tests (drain-point mirror, overflow-discard pop, mid-run store attach/detach), and a refused-
  with-reason anchor re-base.

- [M90 — session tree](milestones/M90-session-tree.md): the store half of pi's session-tree
  workflows — branch summaries with pi's exact target shapes, explicit no-leaf root navigation,
  `BranchSummary` projected on the active branch, and a fork that writes only the selected
  ancestry with fresh identity; the TUI surface is queued as SESSION-01b.

- [M89 — turn hooks](milestones/M89-turn-hooks.md): pi's `AgentLoopConfig` seams
  (`transform_context`, `convert_to_llm`, `prepare_next_turn`, `should_stop_after_turn`) exist
  as `TurnHooks`; a stop ends the run without draining queues, and a replacement context tracks
  the transcript and yields to a committed compaction.

- [M88 — mid-batch abort](milestones/M88-mid-batch-abort.md): a cancelled run refuses
  batched calls it has not started; one checkpoint covers both batch plans.

- [M87 — admission that reflects reality](milestones/M87-rpc-admission.md): RPC and HTTP
  prompts answer with what actually happened (real rejection, atomic run slot, logged
  in-band refusals).

- [M86 — compaction correctness](milestones/M86-compaction-correctness.md): the durable
  projection replaces the superseded one, and summaries commit only when usable.

- [M85 — follow-up default](milestones/M85-followup-default.md): one-at-a-time
  default restored; explicit All preserved; verification and coverage limits.

- [Fresh audit and port-first plan](../audit/history/09-revalidation.md): named baseline,
  verified gate evidence, unresolved fidelity gaps and independent review reports.

- [M84 — find directory resource](milestones/M84-find-directory-resource.md):
  narrow scheduling correction; F09 containment remains open.

- [M83 — baseline integration](milestones/M83-baseline-integration.md): isolated
  candidate verification, explicit selection and preserved baseline risks.

- [M81 — observation ownership](milestones/M81-observation-ownership.md): F02 implementation,
  test evidence and remaining verification/alias limits.
- [M82 — plan-mode enforcement](milestones/M82-plan-mode-enforcement.md):
  F10 implementation, red/green evidence and scope limits.

## Conventions

- `qa/` files are numbered sequentially and never renumbered; add the next number.
- `milestones/M<N>-<slug>.md` is written once, when the milestone lands, and is a record — it
  is not edited afterwards except to correct a factual error.
- Current product facts have **one owner**: the guides in `user/`. Milestones, research and
  handoffs are historical records — their status claims describe the moment they were written,
  not today. Current defects and verification evidence live in `../audit/`.

Product reference material lives in `user/`. Existing moves, instruction corrections,
verification limits and the Phase 2 review handoff are recorded in
[the restructure record](../audit/history/08-restructure-record.md). Those moves predate this
phase; the verification log is not their move map.
