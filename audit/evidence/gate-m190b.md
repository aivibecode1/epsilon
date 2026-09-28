# Gate `m190b` — D27: `get_tree` rebuilt as pi's nested forest

**Verdict: GATE PASS.** 63 targets, **1,447 results, 0 failed.**

| Stage | Exit | Time |
|---|---|---|
| `fmt --check` | 0 | 3s |
| `clippy` | 0 | 7s |
| `test` | 0 | 223s |
| `demo` | 0 | 5s |
| `real-check` | 0 | 1s |

- **Log:** `audit/evidence/gate-m190b.log`
- **Source commits:** `b2b06c5` (D27 fix + record), `d4df88e` (unused-parameter fix)
- **Baseline:** M189B — 1,446 results, 63 targets
- **Delta:** **+1**, `epsilon-cli --lib` (113 → 114). One test split into two: the
  single `get_tree` test became *is a nested forest* and *includes off-branch entries*.

## The failed predecessor

`m190` (first attempt) is preserved at
`audit/evidence/gate-m190-FAILED-clippy-unused-depth.log`. **clippy failed** on
`parameter is only used in recursion` — the `depth` argument threaded through the new
tree-walk test helper, which was never read. `fmt`, `test` (223s→277s, 114 passing),
`demo` and `real-check` all passed.

The parameter is now removed, and a comment in the helper says why, so the next reader
does not add it back: the depth is measured once afterwards by `deep()`.

## Controls

Both run before the battery, each failing on the defect it names:

| Control | Result |
|---|---|
| **D** — `get_tree` reverted to M189's flat `branch_entries()` array | **2 failed**: *"every node needs a children ARRAY (pi's SessionTreeNode)"*; an entry present in `get_entries` but **missing from the tree** |
| **E** — `get_messages`/`get_commands` restored to M188's `{messages}` / `{commands, count}` wrappers | **4 failed** |

Control D's second failure is the more interesting one: the flat handler silently dropped
every entry that was not on the current branch, which is a data-loss defect a
shape-only test would have missed.

## What these controls are evidence *of*

M189's `get_tree` passed 1,446 tests and two earlier controls. The tests asserted the
shape I had guessed, and the controls tested properties of that guess. Both agreed with
the code by construction — D26's failure mode, one level up, on payload structure
instead of a wire name.

Controls D and E are different in kind: they were written by re-reading pi's
`session-manager.ts` implementation and `rpc-client.ts` client, and they fail against
the old handlers. That is the point of running them.

## Not verified here

- **No installed-binary probe.** The release on disk is the M188 build; the D27 shapes
  have only been exercised in-process through `dispatch_line`. A wire check of
  `get_tree`/`get_messages`/`get_commands` on the installed binary is the next step and
  is recorded in `docs/milestones/M190-d27-tree-shape.md` under "what nothing exercises".
- `epoch_millis_to_iso8601` has no reference-comparison test; the leap-year arithmetic is
  reasoned, not measured.
- The parent-cycle branch of `reaches()` is untested — pi has no equivalent test either.
