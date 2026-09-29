# Changelog

All notable changes to Epsilon, grouped by milestone. Most milestones have a dev log in
`docs/milestones/`; where one was not written, the timeline row in `docs/history.md` is the record.

## [Unreleased]

- **Tool commands no longer steal your keystrokes on Windows.** Every command the agent
  ran — a `git` call, a build, an MCP server — inherited the TUI's console stdin, so it
  was reading the same keys you were typing and the screen could be left holding
  whatever the child drew. This was the reported "frozen / shrinking TUI": typed keys
  vanishing while a command ran, and a child competing for the console. Children now
  get a closed stdin and, on Windows, a hidden window. This was the *default* path too,
  not just the unconfined one, and a test now walks every spawn site so a new one
  cannot repeat it (M197).

- **A dead extension no longer freezes the harness for ten seconds per call.** A
  request made after an extension's reader had already stopped could not be answered
  by anything, so it waited out the full invoke timeout and reported a *timeout* — a
  false account, since nothing was slow — and never spent the crash budget, so the
  extension was never disabled either. It now fails immediately, as a crash, which is
  what actually happened (M197).

- **`/tree` is a real tree.** It shows abandoned branches, not just the lane you are
  standing in; folds and unfolds; filters by kind; searches; and takes labels that are
  written through to the session file. Selecting a branch now navigates somewhere real
  instead of looking the entry up in the same lane and silently doing nothing. `k`
  forks at the selected user message (M196).

- **Markdown tables, task lists and strikethrough render properly.** `~~strikethrough~~`
  was printing with its tildes visible: the parser had the feature enabled with no
  match arm behind it. A table too wide for the terminal now degrades to readable
  `a | b` rows instead of one-character columns (M196).

- **A compaction says what it cost.** It used to print a bare "compaction finished"
  while quietly folding the usage into the header's totals, so a long session's cost
  rose with nothing on screen explaining why. The row now names input, output and the
  re-read count, through the same formatter the header uses (M196).

- **`app.message.copy` has a key.** The clipboard path, the OSC-52 fallback and the
  multiplexer notice all existed behind `/copy` with nothing bound to them (M196).

- **A prompt jump key, and a page key that follows the window.** `ctrl+up`/`ctrl+down`
  move to the previous/next prompt rather than by ten rows, and `PageUp`/`PageDown`
  move by the height the list actually drew (M196).

- **The header's cache and cost numbers stopped lying.** The cache rate read
  "measured 0%" for a provider that cannot report cache traffic at all — a confident
  number about nothing. It now distinguishes *unknowable* from *none yet*, prices cost
  per model rather than at the active model's rates, and marks an unpriceable model
  as a lower bound instead of printing a precise-looking wrong figure. The context
  estimate counted neither the system prompt nor the tool schemas, so the compaction
  trigger read a number about 1,600 tokens too small and fired late; a heuristic
  estimate is now marked with `*` so a guess is not dressed as a measurement (M194).

- **`/fork` forks instead of copying the file.** It used to copy the whole session
  file under the same id, while the correct primitive sat wired only to the RPC
  frontend and unused by the TUI (M194).

- **`@`-completion no longer re-walks the workspace on every keystroke.** The ignore
  rules were cached but the traversal was not, so typing one word walked the whole
  tree once per character (M195).

- **Prompt templates.** Type `/review src/parser.rs` and the model receives the body of
  your `review.md` with the argument substituted. Templates live in
  `~/.config/epsilon/prompts/` and `<workspace>/{.agents,.epsilon}/prompts/`; project
  scope wins on a name clash, and `/reload` picks up changes. `/templates` lists what is
  installed with descriptions and argument hints. Supports `$1..$N`, `$@`, `${N:-default}`
  and `${@:N[:L]}`, with `$$` for a literal dollar. See `docs/user/prompt-templates.md`
  (M184, M185).

- **Extensions can participate in a run.** An extension may veto a tool call, rewrite the
  result the model sees, end the run, and choose the next model. Requests to an extension
  are now answered concurrently instead of serialised, so one slow child no longer stalls
  everything behind it. See `docs/user/extensions.md` (M181, M182, M183).

- `/theme` switches the light/dark palette **while the TUI is running**; it was
  previously settable only before launch, via `EPSILON_THEME` or `[tui] theme` (M160).

- `/search` now finds text in the transcript you can see, opens a panel with the match
  position, and **moves the view to the match** — it previously printed the session log's
  hits as notices and changed nothing on screen (M157).

- A tool call is no longer drawn twice — the result settles the call's own card (M155).

- Tool cards now say what the tool actually did — `read src/main.rs:10–50 of 300`
  instead of 400 characters of the file in the header line (M153).

- A tool that is still running now streams its output into its card, so a long build
  no longer looks like a hang (M152).

- The welcome screen no longer repeats the footer's keybindings; it shows the model and
  context window instead (M151).
- The cache metric now reads `R<read> W<write> CH<rate>` instead of `cache N%`, so the rate has
  a denominator and real cache volume is never hidden behind a dash (M151).

- A shell command that exits while a background process still holds its output pipe
  now returns the output it *did* collect. The M148 timeout discarded it entirely, so a
  successful command could report no output at all (M150).
- The Windows confined path (Job Objects) bounds its pipe drain the way bubblewrap and the
  unconfined shell already did, and reports `output_complete` like them (M150).
- The model is now told in the tool text when its output is incomplete, not only the TUI (M150).

- A shell result whose output is known to be short now renders as **partial**, with a warning
  glyph and the word `partial`, instead of a green tick and the word `ok` (M149).

### Fixed

- **A failing test suite could be recorded as a passing check (F05).** The verification record
  decided "did the check pass?" from the tool result's error flag, but a shell command exiting
  nonzero is delivered as a *successful* call carrying `details.exit_code`. `cargo test` with six
  failing tests was evidence of success, so the harness had nothing to say to a model that shipped
  broken code. Verification now reads the exit code, and treats a missing one as unknown rather than
  clean. Ordinary nonzero exits are still delivered to the model as ordinary nonzero exits.


- **A surviving descendant could hang a finished shell command (F11, unconfined).** The command's
  timeout covered waiting for the process but not draining its output, so a background process that
  inherited the pipe kept the call waiting long after the command itself had exited — often
  forever. Draining is now bounded, and a result whose output may be short says so in
  `output_complete` instead of pretending to be whole or failing a command that succeeded.
- **A confined command was not actually bounded (F11, bwrap).** The Linux backend ignored its own
  wall-clock limit, bounded only the direct child's exit and then awaited the pipe drains
  outside that bound — so a descendant holding stdout open could hang past the promised
  timeout — and left the process tree running when a run was cancelled. One deadline now covers
  the wait and both drains, and a cancelled run takes its tree with it.
- **A check could vouch for an edit that had not happened yet.** Ordering was compared by turn
  number, and two calls in one turn share one number, so a check issued before an edit in the same
  batch appeared to cover it.

### Added
- **Epsilon can now read, group and total what its summarizations cost (M144, read side).** Compaction resends
  your whole history and is billed for it, and the usage was discarded even though the response was in scope.
  Charges are keyed by **the model that was actually charged**, never by whichever model the conversation is on,
  and a resumed session folds them into its totals.
  **M145 closed the loop**: the bill is now written, on the compaction entry that incurred it — pi's shape,
  and the reason M144's separate accounting row was reverted. A compaction is a real provider call and its
  cost now shows up in your session's tokens and cost, in exactly the long sessions where you compact.
  The bill carries the **model that was charged**, so it is priced correctly even if you `/model`-switched
  since; a pre-M145 session that recorded a charge with no model is reported slightly low rather than
  guessed at a rate.
- **A summarization bill reaches the token counters, not only the cost breakdown (M144).** Found by M144's own
  test, and the same class of defect the milestone exists to close, one function over.

