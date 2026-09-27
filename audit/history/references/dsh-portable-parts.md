# DSH → Epsilon port candidates

Source mined: `references/deepseek-harness-dsh-v0.1.1-rc.2/` (TypeScript, v0.1.1-rc.2).
Target: the Epsilon Rust workspace (`crates/epsilon-*`).
Read-only audit. Every anchor below was opened; nothing here is inferred from docs alone.
Already-ported DSH ideas (fail-closed confinement, bounded tool pools, sticky turn reasons,
process isolation) are excluded by construction.

## Verdict

DSH is a broader and, at the mechanism level, a more *typed* system: it separates facts that
Epsilon currently conflates. Its sandbox distinguishes "the runner failed" from "confinement
denied the command" from "the command exited nonzero" using a three-part predicate, and reports
enforcement completeness (`full`/`partial`) rather than a boolean. Epsilon's `SandboxError` is one
flat enum and its `Sandbox` trait has no enforcement fact at all, so a partial-ABI host looks
identical to a fully confined one. DSH also closes two holes Epsilon has open today: **child
processes inherit the full ambient environment** (no credential scrub anywhere in
`crates/epsilon-safety` or `crates/epsilon-tools`), and spill files land at a predictable,
world-readable, non-exclusively-opened path. Against that, Epsilon is ahead on loop-level
robustness (never-fail StreamFn, fail-all-on-truncation, cross-process leases, crash repair) —
DSH's loop is plugin-parameterised where Epsilon's is one typed spine. The gap is not philosophy,
it is a dozen small mechanisms; most are S-effort.

## Port candidates

