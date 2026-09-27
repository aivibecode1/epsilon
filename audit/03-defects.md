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