### Changed
- **The help text stopped promising a variable that does nothing, and told the truth about
  `--plan`.** `RUST_LOG` was listed under `ENVIRONMENT` as a "tracing filter for diagnostics"
  while no crate in the workspace depends on `tracing`/`tracing-subscriber`, none installs a
  subscriber, and no source file reads the variable — so setting it changed nothing, and the
  list of environment variables a user trusts was the wrong place for a dead one. It is
  removed, and a test now fails if it returns without a consumer. The `--plan` line said
  "write/edit denied, git commit/push blocked", which was the pre-M82 ruleset: plan mode
  denies `bash` and `shell` outright (command heads cannot establish a read-only shell
  boundary), so `git commit` is refused as one instance of that rather than by pattern. Both
  are user-visible strings in `epsilon --help`; no behaviour changed.

### Added
- **Reopening a session you just closed now says it is waiting, and how long is left (M138).** M136
  made the lease wait out a crashed holder instead of refusing — correct, but it replaced a clear
  error with up to 30 seconds of nothing on screen, which is the same shape of problem as the
  reported TUI freeze. Startup now redraws one line: `waiting for this session to be released — 3s
  elapsed, ~27s left (another process may still be writing it)`.

### Added
- **`/session` now shows where the money went, not just how much (M143).** One line breaks the session's
  cost into input, output, cache read, and cache write. A single total cannot tell you whether the prompt
  cache is working, and the cache is the main lever most users have on cost. The per-component figures
  were computed and tested but had no caller anywhere — the header showed one number and the machinery
  underneath it did nothing.

### Fixed
- **A session's cost is now priced per model, not at whichever model is selected (M141).**
  models with `/model` silently repriced everything the session had spent so far, and resuming a
  multi-model session did the same to history billed at other prices. The error followed the
  direction of the switch — moving to a pricier model made past work look expensive — and nothing
  on screen showed it. Epsilon now keeps the token counts split by the model that produced them
  and prices each at its own rates. A model the price registry does not know now marks the total
  with `+` rather than being treated as free.
- **The context gauge, cache hit rate and cost no longer count the cached prefix twice (M140).**
  All three numbers were on screen and, on every provider except Anthropic and Mistral, wrong — the
  cache rate read low, the context gauge read high, and the cost was charged at the full input rate
  *and* again at the cache-read rate. On the OpenAI Responses API the cached count was dropped
  entirely, so the header showed a confident `cache 0%` where it should have shown that it did not
  know. Providers count the cached prefix inside the prompt total; Epsilon now subtracts it, as pi
  does, and no longer discards the Responses wire's cache details.
- **Switching models now survives closing the session (M139).** `--model` and a mid-session model
  change were recorded in the session file but read by nobody, so resuming a conversation quietly
  reverted to the launch model. Epsilon now continues on the model the session last used — and
  says so. An explicit `--model` still wins, because a flag is an instruction for this run while the
  session's model is a remembered default.
- **A reply split across several server-sent frames in one network chunk is no longer truncated
  (M137).** When a provider sent a short answer in a single chunk, Epsilon showed only the first
  frame's text and reported the rest as a **successful** completion — no error, no warning. Three
  tests cover it, including the case that fails if the fix only works one way.
- **Reopening a session you just closed no longer fails for 30 seconds (M136).** Kill a TUI, open it
  again, and the harness would report that another instance is already writing the session and exit —
  because the process you just killed is exactly the one that was not heartbeating its lock, and the
  lease refused on first sight instead of waiting to find out. The lock's own heartbeat already
  answers "is anyone still there?" without a platform call, so acquisition now waits out the crash
  window and takes the session the moment the heartbeat stops. A live holder is still never taken
  from: that is the corruption the lease exists to prevent, and it is now pinned by a test that fails
  if the wait ever turns into an instant steal. A pid-liveness probe was considered and rejected — it
  needs a platform call to learn less than the heartbeat already reports. If the refusal does stand
  (a second window genuinely owns the session), the message now says how long it waited and that the
  pid is a hint rather than a verified fact.
- **An `edit` no longer hands the model the rest of a partly-read file (M135).** `edit` recorded
  its own success through the same function `write` uses, and that function's contract — "the bytes
  it just wrote are known in full" — is true of `write` and false of a targeted replacement. It
  *set* the file's "wholly seen" state, so a model that read 20 lines of a 200-line file, edited one
  of them, and then wrote the file was allowed to replace all 200. The refusal that prevents this
  is the one that stops a model overwriting text it has never seen, and it now survives an edit. The
  narrow read that reaches the hole is the caller's own `limit`, not the byte cap, so no large file
  is involved. One message changes in the honest direction: a file the model edited without ever
  reading it is now refused as never read rather than as partly read.

### Added
- **`serve` mode reports a run it refused (M133).** The server's boot task did
  `let _ = agent.run(message).await;` with nothing subscribed to the agent's notices, so when a durable
  write failed and the turn was refused, both the error and core's own `Notice` vanished and the
  operator saw only the serving banner. It now prints on the server's log, with the same
  `serve: admitted run failed:` prefix the `/prompt` route already uses. A client is still told nothing
  *during its own prompt* — a `Notice` is a broadcast event rather than a replay log, and the boot task
  starts before the listener binds — so the failure reaches it as the next request's refusal, a limit
  the helper's own doc comment states rather than implies.
- **A lock lost to a transient sharing violation is retried, not reported as a permission error
  (M132).** `acquire_auth_lock` retried on `AlreadyExists` and returned immediately on every other
  error, including `ACCESS_DENIED` — which on Windows is what the second caller gets while the first is
  mid-delete of the lock path. Two processes saving at once (the CLI and the TUI, the case its own test
  is named for) were told "Access is denied", wrong twice over: it is neither a permission failure nor
  permanent. M131, the review round that preceded it, changed no user-visible behaviour — its findings
  were a footer comment claiming a survival order the code did not have, one rule implemented twice,
  and eight doc defects.
- **The TUI design audit's own items (M125–M130).** The owner's report that the TUI was "extremely
  poor in terms of design and usability compared to the references" traced to decoration that existed,
  was tested, and had **no production caller** — so none of it was ever on screen. Now: the transcript
  gutter, per-role markers and filled bands are drawn on the real path; the footer's key hints are
  resolved from the live binding table (the old row advertised `? help`, which was not a bound key,
  and hid PageUp/PageDown, history and Ctrl+O); the header's context figure is the session context the
  compaction trigger actually reads, not the last call's prompt size; the approval dialog is ~6 rows
  with the danger border its own comment had always claimed. Tool calls are **cards** — a title with
  name, status and duration, and an edit's diff rendered as a diff (gutter, line numbers, per-line
  colour, never colour alone) instead of the literal `"{name} → diff:\n{diff}"` pushed through the
  markdown parser. A **light palette** (`--light`, `EPSILON_THEME=light`, `[tui] theme`), and a
  transcript that **says when it is frozen**: a position badge (`↑ 12/40`) and a jump-to-live key,
  both bound through the live table so neither hint can drift from its handler.
- **`/copy` over SSH (M129).** It used the native clipboard only, so on a remote session it printed a
  success notice while copying nothing. It now also emits OSC 52, native first and terminal second,
  and the notice says **"sent via the terminal"** — OSC 52 is a request a terminal may ignore in
  silence, and claiming otherwise was the defect. Also in M129: `NO_COLOR` glyph separation (an answer
  and its reasoning trace were the same pixels) and a real card duration (every result card reported
  `0ms` because it stamped its start and end with the same `now`).

> **Coverage.** This is a curated record of user-visible changes, not a complete one: M125 forward at
> the top, M11–M82 below, and **nothing for M83–M124** — the audit, port-fidelity and hardening
> rounds. That is a choice rather than a loss. The file stopped being updated at M82 and resumed with
> the M125–M130 entries, while [`docs/history.md`](docs/history.md) already carries a finding-level row
> for every milestone in between and [`docs/milestones/`](docs/milestones) the dev log each row was
> written from. A second-hand summary composed here now would be a third copy of that record, in the
> place a reader trusts most.