| # | What DSH does | DSH anchor (path:line) | Why it matters | Effort | Risk |
|---|---|---|---|---|---|
| 1 | Scrubs `*KEY*`/`*SECRET*`/`*TOKEN*`/`*PASSWORD*` from every spawned child's env | `packages/subprocess/subprocess/src/index.ts:60` · `docs/defensive-patterns.md:29` | Epsilon spawns bash with inherited env. One `env` / `printenv` call in a tool result puts the operator's API keys in the transcript, the JSONL log, and any telemetry. | S | Low — drop vars; a legit use that needs one gets an explicit passthrough list |
| 2 | Spill: private 0700 root, `sha256(sessionId)` subdir, random-prefix name, `open(path,'wx',0o600)` | `packages/spill/spill-local/src/store.ts:27`, `:107` | Epsilon writes `.epsilon/spill/<stem>-<millis>.out` with `let _ = fs::write` — predictable name + no mode bits + silently ignored failure invites symlink planting and hides "the full output was never saved". | S | Low |
| 3 | Byte-exact retention with `head`/`tail`/`headTail`, UTF-8 boundary trim, `Omitted{none,exact,unknown}` | `packages/util/output-retention/src/index.ts:247`, `:390` · `docs/subsystems/spill.md:85` | Epsilon is tail-only line-count math. A 500-line compiler error loses the *header* (the actual diagnostic); the model keeps the bottom of a stack trace. DSH keeps a stable prefix + suffix and reports exact omitted bytes. | S | Low |
| 4 | Sandbox reports `enforcement: 'full'\|'partial'`; callers needing an absolute boundary must reject `partial` | `packages/sandbox/sandbox/src/index.ts:59`, `:99` · `docs/subsystems/sandbox.md:30` | Epsilon's `Sandbox` trait exposes only `name()`/`available()`/`run()` (`crates/epsilon-safety/src/sandbox.rs:114`), so an ambient-ACL gap is invisible. A boolean `available()` over-claims. | S | Low |
| 5 | Three-part runner-failure predicate (nonzero exit + allowed-exit-code gate + fatal signature within one line, after exact informational exclusions) kept separate from per-backend denial dialects | `docs/subsystems/sandbox.md:98`, `:118` · `docs/postmortem/0004-landlock-partial-notice-misclassified-child-failures.md:33` | Postmortem 0004 is exactly Epsilon's current shape: one shared marker, any nonzero exit ⇒ sandbox fault. `rg` exit 1 (no matches) would be reported to the model as a confinement failure. | M | Medium — needs per-backend dialect tables |
| 6 | Escalation ladder: strictly-wider mode table, approval *before* execution, argument-pairing validation, model-facing denial + hint markers | `packages/sandbox/sandbox/src/escalation.ts:28`, `:71`, `:157` | Epsilon's ADR-005 says a refusal reaches the model as `[confinement]` so it can "surface a real approval ask" — but there is no escalation vocabulary, so the model has no lever and re-tries or gives up. | M | Medium — touches the gate and the bash schema |
| 7 | Per-call sandbox mode vocabulary `read-only` / `workspace-write` / `danger-full-access`, resolved once per call | `packages/sandbox/sandbox/src/index.ts` (via `docs/subsystems/sandbox.md:9`) · `packages/sandbox/sandbox-local/src/profiles.ts:16` | Epsilon's `bwrap_argv` (`crates/epsilon-safety/src/sandbox.rs:187`) hardcodes the workspace writable; there is no read-only execution mode at all, so a read-only review task cannot be expressed. | S | Low |
| 8 | Generic job registry: first-wins settlement, `reported` flag suppressing duplicate completion notices, per-owner admission cap, bounded `wait`, consuming `readOutput` cursor | `docs/subsystems/jobs.md:102`, `:157`, `:173` · `packages/jobs/jobs/src/types.ts:121` | Epsilon has background subagents but no unified registry; the `reported` flag exists specifically so teardown does not "spend a model request per teardown layer" (jobs.md:135) — a real cost bug Epsilon is exposed to. | M | Medium — new subsystem; can start subagent-only |
| 9 | Tool timeout as a *cooperative* deadline: tool declares `timeoutMs` and honors `exec.signal`; wrapper maps only its own timer to a structured `TOOL_TIMEOUT`, code-scoped so a nested outer deadline reads as an ordinary cancel | `packages/guard/timeout-policy/src/index.ts:25`, `:56`, `:73` | Epsilon races the future with `tokio::time::timeout` (`crates/epsilon-tools/src/shell.rs:226`), abandoning the tool promise — the exact pattern DSH's header comment refuses. Only bash has a wall clock; read/grep/edit cannot time out. | S | Low |
| 10 | Repeat-call guard: deep key-sort canonicalization, threshold ladder `[3,5,8]` gentle→detailed, advisory fold onto `additionalContexts` that survives a *blocked* call, arguments preview cap, chain reset on user interjection | `packages/guard/repeat-tool-reminder/src/index.ts:89`, `:189`, `:213` | Epsilon's doom-loop brake (`crates/epsilon-core/src/exec.rs:271`) keys on raw `format!("{name}\u{0}{args}")` — same args in a different key order are different calls, so the brake misses real loops; it is also a blocking gate with no escalating reminder and no reset after steering. | S | Low |
| 11 | Telemetry seam: `ledger`/`ops` channels, severity pre-mapped at capture, fail-closed redaction waterfall (a throwing rule withholds that record), sharing disclosure, cursor marks *handed-off* not delivered | `docs/subsystems/session-telemetry.md:31`, `:57`, `:126` | Epsilon has no outbound telemetry; when it adds any, the two things that go wrong are leaks (no redaction point) and double-counting (no dedupe key on `(session.id, event.seq)`). | M | Medium — port the seam + a JSONL sink, not OTel |
| 12 | Token meter reuses provider `usage` only when the request envelope matches and its total ≥ that call's heuristic anchor; otherwise `estimated`; carries signed `surfaceDeltaTokens` and positional per-node prices | `docs/subsystems/token-meter.md:13`, `:29`, `:59` | Epsilon's compaction threshold rides a rough heuristic (`crates/epsilon-core/src/compact.rs:11`) and never reconciles against real `usage`. Compaction either fires early (wasted context) or late (a provider 400). | M | Medium |
| 13 | Tool-pairing balance: fold tool-call/result balance over the *current surface order*, expose `balancedBefore/After(seq)` as the only legal compaction cuts | `packages/compaction/compaction/src/tool-pairing.ts:110`, `:129` | Epsilon prunes by walking back a token budget with no pairing guarantee (`crates/epsilon-core/src/compact.rs:191`). A cut between an assistant tool-call and its result is a prompt the Anthropic and OpenAI wires reject. | S | Low |
| 14 | Session stats projection folded from the durable log: turns, steps, `llmMs`, `toolMs`, `ttftMs`, `decodeMs`, `decodeTokens` | `packages/session/session-stats/src/types.ts:22` | Epsilon records `cache_read` per message but exposes no per-session rollup, so "is this session slow because of tools or the model?" is unanswerable without external tooling. | S | Low |
| 15 | Read-before-edit + version-guarded write/edit: unseen target ⇒ `FS_NOT_OBSERVED`, observed version becomes the CAS basis, stale ⇒ `FS_STALE_VERSION` | `packages/fs/fs-observation-policy/README.md:36`, `:58` | Epsilon has no prior-observation tracking (no `observed`/`read_before` symbols in `epsilon-tools`). A model that edits a file it never read, or edits on a stale read after an external change, silently clobbers. | M | Medium |
| 16 | Session-event envelope: an unknown event *type* refuses the log unless that event carries `ignorable: true` — over-refusal is the deliberate default | `packages/core/session/src/types.ts:49`, `:423` | Epsilon has header versions 1→2→3 migrations (`crates/epsilon-session/src/store.rs:445`) but no per-event opt-in, so forward compatibility of the vocabulary is all-or-nothing. | S | Low |
| 17 | Runtime invariants registry: per-package checks installed from a companion module, allow/blocklist-configurable, failure throws `InvariantError` naming the owning package | `packages/runtime-diagnostics/invariants/src/index.ts:17`, `:50` | Epsilon's only enforcement of loop invariants is `debug_assert`-style or test-time. DSH asserts owned relationships against the authoritative event stream at runtime and attributes the failure. | S | Low |
| 18 | Persistent PTY sessions: six tools, `TerminalWaitReason` (`stdin_read`/`inferred_idle`/`timeout`/`session_exit`) kept orthogonal to session status, bounded scrollback paging, signal to the *foreground process group* | `packages/terminal/terminal/src/types.ts:29`, `:39` · `docs/subsystems/terminal.md:17` | Epsilon's bash is one-shot: a REPL, a dev server, or a `git rebase -i` cannot be driven. DSH's orthogonal wait-reason/status split is also the correct shape for Epsilon's job events. | L | High — new primitive (ConPTY / pty crate) |

