# 03 — Defect ledger

Confirmed defects, ranked by what they cost the user. Every row names the line
that has to change, because a defect report that does not end in an edit is a
complaint.

Status vocabulary: **Open** (not fixed), **Open, partly fixed**, **Claimed-fixed
but unverified** (a milestone says so; this audit does not believe it), **Fixed**.

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
  [`installed-binary-2026-09-28-post-adr019.md`](installed-binary-2026-09-28-post-adr019.md).
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
  for the session/TUI work at [`installed-binary-2026-09-28.md`](installed-binary-2026-09-28.md)
  and for ADR-019 at
  [`installed-binary-2026-09-28-post-adr019.md`](installed-binary-2026-09-28-post-adr019.md).
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