### Added
- **Plan-mode enforcement (M82).** `--plan` denied `git commit*` patterns that core never matched
  (preflight authorizes the command head only) and *replaced* project rules, so a project read-deny
  evaporated and bash ran unimpeded. The plan ruleset now denies `bash` and `shell` outright —
  string prefixes cannot establish a read-only shell boundary — `--plan` layers the plan ruleset
  over project rules instead of replacing them, and authorization is additive end to end: the
  original tool capability is checked first, a path-derived external-path subject can only add a
  constraint, and either denial short-circuits, so a `dir:` argument can no longer mask a plan deny
  on shell.
- **Observation ownership (M81).** A read used to authorize a write for *any* agent in the session:
  one shared read-log meant agent B could edit a file it never read because agent A had. Each agent
  now owns an opaque observation token; task children and restored sessions get fresh ones, sibling
  writes invalidate the cached observation even when mtime and length still match, and a failed edit
  no longer refreshes what the next attempt is verified against. Weak references reclaim retired
  scopes on the next log access.
- **Language-server diagnostics on every edit (M79).** After `write` and `edit`, the harness asks the
  language server for the file's language what the compiler thinks and appends a bounded block —
  errors only, at most twenty, `ERROR [line:col] message` — to the tool's own output, so the model
  fixes its own type errors in the same turn. Servers are user-installed (nothing is downloaded), one
  process per project, reused across edits, and a server that refuses to start is never retried. No
  server, no diagnostics, no error: the edit succeeds and nothing is said. `--lsp auto|off`;
  `epsilon doctor` lists the servers it can find.
- **Per-call sandbox modes (M78).** A call can now name its own boundary: `read-only` is always
  allowed (asking for less needs no approval), and `--sandbox read-only` starts a whole session on
  the read-only rung, where commands may look but not touch — the workspace mounts read-only where
  the backend can do that. Asking for a *wider* mode is refused with the reason and the hint, and
  `danger-full-access` is not available per call at all: it is reachable only through the session
  switch a human flips. Both sandbox backends now refuse to be handed "no confinement" and call it a
  boundary.
- **Sandbox truthfulness (M77).** Every confined outcome now reports how completely the boundary
  held (`enforcement: full | partial`, with the unconfined dimensions named), each backend declares
  its own envelope, and `epsilon doctor` prints that declaration instead of a hardcoded string. A
  bash result carries `details.verdict` (`success` / `exited:N` / `signalled`), which keeps "the
  command exited non-zero" and "the sandbox refused" as separate answers — the mislabel that made
  `rg` exiting 1 look like a confinement failure is now impossible by construction.


### Added
- **`epsilon doctor` (M76).** One command that answers "why is this not working": resolved model and
  *how* it was resolved, where the credential comes from (never its value), the sandbox backend the
  platform provides, the session file and its lease holder, project instructions and trust, the
  built-in tool list, and the last failures from the newest session. Exits non-zero when a blocker
  is found and names the command that fixes each one.


### Added
- **Typed corruption reasons (M75).** A session line this build cannot read is no longer reported as
  corruption: an entry kind from a newer Epsilon (`UnknownEntryType`) is refused **by name**, while
  real damage (`NotJson`), a second header and a v1-shape-in-a-current-file each say what they are.
  Session repairs now report their kind (`RepairKind`) rather than a count, the synthesized entry
  carries `details.repair`, and `/session` lists what a resumed session was missing.


### Added
- **`@`-references (M74).** `@file`, `@file:10-20`, `@file:10` and `@dir/` in a prompt now attach the
  content the model needs instead of a path it has to chase. Works in every frontend — TUI, `-p`,
  RPC and the HTTP server — through one constructor. Sensitive paths (`.env`, keys, credential
  stores, VCS internals) and paths outside the workspace are refused with the reason shown in the
  message; the user's own text is never edited, and a prompt with no references is unchanged.


### Added
- **Subdirectory instruction hints (M73).** Working in a subdirectory now pulls that directory's
  `AGENTS.md`/`CLAUDE.md` — and its ancestors' — into the next request's context, labelled by path.
  In a monorepo the crate's own conventions are followed instead of guessed at. Bounded and
  deterministic: ≤5 levels up, the workspace root is not repeated, identical files are deduped by
  content, ≤5 hints per prompt nearest-first, 4 KB each with the cut announced.


### Added
- **Path-aware tool batching (M72).** A batch no longer serializes just because it contains a
  mutating tool: `write`/`edit`/`read`/`grep`/`glob`/`ls` declare the resource they act on, calls
  to *different* files run at the same time, and calls to the *same* file keep the order the model
  asked for. A `bash` or `task` call still serializes the batch — its reach is unknown. Blocked
  calls (denied, invalid, unknown tool) no longer cost the batch its parallelism.


### Added
- **Edit matcher: three more fallback stages, and a refusal that bounds them (M71).** The chain
  grew from three stages to six — escape-normalized (`\n` written as literal text),
  trimmed-boundary (blank lines the file does not have), and block-anchor (openings and closings
  remembered, the middle paraphrased) — ordered safest-first. The one variable-height stage is
  bounded by opencode's rule: a match spanning `max(old+3, old*2)` lines or 500+ characters more
  than `old_string` is refused with the numbers and guidance, and the file is left untouched.
  Every loose match names the stage that matched, so the model can check the diff it gets back.


### Added
- **Tool-call argument repair (M70).** Arguments that are not valid JSON — a trailing comma, a
  payload cut off by the output limit, a double-encoded string, single-quoted JSON — are recovered
  instead of being silently replaced with `{}`. Arguments that cannot be read are handed on verbatim,
  and the tool failure quotes them, so the model sees what it actually sent.
- **Interrupted runs close their own calls (M70).** A tool call left unanswered by an aborted or
  crashed run gets an error result saying it was never executed, before the next request is built.
  Providers validate tool-call pairing; the 400 used to arrive one request after the mistake.
- Duplicate tool-call ids within a message are renamed deterministically (`<id>#2`) — providers
  reject them and the result pairing was ambiguous.

### M69 — cache attribution (and a gate that fails on warnings)
A cache-hit percentage cannot say why it dropped. Port-plan Tier 2 item 4, plus a `scripts/gate.sh`
fix: the clippy step exited 0 with warnings, so one got past the gate.

- **Every call is classified.** `Cold` (the first call of the process), `Served` (answered from
  cache) or re-billed with a named cause: a model switch, a prompt prefix that stopped being
  byte-stable, an idle gap longer than the TTL, or nothing the harness can see (it says so rather
  than blaming a mechanism it did not observe). A provider that reports no cache traffic at all is
  described as silent, never as wasteful.
- **The waste is priced, and only what can be.** Re-billed tokens are billed at the model's list
  input rate — exactly what they cost, from the bundled catalog. Cache reads and writes are
  reported as tokens, not dollars, because their rates are provider-specific multipliers this crate
  does not know; an unknown model gets no price at all.
- **`/session` shows the breakdown**, next to the totals it already printed — including the cause
  grouping, most expensive first.
- **`prompt::stable_fingerprint`** hashes the STABLE tier alone (identity + tool lines), which is
  the quantity ADR-008 promises to hold constant. The volatile tier changing is not a cache break,
  and a test asserts both halves.
- **The gate now runs clippy with `-D warnings`.** The previous gate reported PASS alongside a
  `manual_is_multiple_of` warning in this very milestone's new code.

### M68 — verification evidence and the stop-time nudge
The contract asked the model to run the project's checks after editing, and nothing recorded
whether it did. Port-plan Tier 2 item 5 / defect D-14.