## Detail

### 1. Credential-scrubbed child environment

`scrubbedParentEnv()` (`packages/subprocess/subprocess/src/index.ts:60`) builds the child env by
iterating `process.env` and dropping any key matching `SENSITIVE_ENV_PATTERN` **and** any key in the
harness-reserved `DSH_*` prefix, keeping everything else verbatim. It is the *base layer* for every
spawn in the system: LSP (`packages/lsp/lsp-stdio/src`), the MCP client transport
(`packages/mcp/mcp-client/src/transport.ts:22`), and the web app all spread it before their own
overrides — so the scrub cannot be forgotten by a new consumer.
`docs/defensive-patterns.md:29` states the rule it enforces: "Spawned commands get a scrubbed env
(drop `*KEY*`/`*SECRET*`/`*TOKEN*`/`*PASSWORD*`) so harness credentials cannot leak into output,
`env`, or spill files." **Epsilon has nothing here** — grepping `KEY|SECRET|TOKEN|PASSWORD` across
`crates/epsilon-safety/src/` and `crates/epsilon-tools/src/shell.rs` returns zero hits, and
`shell.rs` never calls `.env(...)`. Landing site: a `scrub_env()` helper in `epsilon-safety`
(alongside the sandbox backends) applied in `crates/epsilon-tools/src/shell.rs` before spawn, plus
the same in `epsilon-mcp` and the extension host. Pair it with DSH's tombstone semantics
(`docs/subsystems/subprocess.md:15`): harness-owned env facts arrive only as deliberate entries, and
an explicit removal must be able to erase an ambient value.

### 2. Spill storage hardening

Three independent defenses in one function. `privateRoot()`
(`packages/spill/spill-local/src/store.ts:27`) lazily `mkdtempSync`s a `0700` directory under the OS
tmpdir — `mkdtemp` supplies an unpredictable suffix, so a peer cannot pre-create the directory.
`sessionDir()` (`:73`) nests a 12-hex-char `sha256(sessionId)` segment so sessions do not collide.
`saveTextFile()` (`:107`) composes a **random 6-byte hex prefix + `encodeSegment(suggestedName)`**,
where `encodeSegment` (`:48`) is an injective, reversible escape of every code unit outside
`[A-Za-z0-9._-]` — so `../`, NUL, and separators in an untrusted session id or suggested name cannot
traverse. The write itself is `open(path, 'wx', 0o600)` (`:113`): exclusive *and* owner-only, which
fails on any pre-existing path whether or not it is a symlink. Compare Epsilon's
`crates/epsilon-tools/src/util.rs:236`: `spill_dir.join(format!("{stem}-{millis}.out"))`, then
`let _ = fs::write(&spilled, output)` — predictable name, no mode bits, and the failure is discarded,
so the model is told "full output saved to X" even when nothing was written. Port the naming and the
open flags; make the write failure change the model-facing text into a best-effort notice (DSH's
policy consumer explicitly *keeps the original inline result* on a save failure rather than turning
a successful call into an error — `docs/subsystems/spill.md:85`).

### 3. Byte-exact head/tail retention

`TextRetainer` (`packages/util/output-retention/src/index.ts:247`) accumulates a prefix up to
`prefixCap` and a rolling suffix of `suffixCap`, dropping whole leading chunks as they slide out, so
memory is bounded by `prefixCap + suffixCap + one chunk` regardless of stream size. `finish()`
(`:350`) then handles two subtleties Epsilon's line-slicing cannot. First, UTF-8: with no budget
omission the two halves are adjacent slices of one stream, so it concatenates *before* decoding
(`:370`) — decoding separately would corrupt a codepoint spanning the artificial split; with a real
gap it trims a partial trailing sequence (`trimTrailingPartialUtf8`, `:211`) and leading continuation
bytes (`:228`) so the cut never manufactures a replacement char. Second, honesty: `omittedBytes` is
computed against the bytes **actually returned** (`:376`), not the budget, because a boundary trim
drops extra bytes — a notice derived from the budget alone would overstate what the model holds. The
`Omitted` union (`:40`) distinguishes `exact` from `unknown` so a notice never prints false
precision. Epsilon's `truncate_output` (`crates/epsilon-tools/src/util.rs:222`) keeps only the tail
by line count, with a byte-cap second pass and a hand-rolled char-boundary walk. Landing site: a
`retention` module in `epsilon-tools` (or `epsilon-contracts` if the TUI also formats notices), with
per-tool strategy choices — `head` for grep/glob, `headTail` for bash and webfetch, `tail` for
logs. The `RetentionNotice`/`formatRetentionNotice` split (`:436`) is worth copying: the library owns
the omission clause, the tool owns the recovery words ("narrow the pattern", "read the spill file").

