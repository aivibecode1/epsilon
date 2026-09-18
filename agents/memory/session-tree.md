# SESSION-01 — session tree / fork / branch (pi parity)

## State
- Brief received 2026-09-18 (deleg_41da778d, read-only, well-cited). Implementation of the
  session-layer half delegated (deleg_24d09389). **Nothing committed yet.**
- **Split decision (mine):** the brief's slice also wires the TUI (`/tree` full listing, Enter
  target-type rule, `/fork` via the new store API). `crates/epsilon-tui/src/app.rs` carries
  another work item's uncommitted changes, so touching it would entangle this slice with theirs
  and make a clean commit impossible. SESSION-01a (now) = the session-layer foundation;
  SESSION-01b (queued below) = the TUI surface once the TUI work lands.

## What pi has (pi-0.84.2, session-manager.ts + agent-session.ts)
- Traversal: `buildSessionPath` walks `parentId` from the leaf (default: last entry, :347) to
  root (:334-360); `getLeafId/getLeafEntry/getEntry` (:1195-1205); `getChildren` (:1210-1218);
  `getBranch(fromId?)` (:1260-1270); `getTree()` = every entry as a timestamp-sorted node,
  roots = `parentId` null/self/orphan (:1310-1348).
- Mutation: `branch(id)` moves the leaf only, appends stay append-only (:1360-1365);
  `resetLeaf()` nulls it so the next append is a new root (:1372-1374); `branchWithSummary`
  appends a `branch_summary` child and advances the leaf (:1381-1405); `createBranchedSession`
  writes a NEW file with only root→leaf, a fresh id and header `parentSession` (:1419-1442),
  deferring the write until an assistant message exists (:1477-1488).
- Projection: `branch_summary` → branch-summary message; `compaction` → compaction summary
  (:401-406); `buildContextEntries` keeps only the latest compaction on the path plus from
  `firstKeptEntryId` onward (:418-454).
- Workflow: `navigateTree` refuses while streaming, collects old-leaf→ancestor entries
  (agent-session.ts:2905-2935), optionally summarizes (:2997-3012), then applies the target-type
  rule — a user/custom_message target moves the leaf to its PARENT and returns the text to the
  editor; any other target keeps the leaf at the node (:3032-3043) — and rebuilds messages from
  `buildSessionContext` (:3077-3078). `fork(entryId,{position})` is agent-session-runtime.ts:262-352.

## Gap in Epsilon (brief's findings)
- No tree surface at all: `read_all` (store.rs:689-695), `leaf_id` (:628-630) and
  `branch_entries` (:636-655, active branch only) are the whole traversal API.
- `navigate_to` (store.rs:699-713) can select any indexed entry but the leaf can never be
  null/root → `resetLeaf` and the user-message→parent case are unrepresentable.
- `BranchSummary` is declared in contracts (session_entry.rs:131-139) with **no producer or
  consumer**: `model_view` drops it via `_ => {}` (store.rs:755) where pi projects it.
- `/fork` in the TUI is a whole-file `std::fs::copy` (app.rs:1446-1477): same id, unrelated
  branches dragged in, no `parent_session`, and the run keeps writing the original file.

## SESSION-01a slice (in flight)
- `crates/epsilon-session/src/store.rs`: `branch_with_summary`; explicit no-leaf root
  navigation; `BranchSummary` projected on the ACTIVE branch in `model_view` (exactly once on
  the new branch, never on the abandoned one; CORE-03 replace-projection still holds);
  `fork_branch_at(leaf_id, dest_dir)` → new file, fresh `ses_` id, v3 header + `parent_session`,
  root→leaf only, `seq` renumbered gaplessly 0..n-1 preserving ids/parent chains, source bytes
  untouched, refuse-don't-truncate on the destination.
- Tests: three session-level regressions (temp dirs) + the loop-shaped alternate-branch
  variant (two branches, compact one, navigate to the sibling, reopen) — the ledger's missing
  F04 variant (audit/02-defect-ledger.md:97-98).
- Milestone doc: `docs/milestones/M90-session-tree.md` (PENDING verification until the battery).

## SESSION-01b (queued, do not silently drop)
- `crates/epsilon-tui/src/app.rs`: `/tree` lists `read_all()` with a leaf marker (replacing
  `branch_entries()` at :1594-1608); Enter applies pi's target-type rule and restores a user
  message's text into the input; `/fork` uses `fork_branch_at` at the leaf or a chosen user
  boundary and switches through `switch_session_file` → `live_resume` (app.rs:644-681).
- Blocker: the TUI file is another work item's uncommitted work — coordinate or land after it.

## Constraints to respect
- ADR-006 append-only log; ADR-007 M86 ordering (durable append before any context
  replacement); F08 refuse-don't-truncate for fork/reopen; v1 migration keeps working
  (session_entry.rs:154-158, store.rs:510-528); contracts unchanged (BranchSummary already
  declared; ADR-017 item 6 freezes shapes).