- **Edits and the checks that cover them are now recorded.** Every successful `write` / `edit` is
  logged with the turn it happened in; every `bash` invocation that looks like a check
  (`cargo test`, `npm run build`, `pytest`, `tsc --noEmit`, `make`, `cd x && cargo test`, …) is
  logged with its exit status. A file is unverified when no *successful* check has run since it
  was last changed — "I ran the tests" is not "the tests pass".
- **A run that ends with unverified edits is told once.** At the stop point the harness injects one
  message naming the changed files and what has not run, delivered as an ordinary message so it is
  in the transcript and the session file. Twice at most (`verification_nudges`, default 2); the
  run is never refused. A failing check changes the wording instead of clearing the edit, and a
  `Notice` event carries the same state to a frontend.

### M67 — stuck-run guards
The brakes saw identical *calls*; a stuck run usually looks nothing like that. Port-plan Tier 2
item 7.

- **Identical results now count, whatever the calls were.** `rg`, `grep` and `find` failing the
  same way three times is one piece of information, not three. Results are signatured (error-ness
  plus a normalized head of the text), consecutive identical ones are counted, and the observation
  rides along with the result the model reads — with the wording escalating at twice the
  threshold ("report the blocker … or change strategy"). A stalled run also raises a
  `no_progress` permission request before the next batch, once per batch, the same surface the
  doom-loop brake uses. `no_progress_threshold` (default 3) configures it; `0` disables it.
- **A run can no longer end on an empty turn and call it success.** A finished turn with no text
  and no tool calls gets one nudge message; if the next turn is empty too, the run ends with an
  error naming the cause. Degenerate repetition (one line taking half a long reply) takes the same
  path. `empty_turn_retries` (default 1) sets how many nudges are allowed.

### M66 — content provenance: a boundary the text cannot forge, and user words that survive
Two seams where text is shaped before the model reads it. Port-plan Tier 2 item 8, plus the
never-paraphrase-the-user invariant from the Hermes mining.

- **External content arrives inside a boundary it cannot forge.** Text from `webfetch`, MCP
  servers and extensions is enclosed in `<untrusted_tool_result tool="…">`, and every
  case-variant of the delimiter inside the body is rewritten first — so a page cannot close the
  boundary early or open a fake one. Local tools (`read`, `bash`, `grep`, …) are untouched:
  workspace content is the user's own, and it arrives byte-for-byte.
- **A truncated payload says so.** Providers truncate silently and leave markers that read as
  prose (`"has_more": true`, `… more items`). When one appears in a long untrusted result, a note
  follows the wrapper: the data may be incomplete. Before this, a model would summarize the
  visible slice and report it as the whole dataset.
- **The user's own words are never paraphrased into a summary.** Compaction now carries the user
  turns from the summarized range into the new context verbatim, in a `<user_instructions>` block
  beside the summary — and recovers that block from a previous summary, so instructions survive
  successive compactions instead of evaporating on the second one. "Do NOT touch the parser"
  stays "Do NOT touch the parser".

### M65 — the stale-read guard
`write` and `edit` could destroy work without ever noticing that the file had changed underneath
the model — a `task` child, a background script or the user's own editor all write to the same
files. Port-plan Tier 2 item 6 / defect D-15.

- **A write is now checked against what was read.** Overwriting a file that exists but was never
  read is refused; so is a write or edit to a file that changed since the last read (mtime or
  size), and a wholesale overwrite of a file that was only read in part. Each refusal names the
  path, what changed and the action that lifts it. A successful write records the state it
  produced, so the harness's own change never looks like someone else's. The check runs inside
  the per-path mutation queue, and the registry is shared by the parent and every `task` child —
  which is exactly the case it exists for.
- **The 256 KB read cap is no longer silent.** A large file used to look fully read while the
  model had only seen its start. The read result now says how big the file is and that the rest
  was not sent, and the same flag decides whether a later overwrite is legitimate — a cap the
  model cannot lift is not a reason to refuse, or the guard would trap every file over the cap.
- **The `write`/`edit` tool descriptions state the rule**, so it is predictable rather than
  discovered.

### M64 — the compaction trigger is measured, the tail is legal, and files are remembered
The pressure trigger was a `chars / 4` guess over the whole transcript, the retained tail could
start in the middle of a tool batch, and every summary forgot which files had been read or
changed. Port-plan Tier 1 items 1–3, with mutation-checked tests.

- **Context pressure is anchored to what the provider billed.** Every provider reports the true
  prompt size on every response; the loop now remembers that number with the messages it
  covered and estimates only what was appended since (`compact::anchored_tokens`, pi
  `compaction.ts` parity). Before this, one `chars / 4` figure decided the call for prose
  (over-counted, compacting early and paying for a summary nobody needed) and for code/CJK
  (under-counted, overflowing the window). The anchor is dropped whenever the history it
  describes is rewritten — a compaction or a session switch — so a stale number can never
  describe a different transcript. `Agent::estimated_context_tokens()` exposes the same figure
  the trigger decides on.
- **A retained tail can no longer begin with an orphaned tool result.** The tail budget is
  measured from the end of the transcript, which can land in the middle of a tool batch: the
  result survives into the tail while its call is summarized away, and wires that validate
  tool-call pairing reject the whole request — after the summary was paid for. The cut now
  moves forward to the next turn boundary, or back when the batch runs to the end of the
  transcript (a slightly larger tail beats an invalid one).
- **Summaries remember which files were read and changed.** `FileLedger` reads that out of the
  tool calls themselves — `read` → read, `write`/`edit` → modified — and rides in every summary
  as a harness-maintained `<file_ledger>` block, accumulating across compactions. Before this,
  the model re-opened files it had already seen after every compaction, because the summary it
  was handed depended on the summarizer's recollection rather than on what actually happened.

### M63 — the last mile: a harness someone else can actually use
An audit of the built binary (not the code) found that the engine worked and the on-ramp did
not. Everything below is a behaviour change with a regression test; `audit/` holds the review,
the defect ledger, the port plan and the roadmap.

- **`epsilon --help` exists.** It printed `unknown argument: --help` and exited 2 before. The
  full reference (modes, flags, environment, files, examples) now ships inside the binary, which
  matters most for people who installed it and have no README next to the exe.