### 4. Sandbox enforcement reporting

`SandboxEnforcement = 'full' | 'partial'` (`packages/sandbox/sandbox/src/index.ts:59`) travels on
`ConfinedArgv.enforcement` (`:99`) as a *fact about this host and this call*, not a config value.
The doc is blunt about the contract (`docs/subsystems/sandbox.md:30`): "`partial` means an active
backend or older kernel ABI governs only a subset, so consumers that require the absolute promise
must reject or surface that distinction. Older Landlock ABIs and the Windows ACL runner's
Everyone/hard-link boundaries are current partial cases." Epsilon's `Sandbox` trait
(`crates/epsilon-safety/src/sandbox.rs:114`) exposes `name()` and a boolean `available()`, and
`ConfinedOutcome` (`:64`) carries `exit_code` / `stdout` / `stderr` / `backend` — no completeness
fact anywhere. Landing site: add an `fn enforcement(&self) -> Enforcement` to the `Sandbox` trait
(and to `ConfinedOutcome`, so a per-call decision can differ), return `Partial` from the Windows
Job Object backend **and** from bwrap whenever the requested policy names a cap bwrap cannot
enforce. That last point matters: `sandbox.rs:215` already documents that bwrap enforces no
memory/CPU caps, and today that documented limitation is invisible to the caller that set
`process_memory_limit_bytes`.

### 5. Runner-failure rules vs denial dialects

Two orthogonal classifiers ride the wrapped argv. `denialSignatures`
(`packages/sandbox/sandbox/src/index.ts:108`) are the *dialect a denial produced by this backend*
prints — EROFS text under bwrap read-only binds, EACCES under Landlock, EPERM under Seatbelt — and
the doc insists a consumer match **exactly these, not a cross-backend union**, because the union
"claims denials a given backend never produces." `runnerFailureRules` (`:115`) is the harder half:
`RunnerFailureRule` (`docs/subsystems/sandbox.md:98`) requires a nonzero exit, an optional
`allowedExitCodes` gate, and a case-insensitive `fatalSignatures` substring *within one remaining
stderr line*, after removing `informationalLines` by exact full-line (case-insensitive) equality.
Ordering is explicit (`docs/subsystems/sandbox.md:118`): check runner failure **first** — that means
the command never ran and is an infrastructure fault — because "denial means confinement worked and
blocked it." Postmortem 0004 (`:33`) is the failure this prevents: the shared `landlock-run: `
prefix plus any nonzero exit classified `rg`'s no-match exit 1 as `SANDBOX_UNAVAILABLE`, and the
filesystem layer then replaced the structured error with a generic `SEARCH_FAILED` — losing the code
and misdiagnosing a valid result. Epsilon's `SandboxError` (`crates/epsilon-safety/src/sandbox.rs:80`)
has one `Setup(String)` and one `Unavailable(String)`, and `shell.rs` renders both behind one
`[confinement]` marker. Landing site: split the enum into `Denied { mode, dialect_matched }` vs
`RunnerFailed { backend, exit, matched_line }`, give each backend a signature table, and keep the
`rg`-exit-1 case out of both.

### 6. Sandbox escalation ladder

`WIDER_MODES` (`packages/sandbox/sandbox/src/escalation.ts:28`) is a strictly-wider table —
`read-only → {workspace-write, danger-full-access}`, `workspace-write → {danger-full-access}` — and
the comment pins *why* it is an execution check rather than a schema enum: "schemas are
registry-global while the effective mode is per-call truth" (`:24`). `ESCALATION_TARGETS` (`:41`) is
the closed advertisement vocabulary, deliberately not narrowed to modes wider than the composition's
default, "cutting the enum down ... would strand a session whose effective mode sits below it."
`approveEscalation` (`:157`) is the ordered fail-closed sequence: validate strict widening → require
an approval service → require an agent → request with a self-contained audit reason
(`escalate sandbox to ${mode}: ${justification}`) → map all four outcomes, with `assertNever` on the
exhaustive switch. Two model-facing texts ship as constants: `sandboxDenialMarker(mode)` (`:71`)
and `escalationHintMarker(subject)` (`:85`), the latter the nudge to retry once with
`sandbox_permissions` + `justification`. `validateEscalationArgs` (`:51`) enforces the pairing a
schema cannot express: the two arguments travel together and the justification must be a non-empty
sentence. Epsilon's ADR-005 wants this ("surface a real approval ask") but has no vocabulary, no
ladder, and no paired arguments. Landing site: a new `escalation.rs` in `epsilon-safety` consumed by
`epsilon-tools/src/shell.rs` and the mutation tools, reusing Epsilon's existing gate for the ask.

