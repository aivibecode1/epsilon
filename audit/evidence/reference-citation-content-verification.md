# Content-verification of the 55 pi citations: what a path check cannot tell you

Captured, not composed. Three read-only subagents read every cited line range in
`references/pi-0.85.1/`. Companion to
[`reference-citations-point-at-a-missing-tree.md`](reference-citations-point-at-a-missing-tree.md),
which established that the citations point at a tree that no longer exists.

## Why this pass exists

`scripts/check-reference-citations.py` proves two things: the cited file exists under 0.85.1, and
the line range is inside it. It cannot tell whether those lines say what the document claims.
**"In bounds" is not "correct"** — a version bump shifts line numbers, and a citation can sit
comfortably inside a real file while pointing at unrelated code. Judging that is not mechanical,
and a script that guessed would be exactly the fabricated evidence this repo forbids.

## Results

| Group | Citations | SUPPORTS | PARTIAL | CONTRADICTS | UNFINDABLE |
|---|---|---|---|---|---|
| `audit/03-port-plan.md` | 22 | 12 | 8 | 1 | 1 |
| `audit/06-reference-comparison.md` | 7 | 2 | 3 | 0 | 2 |
| `docs/research/gap-analysis-2026-08-27-v2.md` | 7 | 3 | 3 | 0 | 1 |
| 12 smaller files (plans, milestones, evidence) | 18 | 7 | 11 | 1 | 0 |

Every non-SUPPORTS verdict is **line drift or a rename, not a refutation.** The underlying
findings stand; their evidence does not. The four that change a *conclusion* rather than a line
number are below.

## The four that change a conclusion

**1. `navigateTo` does not exist — the method is `navigateTree`, and it has five callers.**
This invalidated a claim I had committed 20 minutes earlier. I had written that pi's
`AgentSession.navigateTo` "has no caller in pi's own source", from a repo-wide grep for
`navigateTo` that returned 5 hits, all an unrelated cursor option in
`tui/src/components/settings-list.ts`. **That identifier does not exist in 0.85.1.** The real
callers are `interactive-mode.ts:1889,5288`, `print-mode.ts:86`, `extensions/runner.ts:827`,
`agent-controller-provider.ts:60`.

The F22 withdrawal it supported **still holds**, but on entirely different grounds: the only
production construction of `EntryKind::BranchSummary` is `store.rs:1397` inside
`branch_with_summary`, and all 7 of its callers are tests. A grep for "how do you navigate" can
miss; a grep for "where is the thing constructed" cannot. See
[`F22-compaction-usage-never-read.md`](F22-compaction-usage-never-read.md).

Also surfaced: Epsilon **does** have a `/tree` command (`picker.rs:354`, "navigate session tree
(switch branches)") which my earlier identifier sweep missed. It is an **alias for `/timeline`**
(`app.rs:1721`) — it calls `store.branch_entries()` and opens a picker. It lists branches; it
does not create a `BranchSummary`. A slash command's own description is not a specification.

**2. pi has eight coding tools, not seven.** `tools/index.ts:95`:
`ToolName = "read" | "bash" | "powershell" | "edit" | "write" | "grep" | "find" | "ls"`.
pi added `powershell` after 0.84.2. Epsilon registers 11 tools covering **7 of those 8**; there
is no separate `powershell` tool. The capability is not missing — Epsilon's `bash` tool *is*
PowerShell on Windows (`shell.rs:47`, asserted at `shell.rs:483`) — but the surface differs, and
on Windows an Epsilon model cannot run `bash` at all. The old row claimed "all seven pi tool
names implemented", which was once true and is now an undercount.

**3. Three counts that were used as a parity floor have moved.** pi gained capabilities, so any
Epsilon "parity" claim resting on the old number is void:

| Count | Cited | 0.85.1 | Consequence |
|---|---|---|---|
| Slash commands | 22 (`slash-commands.ts:8`) | **23** — array is `:19–43`, and `thinking` (`:23`) is new | the closed-G1 parity claim and `docs/plans/013` are stale |
| LaTeX symbols | 217 (`tui/src/latex.ts`) | **224** — `SYMBOLS` spans `:3–228` | the only falsifiable "217/217 parity" claim is now 7 short |
| Harness deferred ops | "several" | **one** (`SliceNotImplemented("watchSession")`) | the claim is weaker than written, and its citation pointed at type declarations — the very thing the sentence warns against |

**4. `docs/plans/014:38` cites the wrong file and the prose describes the right one.** It claims
`withFileMutationQueue(env, path, fn)` keyed by a per-env `WeakMap`. The cited
`coding-agent/.../file-mutation-queue.ts` is a 2-argument module-`Map` queue
(`32| export async function withFileMutationQueue<T>(filePath: string, fn: () => Promise<T>)`).
The described shape is real, at
`packages/agent/src/harness/tools/file-mutation-queue.ts:10,30–35`. That plan is marked DONE but
still carries "Executor instructions: follow this plan step by step" with no supersession banner,
so it reads as runnable.

## Live versus historical

The distinction matters more than the count. A stale line number in a dated milestone dev log
(`M88`, `M95`, `m92-review`) is archaeology. The same defect in a document someone will work from
is a live hazard:

**Live — fix first:** `audit/02-defect-ledger.md:666` (the fix-authority document, actively
swept; its `629–635` sub-range lands 7 lines short of the "Operation aborted" return);
`docs/plans/014:38` (wrong file); `docs/plans/013:36` (omits `thinking`); `docs/plans/016:63`
(both specifics wrong — the trigger regex is `[^\s]*`, not `[\w./-]*`, and there is **no**
Tab/Enter asymmetry, both call `applyCompletion`); `docs/plans/015:39` (truncated `trust`
description); `docs/plans/012:71` and `016:38` (file filtering and the `path `-with-space
insertion live in `packages/tui/src/autocomplete.ts:412–418`, not `editor.ts`).

**Historical — record, do not rewrite:** `docs/plans/README:73` (self-declared historical),
`docs/milestones/M95:20` and `M88:14`, `audit/evidence/m92-review:18`, `docs/plans/012:71`
(file is stamped `REJECTED (2026-08-28)`).

**Verified clean, no action:** `plans/014:39,88`, `plans/012:47`, `plans/013:100`,
`queue-doc-review:54` (both — line-for-line exact), `audit/09-revalidation:65`.

## Two more defects in the checker itself

Both found by the subagent, both mine, both the same shape as the earlier `json`/`js` bug:

- **Brace expansion was silently skipped.** `src/{a.ts,b/c.ts}` never matched, so the regex just
  failed and the count came out low **with no error** — a tool that under-reports is
  indistinguishable from a tool that finds nothing wrong. 5 citations were never validated.
- **The first fix attempt made it worse:** a single `path+extension` alternation stops at the
  first `.ts` *inside* the braces, yielding the nonsense path `src/{a.ts` and a bogus "file not
  found". Fixed by matching the brace group as its own alternative, ordered first.

After the fix: **55 citations, 60 concrete paths, 0 failures.**

## Still open

- **47 of 55 citations still cite `pi-0.84.2`** and should be repointed at `0.85.1` with
  corrected line numbers. Every SUPPORTS above is a *content* match under 0.85.1, but the
  document text still names a version that is not present.
- `audit/03-port-plan.md:4` still pins the Rust tree to `e200815`; it is now `9c61af7`. The same
  staleness affects other `audit/` pins.
- The 8 PARTIAL rows in the capability matrix need corrected line ranges; only rows 5 and 8 are
  rewritten so far.