- **Headless failures report the real reason.** `-p` printed `no final assistant text produced`
  for *every* non-clean ending and threw away the provider's own error. An expired key, a rate
  limit, a context overflow and a mid-stream reset all looked like an empty answer. Now: the
  reason goes to stderr, partial text is kept on stdout (M62's salvage rule, applied at the CLI),
  exit stays non-zero.
- **`real` is a default feature.** `cargo install epsilon-cli` used to produce a binary that
  could not reach a single real model. Offline-only builds use `--no-default-features`.
- **Model resolution prefers a provider you can call.** `--model deepseek-chat` resolves to an
  OpenRouter row; a user holding only a DeepSeek key got a silent fallback to the offline faux
  provider and a banner naming no provider. Now a callable sibling is preferred (and the
  substitution is reported), and when nothing can run the message names the provider, its env
  var, the login command, the providers you *do* have, and a model to try.
- **Byte-stable prompts are actually byte-stable.** The tool list feeding the STABLE prompt tier
  and every provider `tools` array came out of a `HashMap`, so two runs of one session sent
  different bytes and provider prompt caching could never hit across invocations (ADR-008's
  premise, silently false).
- **Child processes no longer inherit your credentials.** Every spawned child drops
  credential-shaped environment variables (`*KEY*`, `*SECRET*`, `*TOKEN*`, `*PASSWORD*`, …) on
  all three spawn paths; `EPSILON_ENV_PASSTHROUGH` exempts specific names. One `env` in a tool
  result used to put every key in the transcript, the session file and any export.
- **Output truncation keeps the head as well as the tail**, with an exact
  `[... N lines / M bytes omitted]` count — a long compiler log no longer loses its first error.
  Spill files are written exclusively under a private (`0700`) root, and a spill that fails says
  so instead of promising a locator that is not there.
- **A running command is visible.** `Sandbox::run` accepts an output sink, both backends drain
  pipes incrementally, and `bash` throttles chunks into `ToolExecutionUpdate` events (250 ms,
  bounded tail) that the TUI shows in the status line. The complete output still returns with the
  tool result. Previously the sink existed and nobody listened, so a ten-minute build showed a
  static screen.
- **Repository and docs**: added `LICENSE` (MIT was declared but the file was missing), removed
  two build-probe artifacts from the repo root, rewrote the README as an entry point with a table
  of contents, added `docs/README.md` as the documentation map, moved `design/`, `plans/` and
  `reports/` under `docs/` (`docs/research/`), added `scripts/gate.sh` for the whole gate
  battery in one command, and replaced the previous audit's unsupported claims with measured
  ones.
### M62 — a failure never discards visible output
- A stream that dies mid-answer (transport reset, provider error frame, missing terminal frame,
  stall) now **keeps the text and thinking already delivered**, flags the turn `Error` with the
  reason, and drops tool calls whose arguments may be half-written — one salvage rule shared by the
  wire accumulator (`HttpEventStream::failure_message`) and the loop (`Agent::failure_message`).
- Previously the abort path salvaged and every failure path discarded, so a dropped connection could
  erase an answer the user watched arrive — including from the durable session a resume reads back.
### M61 — session write leases
- **One writer per session file, enforced.** The store owns a cross-process lease (M61):
  a second `epsilon` opening the same session is refused with the holder's pid instead of
  silently interleaving appends.
- The lease **heartbeats** (10 s) so a live holder is never mistaken for a crash leftover;
  genuinely stale locks are now actually removed (the steal path had never deleted them).
- `/sessions` marks leased rows `· in use`; re-resuming the session this process already holds works
  (its own lease is released first), and a refused switch keeps the current session attached.
### M60 — retries that run, and a stall watchdog
- **The retry policy is now engaged** on both frontends (direct and fallback streams):
  transient 429/5xx/network failures are retried with the ADR-010 backoff instead of failing the turn.
- **Retry waits are visible**: `AgentEvent::Notice` → TUI status line + `⟳` transcript line, `[retry] …` on CLI stderr.
- **Stall watchdog**: a live stream that produces no bytes for 120 s fails as a network error
  (`EPSILON_STREAM_IDLE_SECS` to tune, 0 = off) instead of hanging the turn forever. Salvage keeps any prose
  and never executes a tool call whose arguments may be half-written.
- **Content-free mid-stream failures are retried** (a stream that dies before any visible delta is re-run
  transparently, with the duplicate `Start` swallowed); failures after visible output still pass through.
- `retry::classify` now recognises "timed out" (reqwest's own wording), not just "timeout".
### M59 — external editor
- **`/editor`** (also `Ctrl+O` / `Alt+E`) opens the current draft in your own editor:
  `[editor] command` in the config, else `$VISUAL`, else `$EDITOR` — the TUI suspends,
  the editor gets the terminal, and the result replaces the draft (one trailing newline trimmed).
- A non-zero exit keeps the draft and says so; an unconfigured editor names both remedies
  instead of doing nothing.
- `epsilon_core::config::load_config_string("<table>", "<key>")` is the new generic config reader.
### M58 — durable child sessions
- **`task_id` survives a restart**: every delegated child now gets its own session file
  (`<parent>-<task-id>.jsonl`, header `parent_session`, a `task_meta` entry written before the
  first turn), so a resume named after the parent's process is gone rehydrates the child's
  transcript, model, and type from disk and continues it.
- **Honest state on restore**: the snapshot's state comes from the file's last `TurnEnd`
  (`Failed` for error/abort/budget/blocked), output is labelled `[restored from disk]`, turns
  accumulate across the restart, and a child whose model retired restores on the inherited one
  with a note.
- `/sessions` marks those rows `· task child` (M57 browser).
- No store attached (tests, embedding callers) means children stay in memory exactly as before.
### M57 — session discovery + the `/sessions` browser
- **`/sessions`** (and bare `/resume`) opens a browser over the workspace's session files:
  newest first, the attached session marked, each row `title · age · size · file`; typing filters,
  Up/Down move, Enter resumes, Esc closes.
- **Discovery moved to the session brick**: `epsilon_session::{list_sessions, latest_session,
  summarize}` (bounded head/tail reads — listing cost is independent of session length) now back
  both the browser and the CLI's `--continue` scan, so "which files here are sessions" has one answer.
- **One switch path**: `/resume <path>`, `/import`, and the browser all call
  `App::switch_session_file` (store swap + transcript rehydrate + session-name rehydration).
### M56 — per-delegation model override
- **`task` takes a `model`**: delegate a child onto a chosen catalog model (e.g.
  `deepseek-flash`) — cheap scouts, mechanical sweeps — instead of always inheriting the
  session's model. Precedence: the agent type's own model wins (operator config), then the
  call's request, then inheritance.
- **No silent substitution**: an unknown or partial-only model id is refused before any child
  is built (a partial match is offered as a suggestion), and every report — tool envelope,
  `details`, `/tasks`, and the background completion notice — names the model that actually ran.
### M55 — GitHub Copilot: wire, catalog rows, device login
- **`github-copilot` is a real provider now.** `CopilotWire` wraps the family wire
  (`claude-*` → Anthropic Messages, everything else → OpenAI-compatible), sends the
  bearer on both dialects, and resolves its host from the credential (`proxy-ep` →
  `api.…`, else `copilot-api.<enterprise>`, else the individual default) — the reason
  `ProviderWire` grew `endpoint_for(model, key)`.
- **27 catalog rows** generated from models.dev's `github-copilot` registry (`github-copilot/…`
  ids, 8 Claude rows on the Messages wire), with `-picker` aliases excluded.
- **Login**: `epsilon auth login github-copilot --oauth` runs GitHub's device flow, then mints
  the Copilot bearer from the GitHub token — nothing unminted is ever stored. Enterprise
  hosts come from `[oauth.github-copilot] enterprise_domain` and ride the credential.
- **Refresh**: a third style (`RefreshStyle::CopilotMint`) — GET with editor headers, no client
  id, no body, no rotation; the stored GitHub token is re-spent until it is revoked.
- **Drift absorbed** by the same refresh: bedrock ids are canonical (`bedrock/…`, region/version
  qualifiers stripped — M49's hand rows are gone), and two catalog pins moved with the registry.
### M54 — task abort cascade + `/tasks`
- **Esc stops the children too**: aborting a run now cancels every delegated
  child still working (`Agent::abort_delegated_tasks`), instead of leaving
  background subagents spending tokens after you pressed stop. Aborted
  children record *"subagent run aborted"* rather than an opaque stop reason.
- **`/tasks`**: the TUI lists what the registry is holding — task id, state,
  description, turns taken, first output line — or explains how tasks appear
  when there are none.

### M53 — `task` background runs + resumption
- **Background subagents**: `task` accepts `background: true`, returns the task
  id immediately, and delivers the child's result later as its own message —
  the model keeps working while the child runs. The notice is emitted as a
  normal message event, so the transcript shows what the model saw.
- **Resumable children**: pass `task_id` to continue the *same* child (it
  remembers its earlier turns; turn counts accumulate), or with no prompt to
  get a status report — no more silent "started fresh".
- **Honest failures**: a child that dies on a provider error lands in the
  registry as `failed`, not as a completed task; unknown ids name what is
  tracked. The registry is bounded (16 tasks, oldest finished evicted).

### M52 — OAuth login ceremonies + real-feature forwarding fix
- **Subscription login works**: `epsilon auth login anthropic --oauth` (Claude
  Pro/Max), `openrouter --oauth` (PKCE → permanent key) and `xai --oauth`
  (RFC 8628 device code, SuperGrok/X Premium) run the provider's own ceremony
  and store the credential in `~/.config/epsilon/auth.json`, where the M51
  refresh path keeps it alive. `--paste` for a browser on another machine.
- **TUI parity**: `/login <provider> oauth` runs the ceremony on a worker
  thread — browser round-trip, device code and paste overlay, Esc cancels.
- **Client ids are yours**: `<PROVIDER>_OAUTH_CLIENT_ID` or
  `[oauth.<provider>] client_id` in the global config; Epsilon never ships
  another project's registration.
- **Fix — `--features real` now means real**: the CLI's feature enabled only
  `epsilon-llm/real`, so `epsilon-core`'s live provider hot-swap compiled to a
  `false` stub in every installed build (M32d's `/login` "now live" path could
  not work). All three crates forward now, and a compile-time guard turns a
  future omission into a build error.

> M32–M51 landed as commits `e200815` and the working tree; their summaries
> live in [`docs/milestones/`](docs/milestones) (M33 forward).

### M31 — Reference gap closure (G-M1..G-M5)
- **Slash-navigable sessions**: `/compact` reclaims context on demand (complements 80%-threshold auto-compact), `/timeline` lists `branch_entries()` with lane and snippet and jumps via `navigate_to`, `/fork` copies JSONL to `*-fork-<id>.jsonl` preserving the tree.
- **MEMORY.md tier**: global `~/.config/epsilon/MEMORY.md` plus workspace `MEMORY.md` injected into the CONTEXT prompt tier alongside `AGENTS.md`, hash-tagged for cache stability (Hermes pattern, hash-tested).
- **Wildcard parity**: `wildcards.rs` now matches opencode `wildcard.ts` (`*` → `.*`, `?` → `.`, `\` → `/`, regex-metas escaped, `**` collapsed) via the `regex` crate, with backslash-normalized tests.
- **webfetch deferred honestly**: rule already in `epsilon-safety` (`webfetch → Ask`), tool ships in the next milestone (plan 010); `docs/design/epsilon-architecture.md` now says so instead of promising it.

### M30 — Todo tool + checklist rendering; edit diff preview + TUI unified diff
- **Todo tool** closes the agent-contract gap: the model keeps a typed checklist the TUI renders inline.
- **Edit diff preview**: `details.diff` on tool results plus a TUI-expandable unified diff (opencode `diff.ts` pattern).

### M29 — Steering queue badge + drain-on-abort restore
- Steering queue visibility badge and drain-on-abort restore-to-editor polish for the TUI.

### M28 - Custom OpenAI-compatible providers
- **`[provider_custom.<name>]` in the global config** declares an endpoint
  (`base_url`, optional `env_key`); `<name>/<model>` ids then stream to it on
  the existing OpenAI-compat wire — LM Studio, vLLM, llama.cpp server, fresh
  cloud providers (opencode `baseURL` / Hermes `base_url_override` pattern).
- **Keys resolve like built-ins**: stored `epsilon auth login <name>` first,
  then the row's env var; no key is fine for local servers. Fallback chains
  and the startup key-source diagnostic understand customs. Catalog ids
  always win over same-named custom rows.

### M27 - Weekly catalog-drift CI job
- **Monday regeneration against live models.dev**: a scheduled workflow runs
  `scripts/regen_catalog.py`, formats, and opens/refreshes ONE pull request
  (`catalog-drift/latest`) when the bundled catalog drifted. Silent when it
  did not. Never pushes main; the refresh rides the normal CI gates like any
  change. Manual fire available via workflow_dispatch.
- **Audit follow-up (same milestone)**: F2 favorite cycling is now gated by
  the M22 project-config trust review, like Ctrl+P already was — since M26 a
  cycle also mutates the thinking level and persists config, so it must not
  act against unreviewed rules. Gate-blocked regression test added.

### M26 - Per-favorite thinking levels
- **Ctrl+F remembers the level**: starring a model stores your current
  thinking level for it (`[picker.favorite_thinking]` in the global config);
  unstarring drops the memory. Star notice names what it remembered.
- **F2 / Shift+F2 apply it** (pi scoped-models semantics): a remembered level
  overrides the session level on cycle; favorites without a memory inherit.
  Memory is mirrored on `App` (loaded at startup, refreshed on toggle) so
  cycling never touches disk mid-keypress.

### M25 - Footer thinking-level indicator
- **`think:<level>` in the status bar** (pi pattern): renders between cost and
  model whenever extended thinking is active, hidden while Off. `/help`
  updated to match.
- **Displayed state = executed state**: the level is mirrored into one `App`
  field written only by `set_thinking` (the same choke point that updates the
  agent); a key-path test asserts mirror == agent after Ctrl+T cycles.

### M24 - MCP health probes + fallback chains (DIRECTION-02/03 closed)
- **MCP health probes**: `Connector::is_healthy` (stdio detects a dead child
  via rmcp); the bridge probes at most once per 30s TTL before each call, so
  a dead server fails fast with "process is not running... re-attach with
  --mcp" instead of hanging or erroring opaquely mid-call.
- **Provider fallback chains**: repeatable `--fallback <model>` composes
  `epsilon_llm::fallback::FallbackProvider(primary, candidates...)`.
  Retryable failures (rate limit / transient 5xx / network) walk the chain;
  auth failures stop it. Per-turn primary restoration: every call starts at
  the head, so a recovered primary is used again automatically.

### M23 - Picker favorites + favorite cycling (DIRECTION-06)
- **Star any model** with Ctrl+F inside the picker (opencode's
  `model_favorite_toggle`); ★ column in both picker stages; persisted in
  config `[picker] favorite` (cap 16, loud error when full).
- **Favorites section above Recents** in stage-1 rows, recents filtered to
  exclude favorited ids (opencode dialog order); star follows the id into the
  full catalog list too.
- **F2 / Shift+F2** cycle favorites forward/backward from the editor (pi
  `cycleModel` semantics: wrap-around, non-listed current enters forward,
  every switch persists + updates recents). Empty list gives guidance.
- Live force-refresh moved from Ctrl+F to Ctrl+G (auto-fetch on open already
  covers the common case). Hint bar updated.
- Suite: 239 tests passing.

### M22 - opencode auth store + trust re-prompt (DIRECTION-01)
- **`epsilon auth` subcommand** (opencode auth.json port): `login <provider>
  [key]`, `list` (masked), `logout <provider>`. Keys live in
  `~/.config/epsilon/auth.json` in opencode's exact schema; 0600 on unix.
- **Precedence**: stored key wins, env var falls back (`catalog::resolve_api_key`);
  auto-real detection, picker auth column, live `/models` fetching, and request
  auth all follow one resolver. Store lives in epsilon-llm (pi keeps provider
  auth inside `packages/ai`; dependency law forbids llm -> core).
- **Trust re-prompt** (pi trust-manager semantics): `load_project_config`
  returns an explicit `TrustDecision`; the TUI holds all submits on a changed
  `.epsilon/config.toml` until you type `trust` (accept) or `trust no` (quit).
  Headless modes keep the banner policy; the old hash persists until accepted.
- Suite: 234 tests passing.

### M21 - Thinking fidelity sweep (DIRECTION-04)
- **Shared ladder** (`epsilon-contracts::ThinkingLevel`): off/minimal/low/
  medium/high/xhigh/max with cycle + parse semantics ported from pi.
- **Per-wire native mapping**: Anthropic adaptive thinking + `output_config.effort`;
  OpenAI-compat family dispatch (gpt/o-series + kimi `reasoning_effort`,
  OpenRouter nested `reasoning`, zai GLM `thinking{clear_thinking}`, deepseek
  `thinking{enabled}`); Google `generationConfig.thinkingConfig` with
  `includeThoughts` + level. `Off` sends nothing on every wire. Fixture tests
  pin all three request shapes.
- **Session state**: `Agent::set_thinking_level`/`thinking_level`; the level
  rides every run like the model id; compaction summarizer stays Off (ADR-007).
- **TUI surface**: `/think <level>` (bare = report, unknown = inline correction)
  and Ctrl+T to cycle the ladder with an immediate notice.
- Suite: 226 tests passing.

### M20 - Catalog regeneration + live picker refresh
- **Generated catalog** (DIRECTION-05): `scripts/regen_catalog.py` fetches
  models.dev and writes `epsilon-llm/src/catalog/catalog_data.rs` — 383 rows
  across the nine direct-API providers (was ~40 hand-curated), with auditable
  selection rules (tool-calling text models, dated snapshots excluded,
  first-provider-wins dedupe) and registry-faithful prices.
- **Live fetch fixed**: M17's Ctrl+F passed a MODEL ID where `live_models`
  requires a PROVIDER name, so live listing never worked; the contract is now
  honored at the single shared call path.
- **Auto-refresh picker** (opencode/Hermes convention): opening `/models` (or
  Ctrl+M) and descending into a provider auto-fetch that provider's live list
  once per session; Ctrl+F force-refreshed at the time (since M23 that key
  stars a favorite and Ctrl+G refreshes). Live ids merge into both
  picker stages under their own provider group, no duplicates against bundled
  rows.
- **Clippy zero under `--features real --all-targets` too**: M19's fixture
  visibility widening had left 11 warnings in an ungated lane; docs added on
  wire structs, collapsible ifs collapsed, `Usage` initializer-built.
- Suite: 221 tests passing.

### M19 - Audit remediation: all 8 improvement plans executed
- **Provider stop-reason fidelity** (plan 001): OpenAI `[DONE]` maps the recorded
  `finish_reason` (`length` → MaxTokens, `content_filter` → Error) instead of always
  EndTurn; Anthropic `message_stop` consumes `message_delta`'s stop_reason; Gemini
  early-return now accumulates parts before the finish flag instead of dropping them.
- **Loop integrity** (plan 002): overflow-retry latch resets per run (recovery works
  more than once per process); drained-but-uninjected steering messages requeue on
  abort/budget/error exits; compaction clipping and `/search` snippets are
  char-boundary safe (CJK/emoji no longer panic or mojibake).
- **Serve-mode trust boundary** (plan 003): per-boot bearer token required on every
  route except `/healthz` + `/openapi.json`; loopback Host allow-list + cross-origin
  Origin rejection; unattended runs default to deny-with-guidance, `--yes` opts into
  auto-approve with a warning.
- **Secret-leak closure** (plan 004): engine deny evaluated BEFORE session-memory
  promotions; grep skips secret-class files (`.env*`, credentials, `.npmrc`, ...);
  gate subjects lexically normalized so `.env`/`./.env`/`src/../.env` match one rule.
- **Wire fixture harness** (plan 005): `tests/wire_fixtures.rs` drives all three
  adapters with raw JSON frames; CI gained a real-feature TEST lane.
- **Byte-safe SSE buffer** (plan 006): line reassembly is raw bytes; multi-byte chars
  split across TCP chunks survive.
- **Extension event forwarding wired** (plan 007): host evaluates its manifest's
  `events` filter at send time (hardcoded early-return had made it dead code).
- **CLI composition cleanup** (plan 008): `RunParams`/`SharedRun` structs replace the
  11-param composition; single Diagnostics sink; reasoned targeted dead_code allow;
  unused manifest deps dropped; AGENTS.md stale test count fixed.
- Suite: 220 tests passing. Plans live in `docs/plans/` with the audit report at
  `docs/research/re-audit-2026-08-24.md`.

### M18 - Alignment pass: docs audit, pi parity closure, plan mode
- **Documentation audit**: M14 dev log backfilled from commit evidence
  (`docs/milestones/M14-harness-maturity.md`); beginner primer added
  (`docs/qa/06-agent-harness-primer.md`); stale genai-era claims purged from README,
  architecture doc, ADRs, and qa notes; Rust requirement corrected to >= 1.88;
  architecture-doc overclaims (edit mutation queue, tracing/OTLP) corrected to
  shipped reality.
- **Session search** (pi `search/scanning.ts` parity): case-insensitive scan over
  user/assistant/tool-result text with snippets, newest-first, limit and
  active-branch options; `/search <query>` TUI command; re-exported through
  epsilon-core so the TUI keeps zero direct session dependency. 7 new tests.
- **Plan mode** (opencode `plan` agent import): `--plan` runs deny write/edit
  outright plus git commit/push commands; monotonic-deny makes the boundary hold
  against project rules; startup banner states the posture. Live-verified against
  the faux provider including a control run.
- Reports: `docs/research/pi-alignment-2026-08.md` (layer-by-layer pi correspondence),
  `docs/research/research-pass-2-2026-08.md` (community signal + import decisions).

### M17 - pi-agent provider + TUI port (full attribution)
- **Provider layer rewritten as a faithful pi port**: direct per-provider HTTP
  (Anthropic Messages, OpenAI-compatible `/chat/completions`, Google
  `streamGenerateContent`) replaces the genai router crate entirely.
  Ported from pi-0.84.2 `packages/ai/src/api/` and `packages/ai/src/providers/`;
  SSE decoding follows pi's `iterateAnthropicEvents` wire handling; request shapes,
  auth headers (`x-api-key`+`anthropic-version`, `Bearer`, `x-goog-api-key`), usage
  extraction (`message_start`/`message_delta`, `include_usage`, `usageMetadata`),
  tool-call assembly (indexed OpenAI deltas, `input_json_delta` buffering) all cite
  their pi origin in doc comments. New dep: `reqwest =0.13.4` (already in-tree via
  rmcp); `genai` removed from the workspace.
- **Catalog v2** regenerated from a live models.dev fetch (2026-08): current model
  generations across anthropic/openai/deepseek/google/xai/groq/moonshot/z.ai/
  openrouter/ollama with REAL published prices replacing M3 placeholders — footer
  cost is now an estimate against public list prices. Each row carries a `wire`
  column (pi's `model.api`): routing reads capability data, never id substrings.
  Ids are provider-native (`kimi-k2.6`, `glm-5`, `grok-4.6`, `gpt-5.4`,
  `claude-opus-5`, `gemini-3.6-flash`).
- **Live listing** (`real` builds) now plain authenticated GET `/models` per provider
  (pi/opencode convention), replacing genai's resolver.
- **Slash autocomplete rebuilt on pi's pattern**: arrow-key navigable popup with
  descriptions (pi `AutocompleteItem`+`SelectList`), Up/Down move the highlight
  (history recall suspended while open), Enter applies-and-runs the highlighted
  command (pi applyCompletion→submit fall-through), Tab completes the highlight,
  Esc dismisses before quit/abort. Popup state tracks the buffer — it never lingers
  over unrelated text.
- **Keybind parity sweep** (opencode keybind.ts / pi keybindings.ts defaults):
  Ctrl+A/E line home/end, Ctrl+B/F char left/right, Alt+B/F and Alt+←/→ word jumps,
  Alt+Backspace word kill, Ctrl+D forward delete, Ctrl+K kill-to-line-end (kill-ring
  integrated), Ctrl+. redo (new redo rail in undo.rs; divergent push clears it).
  (An "F2 cycles recents" claim in this entry was aspirational and never shipped;
  F2 arrived in M23 cycling favorites.)
- Tests: epsilon-tui 79 green (12 new key-path regressions), epsilon-llm 24 green
  under `--features real` (SSE decoder fixtures, catalog routing/auth invariants,
  current-generation guards).

### M16c - Picker/palette parity (opencode patterns)
- Two-stage model picker: providers first (auth + active marks, model counts),
  Enter descends into that provider's models with live fuzzy filtering;
  Esc walks back stage-by-stage; Ctrl+F live-fetch retained
- Command palette (Ctrl+P): fuzzy-filtered list of all commands with keybinding
  hints, dispatched through the same parse path as typed slashes
- Draft stash: opening any dialog parks the in-progress draft and restores it on
  close (opencode prompt.stash)
- Fuzzy matcher ported from pi's fuzzy.ts (word-boundary/consecutive scoring),
  zero new dependencies
- Editor undo (Ctrl+Z): bounded snapshot stack restoring buffer + cursor before
  backspaces and kills (pi undo-stack.ts parity)

### M16b - Editor v2 (pi editor behaviors)
- Emacs kill-ring: Ctrl+U/Ctrl+W push kills, Ctrl+Y yanks; consecutive kills
  accumulate into one entry; repeated Ctrl+Y cycles entries yank-pop style
- Atomic paste markers: bracketed pastes become `[paste #N +L lines]` in the
  editor, expand to full text on submit; Ctrl+W removes a marker whole
- Multi-line buffer: Ctrl+J inserts a newline (reliable on legacy Windows
  consoles), Enter submits the whole draft; caret tracks line/column
- Persistent prompt history: `~/.config/epsilon/history` (JSON-lines, capped 500,
  corrupt-line tolerant), seeded into Up/Down recall across restarts

### M16a - Rendering efficiency + footer parity
- Virtualized transcript: only visible rows materialized per draw; markdown+wrap
  happens once per line in an id-keyed render cache (draws ~30–69× faster and
  near-flat as sessions grow — bench via new `--tui-bench <n>` mode)
- Picker rows cached per filter; navigation no longer re-filters on every keypress
- pi footer parity: cwd (~-shortened) + git branch, compact token format (k/M),
  context-window usage with `?` right after compaction, cache-hit rate (CH%),
  session cost; threshold coloring >70%/90%
- Own unicode-width word-wrap replacing Paragraph::wrap (style-preserving,
  hard-splits oversized words, CJK-correct)

### M15 - TUI usability overhaul (pi/opencode patterns)
- Picker viewport fix: centered scroll window (pi select-list) + position indicator —
  long lists were previously unreachable past the box height
- Editor: Ctrl+←/→ word jumps, Home/End, Delete, Ctrl+U/Ctrl+W kills (unicode-aware)
- Markdown rendering for assistant messages via pulldown-cmark (headings, bold/italic,
  inline code, fenced blocks, lists, quotes)
- Braille spinner + elapsed seconds while running; contextual key-hint bar
- Mouse wheel scrolling; bracketed paste support; slash-command autocomplete (Tab)
- Visual prefixes (❯ user, ✓/✗ tool results); central theme palette

### M13 - Polish
- Live per-provider model listing in `/models` (Ctrl+F; genai resolver behind `real`)
- Persisted recently-used models (`[picker] recent`, shown as a picker group)
- Markdown-lite transcript rendering (fenced code blocks styled, inline markers stripped)
- Expandable tool cards: `x` toggles full output vs truncated head
- bwrap integration tests (`#[cfg(unix)]`) — runtime verification path for Linux CI
- Fixed latent `--features real` compile break in the CLI (GenaiAdapter re-export)


### M12 - Completion pass
- Project config layer: `.epsilon/config.toml` ([model], [[rules]], [[extensions]],
  [[mcp_servers]]) with SHA-256 trust hash + change banner (completes ADR-012 core)
- Reliability: `Retrying` StreamFn executor (pre-stream failures, classified, jittered
  backoff, on_retry hook); run-scoped cancellation reaching both shell paths (Esc aborts);
  session cost in the TUI footer
- `--resume <path>` / `-c/--continue` (newest session file; shadowed-view restore)
- rpc(stdio) mode (`--rpc`) - NDJSON ops in, AgentEvent frames out
- `GET /openapi.json` on serve mode (hand-maintained 3.1 spec + drift-guard test)

### M11 - Model picker, catalog, runtime wiring
- pi-style /models TUI picker (filter, provider groups, custom ids) + /model, /help, --list-models
- Bundled catalog: 18 models / 10 providers with env-key auth hints
- [model] id persistence in ~/.config/epsilon/config.toml (first ADR-012 slice)
- Skills + extensions now load from conventional directories (closes M8 reachability debt)

## [0.1.0] - 2026-08-23

The first complete harness: pi's proven loop and session format, DSH-grade two-layer
safety, Hermes-grade prompt discipline, opencode-grade manners — one static binary,
three frontends, offline-testable end to end.

### M0–M2 · Foundation
- Docs baseline, reference audit (pi / DSH / Hermes / opencode), 17 ADRs.
- 11-crate workspace with compile-enforced dependency direction; quality gates green
  from commit one (`fmt`, zero-warning `clippy`, offline tests).
- `epsilon-contracts`: pi's type vocabulary ported — messages, stream events, agent
  events, tool descriptors, sticky turn-end reasons; types only, zero logic.

### M3 · Provider layer
- Never-fail `StreamFn` contract: provider failures arrive in-band as
  `Done{stop_reason: Error}`; a `PanicGuard` converts even panics.
- Scripted faux provider (offline, deterministic, free) + genai 0.6 adapter for real
  providers behind the `real` feature; retry policy keyed by error kind.

### M4 · Sessions
- Append-only JSONL tree store with shadowed model view, cross-process turn leases
  (30s steal), crash repair (synthetic tool results, typed `TurnEnd{Aborted}` markers).

### M5 · The loop
- pi's two-level loop verbatim: steering inner / follow-up outer, explicit queue modes.
- Named improvements: semaphore-bounded parallel tool pool with model-order commits;
  fail-all-tool-calls on truncation; doom-loop brake at 3 identical calls.

### M6 · Real tools + permissions
- read/write/edit/bash/grep/glob/ls against the declared workspace; path-escape
  downgrades to an `external_dir` capability that asks before running.
- Last-match-wins wildcard rulesets (defaults ← global ← project) with ask/deny/allow,
  once/session memory, deny-with-guidance fed back to the model.

### M7 · Compaction + prompts
- Auto-compaction past the token threshold: prune old tool outputs → clamp tail →
  isolated summarizer call → amend prior summaries → overflow retry once per run.
- Three-tier byte-stable prompt assembly (STABLE/CONTEXT/VOLATILE), hash-tested.

### M8 · Skills, extensions, MCP bridge
- Agent-Skills-spec catalog in CONTEXT tier, bodies loaded on demand.
- Process-isolated extension host over NDJSON-RPC with timeouts, crash budget, and
  manifest-filtered event forwarding.
- MCP tools bridged into the normal registry under `mcp__<server>__<tool>`.

### M9 · Frontends
- **M9a**: extension event forwarding (closes known-gap #1); print mode `-p`.
- **M9b**: ratatui TUI (`--tui`) — live streaming transcript, approval dialogs, steering
  from the input box; serve mode (`--serve`) — axum HTTP/SSE with prompt/steer/approve
  REST; rmcp stdio transport verified against a real spawned MCP server binary.

### M10 · Hardening & release
- **M10a**: OS confinement seam (ADR-005 Layer 2): Windows Job Objects with
  memory/active-process/CPU-rate caps, kill-on-close, breakaway refused; Unix bubblewrap
  backend; fail-closed policy selection (`Auto|Off|Custom`, `--sandbox` flag);
  `[confinement]` denial signatures distinguish blocks from failures. Permission events
  now carry correlatable request ids. Windows ACL restricted-token runner deferred with
  rationale (ADR-005 amendment).
- **M10b** (this release): real-provider wiring via `--provider real --model <id>`
  (genai auth from env), configurable model id plumbed through prompt tiers and
  compaction summarizer, release profile tuned, binary 8.8 MB (<15 MB criterion),
  changelog + `v0.1.0` tag.

### Verification posture
104 offline tests, zero clippy warnings, byte-stable prompts hash-tested, full-loop
scenarios run against scripted providers — CI never touches paid APIs.