### 7. Per-call sandbox mode

`SandboxExecutionPolicy` (`docs/subsystems/sandbox.md:51`) carries `mode`, absolute `workspaceRoot`,
and an optional `sessionId`, resolved **per call** by `ctx.sandboxPolicy.resolve()` with documented
precedence: approved explicit mode > session's last logged mode > deployment default. The doc makes
the concurrency property explicit (`:79`): two consumers may confine under different policies at the
same instant, and "an approved escalated retry is a new call with a wider policy" — so the provider
holds no mutable mode state. `workspace-write` adds `--bind workspaceRoot`; `read-only` omits the
bind entirely and grants only the `/dev/null` sink the shells require
(`packages/sandbox/sandbox-local/src/profiles.ts:16-22`). Epsilon's `bwrap_argv`
(`crates/epsilon-safety/src/sandbox.rs:187`) always emits exactly one writable `--bind` for the
workspace, and there is a test asserting that (`:301`) — so the mode is structurally absent.
Landing site: turn `bwrap_argv`'s workspace argument into a mode-driven argument list, add a `mode`
field to `ConfinementPolicy`, and let the gate's decision carry an override.

### 8. Generic job registry

`JobHooks` (`docs/subsystems/jobs.md:62`) is the whole producer contract: synchronous idempotent
`cancel(reason?)`, `done: Promise<JobOutcome>` that resolves "after the producer releases its
resources, not merely when work finishes" (`:60`), and an optional `readOutput()` whose *presence*
marks a consuming-stream job. `JobSnapshot.reported` (`packages/jobs/jobs/src/types.ts:121`) is the
subtle piece: "True when a kill, read, wait, or teardown cancel has reported or committed to report
the terminal state. Completion reporters suppress redundant notices when set. Teardown claims it
because the owner or service being destroyed leaves no reader: a reporter that opens a turn on
notice would otherwise spend a model request per teardown layer" (`docs/subsystems/jobs.md:135`).
Settlement is first-wins and announced **last**, "after the record is committed and every other
observer of the settlement has seen it, because a reporter may open a model turn synchronously"
(`:177`). Admission is bounded: `maxConcurrentJobsPerOwner` defaults to 10 and counts `running` +
`stopping`, with one shared bucket for unowned jobs (`:157`). Authorization fences on `ownerSession`
— "Ids are predictable, so authorization — not secrecy — is the boundary" (`:176`). Epsilon has
background subagents with `task_id` status probes (`crates/epsilon-tools/src/task.rs:130`) but no
shared registry, so bash background, subagents, and (future) PTY sends cannot be listed or killed
through one surface. Landing site: `epsilon-core/src/jobs.rs` with the `reported` flag and first-wins
settlement; migrate `epsilon-tools/src/task.rs` onto it as the first producer.

### 9. Cooperative tool timeout

`packages/guard/timeout-policy/src/index.ts:55` registers on `tools/execute`, reads the
caller-visible tool's declared `timeoutMs` (`:57`), and returns `next()` unchanged when the tool
declares none (`:59`). The plugin names its own code `TOOL_TIMEOUT` (`:25`) and uses it **twice** —
as the deadline's classification code and as the structured `error.code` on the replacement result —
"Scoping `timeoutOf` to it keeps a nested outer deadline (another `tools/execute` wrapper's timer
that fired first) from being misread as this plugin's own timeout — it reads as an ordinary upstream
cancel" (`:18`). It swaps the derived signal onto `exec.signal`, awaits `next()`, and only replaces
the result when its own timer fired (`:73`), restoring the upstream signal in a `finally` so
post-execute listeners never observe an already-aborted timeout signal. The stated contract (`:1`):
it "arms that deadline and maps its own expiry to `TOOL_TIMEOUT` without racing or abandoning the
tool promise." Epsilon does the opposite — `crates/epsilon-tools/src/shell.rs:226` wraps the future
in `tokio::time::timeout`, which drops the in-flight future at expiry, and only bash has any clock.
Landing site: a `timeout` wrapper in the `epsilon-core` executor driven by a `timeout_ms` field on
`ToolDescriptor`, with the deadline propagated as the existing cancellation token.

### 10. Repeat-call guard upgrade

`canonicalize` (`packages/guard/repeat-tool-reminder/src/index.ts:103`) deep key-sorts every object
before `JSON.stringify`, so two argument objects differing only in property order key identically —
the comment notes the input domain is exactly JSON's, so no bigint/cycle handling is needed. The
chain is per-agent (`WeakMap<Agent, Chain>`, `:173`) and holds only `{key, count}`. Thresholds are
validated fail-loud at load (`:128`): non-empty, integers ≥ 2, no duplicates, sorted ascending so
`thresholds[0]` is reliably the gentle tier (`:200`). The reminder escalates from
`GENTLE_REMINDER` (`:63`) to a structured `detailedReminder` (`:70`) naming tool, run length, and the
canonical arguments — truncated by `previewArguments` (`:118`) to `argumentsPreviewChars` (default
500) with an explicit `… (+N more chars)` marker, because "Large payloads ... would otherwise ride
into the next request unbounded — precisely in a loop scenario" (`:36`); the *chain key* still uses
the full string. The most interesting choice is at `:213`: it counts **before** delegating, then
folds its reminder onto whatever `next()` returned, including a `block` decision — "a blocked call
still gets the nudge" (`:211`) — and it counts denied calls at all because "a model hammering a
denied call is exactly the loop worth breaking" (`:186`). Finally `agent/pre-step` (`:229`) deletes
the chain when any incoming message has `source.kind === 'user'`: "A user interjection changes the
context; repetition across it is not a loop." Epsilon's brake (`crates/epsilon-core/src/exec.rs:271`)
compares a raw `name\u{0}args` string, fires a permission request at a fixed threshold, and never
resets on steering. Port the canonicalization, the ladder, the preview cap, and the reset.

### 11. Session telemetry seam

`SessionTelemetryRecord` (`docs/subsystems/session-telemetry.md:31`) splits two channels that must
never be confused: `ledger` records mirror session-log events one-to-one and carry `event.seq`
identity; `ops` records carry the two signals with no log home (`agent-error`, `shutdown`) and
"deliberately omit `event.seq`-style identity so they can never be mistaken for ledger rows" — they
are "signals to alert on, not entries to sum" and tolerate duplicates (`:57`). Severity is mapped at
**capture**, not by the receiver: `error` for events whose own outcome flag says so (the tool-result
block's `isError`, `turn/end` error reasons), capturing "so a receiver can alert with zero
configuration" (`:15`). The `session-telemetry/record` waterfall (`:169`) is the redaction point and
ships **no rules of its own** — "with no listener mounted records reach the backend as captured, so
exported data is exactly as clean as the rules a deployment mounts"; a throwing listener withholds
that one record fail-closed inside the coordinator's containment and never reaches the loop.
Redaction applies to the exported copy only; the canonical session log is never rewritten. The
`emit()` contract (`:82`) is a non-blocking enqueue because it is called synchronously from the
`session/event` hot path, and `shutdown()` (`:105`) must reach quiescence with everything emitted
before it still delivered. Landing site: an `epsilon-telemetry` crate (or a module in
`epsilon-session`) emitting from the existing event bus, with a JSONL sink as the first backend.

### 12. Token meter with anchor reuse

`measure(session, requestHeader?)` (`docs/subsystems/token-meter.md:59`) returns a detached snapshot.
The pricing rule is stated exactly (`:29`): `baseline.kind === 'usage'` applies only when the latest
successful provider call has the **same canonical request envelope** and its total is **no lower
than that call's full heuristic anchor** — otherwise `estimated`, meaning the service repriced the
whole envelope with its fixed heuristic. The asymmetry is deliberate: reuse is conservative, so an
anchor is never allowed to under-report current pressure. `surfaceDeltaTokens` is *signed*, so both
growth and shrinkage relative to a matching anchor survive; `totalTokens` is request-and-response
pressure while `surfaceTokens` is surface-only and equals the sum of node prices (`:29`).
`TokenSurfaceNode` (`:33`) carries the durable `seq` and the heuristic price of "the exact message
projected by this node," with the ordering caveat that surface order is authoritative and
"replacement nodes can have higher durable seqs than later positional nodes" (`:43`) — exactly the
shadow-view situation Epsilon has after compaction. `logRevision` (`:15`) is the count of durable
events consumed, so a measurement is pinned to a revision and cannot silently grow. Every call
clones its positional nodes, so measurement is O(surface) (`:72`). Landing site:
`epsilon-core/src/tokens.rs`, replacing the "roughly right" estimator at
`crates/epsilon-core/src/compact.rs:11` for the threshold decision while keeping the cheap estimator
for the per-message hot path.

### 13. Tool-pairing balance

`BalanceCache` (`packages/compaction/compaction/src/tool-pairing.ts:11`) stores one boolean per
legal cut: a surface of N sequences has N+1 cuts, entry `i` being the cut before sequence `i` and
the final entry the cut after the tail. `eventDelta` (`:29`) contributes `+count(tool-call blocks)`
for an `assistant/message` and `-1` for a `tool/result`; a cut is balanced iff the in-progress count
is zero. Two integrity checks make this more than bookkeeping: `extendCache` (`:50`) validates the
unseen tail *before* mutating the live cache, "so a corrupt append cannot leave a partially advanced
state behind," and throws on a `tool/result` with no preceding open call (`:65`); `eventForSeq`
(`:41`) throws when a surface seq has no matching log event. The cache invalidates on
`surface.replaceGeneration` change or when the cached length exceeds the current surface (`:83`) —
necessary because compaction rewrites positions, and the module's header says safe cuts must be
"derived from tool-call/result content in current surface order rather than step markers" (`:3`).
Epsilon prunes by walking backwards accumulating `tail_protect_tokens` and cutting at an index
(`crates/epsilon-core/src/compact.rs:177-180`), replacing pre-cut tool results with a tombstone —
but nothing prevents that index from landing between a call and its result. Landing site: a
`pairing.rs` beside `compact.rs` exposing `balanced_before/after(seq)`, and make the pruner refuse an
unbalanced index by walking outward to the nearest balanced cut.

### 14. Session stats projection

`SessionStatsProjection` (`packages/session/session-stats/src/types.ts:22`) folds, from the complete
durable log, independent of how much history a client has paged in: `turns` (distinct turns carrying
at least one closed step — "rejected or empty turns are uncounted"), `steps`, `llmMs`
(`step/start` → `assistant/message`) `toolMs` (`tool/call` → `tool/result` **matched by callId**),
`ttftMs` (first-token latency, with a companion `ttftSteps` count so an average is never taken over
a wrong denominator), and `decodeMs`/`decodeTokens` (first token → `assistant/message`, "over steps
that also report output tokens" — the same denominator guard). Every field is 0 until its first
contributing event lands (`:17`). It is registered as a projection key (`:41`), so it is computed
incrementally off the existing projection machinery rather than by a fresh scan. Epsilon has no
per-session rollup: `Usage` with `cache_read` exists on messages
(`crates/epsilon-contracts/src/message.rs:119`) but nothing aggregates it. Landing site: fold into
the session store's existing read path and surface via the TUI status line and `/session`. The
denominator guards are the part worth copying deliberately — a naive `ttftMs / steps` is a
permanently wrong average once any step lacks a first token.

### 15. Read-before-edit and version-guarded mutation

The policy is three listeners over the `fs/*` event gate, not a service
(`packages/fs/fs-observation-policy/README.md:36`). On `fs/write-intent`: a target never seen or
observed absent resolves to `{kind:'createIfAbsent'}`; observed present resolves to
`{kind:'replaceIfVersion', version: vObserved}`. On `fs/edit-intent`: never seen ⇒ `FS_NOT_OBSERVED`,
observed absent ⇒ `FS_NOT_FOUND`, observed present ⇒ `{version: vObserved}` as the CAS basis. The
`fs/observed` listener records `{kind:'present', version}` or `{kind:'absent'}` per owner+target
with a synchronous `WeakMap.set`. The design point is at `:58`: observed state is *the
prior-observation record*, and **freshness is provider-owned CAS** — a stale positive observation
propagates the provider's `FS_STALE_VERSION` rather than being re-checked here. Two documented
consequences: "Actors without an agent session can never satisfy the policy — their edits throw
`FS_NOT_OBSERVED` ... so a non-agent caller cannot overwrite an existing file through the gate"
(`:71`), and "Direct `ctx.fs` reads emit no `fs/observed` — a file read outside the `read` tool
stays unobserved, and a later guarded edit rejects ... until the tool reads it" (`:72`). Error
codes are preserved and only the *recovery instruction* is appended by the tool layer (`:58`).
Epsilon has no prior-observation tracking at all (no `observed` or `read_before` symbols in
`crates/epsilon-tools`), so a model editing a file it never read, or editing on a stale read after
an external writer changed it, silently clobbers. Landing site: a per-agent
`HashMap<(owner, path), Observed>` in `epsilon-tools/src/mutation.rs`, with the edit path rejecting
unseen targets and the write path choosing create-vs-replace.

### 16. Forward-compatible session-event envelope

`SESSION_FORMAT_VERSION` is `0` and the doc is explicit about when it must **not** bump: "Adding an
ordinary event type does not bump — the per-event `SessionEvent.ignorable` guard covers vocabulary
growth instead. When in doubt, bump: a near-identity upgrade step is almost free, a missed bump
makes older runtimes read new logs wrong silently"
(`packages/core/session/src/types.ts:49`). The mechanism is the `ignorable?: true` member on the
event envelope (`:423`), and its comment states the asymmetry deliberately: a `SessionEventMap`
member is **required-on-read by default** — a build that does not know the type refuses the log —
"so defaulting to required means a forgotten marker over-refuses (an inconvenience) rather than
silently resuming a gutted session." So there are two levers with different costs: bump the format
version (expensive, structural change) or mark the event `ignorable` (cheap, vocabulary growth), and
the default is refusal. Epsilon has header versions with real migration code
(`crates/epsilon-session/src/store.rs:445`, `migrateV1ToV2` + `migrateV2ToV3` at `:762`) and a guard
that "a current-version file must not contain legacy shapes" (`:457`) — but nothing per event, so
the choice is all-or-nothing. Landing site: an `ignorable: bool` (default false) on Epsilon's
session entry type plus a load-path policy that skips ignorable unknowns and refuses the rest with
a message naming the unknown type.

### 17. Runtime invariants registry

`Config` (`packages/runtime-diagnostics/invariants/src/index.ts:17`) carries a global `enabled`
switch plus `package_allowlist`/`package_blocklist` of case-sensitive regex sources — so a
deployment can run all invariants, none, or one package's. `InvariantInstaller` (`:32`) takes a child
`ctx` and a `fail: InvariantFailure` reporter (`:29`), and the failure is package-attributed by construction:
`InvariantError` (`:50`) carries `readonly packageName: string` and composes the message as
`invariant violated by "${packageName}": ${message}` (`:62`). The pattern is a `./invariant`
companion module per package so "ordinary package entrypoints stay independent of diagnostics"
(`:4`) — i.e. the checks are opt-in at load, not compiled into the hot path. The repo's convention
doc states what these must assert (`AGENTS.md:104`): "Runtime invariants assert owned
relationships. Check authoritative event streams or mutable data, not service or method presence,
plugin metadata or effects, or fixed pure examples." Epsilon's loop invariants are currently
enforced only by tests and `debug_assert`. Landing site: a `debug_invariants` module in
`epsilon-core` behind a cargo feature, checking relationships over the event stream (e.g. every
`ToolResult` has a preceding `ToolCall` in the same turn, sticky turn reasons never downgrade,
queue depths never go negative) and attributing failures to the crate that registered them.

### 18. Persistent PTY sessions

`TerminalWaitReason` (`docs/subsystems/terminal.md:17`) is `'stdin_read' | 'inferred_idle' |
'timeout' | 'session_exit'`, and the doc draws the line that makes it usable (`:11`): "It is
independent from `TerminalSessionStatus`: silence or timeout may return while the top-level shell
remains alive, while `session_exit` means that shell exited rather than an arbitrary foreground
child." Conflating those two is the classic PTY bug — a `timeout` read reported as "the shell
died". `TerminalBackendSession` (`:44`) exposes `motd` (bounded initial output), `startSend`,
`read` (one bounded page from retained scrollback), `signal` (to the *verified foreground process
group*), `status()`, and an idempotent `close(reason)` that closes "the captured owned process tree
and await[s] quiescence". A backend that cannot clean partial startup resources rejects with
`TerminalBackendCleanupError` so disposal "retain[s] the cleanup failure without replacing the
caller's cancellation reason" (`:33`). Six model-facing tools (open/list/send/read/signal/close)
with `run_in_background` registering with `ctx.jobs` (`docs/tool-catalog.md:29`). Epsilon's bash is
strictly one-shot, so a REPL, an interactive installer, a dev server, or `git rebase -i` cannot be
driven at all. Landing site: a `terminal` module in `epsilon-tools` over a pty crate (ConPTY on
Windows), reusing the job registry from candidate 8 for background sends. Large; the wait-reason vs
status split is worth adopting even if the full toolset is deferred.

## Deliberately do not port

- **Cordis vendoring / everything-is-a-plugin** — Epsilon's compile-time enforced crate DAG and typed
  seams are the deliberate opposite choice; the plugin loader buys runtime remountability Epsilon does
  not want to pay for.
- **Code Mode (`run_code`) and the self-modification toolset** (`cordis_define`, `cordis_run`,
  `cordis_inspect_*`; `docs/tool-catalog.md:19,23`) — routes model-authored code into the live
  runtime; Epsilon's security posture is a deliberate no.
- **Waterfall middleware with mandatory `next()`** — clever, but Epsilon's explicit typed hooks give
  the same extension points without the "forgot to call next()" failure class.
- **Declaration-merged `SessionEventMap`** — TypeScript-only mechanism; Epsilon's enum + version field
  is the right shape once candidate 16 adds the per-event opt-in.
- **E2B / ACP / remote-execution seams and the Codex/Claude-Code subagent backends** — out of scope;
  Epsilon's isolation story is process-local.
- **Durable web schedule** (`docs/subsystems/schedule.md`) — timer-driven future turns are a web-UI
  feature; Epsilon sessions are local and operator-driven.
- **The Typert type-graph generator and the Python SDK parity gates** — build tooling for a
  multi-language surface Epsilon does not have.
- **Vendoring the OpenTelemetry JS SDK as a backend** (`dsh-session-telemetry-otel`) — port the seam
  and a JSONL sink (candidate 11); the SDK's batching/retry/loss policy is someone else's problem to
  own, not ours to inherit.
- **LSP capability seam** — a real feature, but it is a new tool family rather than a robustness
  mechanism, and it needs a language-server lifecycle Epsilon has no home for yet.
- **Per-file 100% coverage gate** — Epsilon's own gates (zero-warning clippy, MSRV, bwrap suite)
  already cover the regression surface that gate protects.
- **Branded-id types on every cross-boundary id** — valuable in TS where `string` is the default;
  in Rust `newtype` wrappers give the same safety and Epsilon already uses them where it matters.
- **`DSH_*` env prefix naming** — adopt the *concept* (candidate 1) with an `EPSILON_*` prefix.
