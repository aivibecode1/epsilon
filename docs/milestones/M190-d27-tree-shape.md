# M190 — D27: the reference's type is not its behaviour

## What

M189 shipped `get_tree` with **the wrong shape**, and this milestone found it, recorded it,
fixed it, and proved the fix with two controls that fail on the defect they name.

`get_tree` now returns pi's nested forest. `get_messages` and `get_commands` return the bare
arrays pi's client unwraps.

## What was wrong

Reading pi's `core/session-manager.ts` — the **implementation**, not the type declaration:

```ts
interface SessionTreeNode {
    entry: SessionEntry;
    children: SessionTreeNode[];
    label?: string;
    labelTimestamp?: string;
}

getTree(): SessionTreeNode[] {
    const entries = this.getEntries();   // the WHOLE file
    // ... roots are parentId === null, parentId === self, OR a broken parent chain
    // ... children sorted by entry.timestamp at EVERY level
}
```

and `SessionEntryBase`:

```ts
{ type: string; id: string; parentId: string | null; timestamp: string }  // ISO-8601
```

What M189 shipped:

```json
{"tree": [{"id": "ent_…", "parentId": null, "seq": 0, "timestampMs": 1790584501907,
           "kind": "message"}], "leafId": "ent_…"}
```

Three separate mismatches:

| | pi | M189 |
|---|---|---|
| structure | recursive `{entry, children, …}` forest | flat array |
| coverage | **every** entry, orphans become roots | the **current branch** only |
| entry fields | `type`, `parentId`, `timestamp` (ISO **string**) | `kind`, `parentId`, `timestampMs` (number), plus `seq` |

A client written against pi walks `node.children`, gets `undefined`, and draws nothing.

**And three more, same severity, found in the same reading:** pi's
`getData<AgentMessage[]>(response).messages` and `getData<RpcSlashCommand[]>(response).commands`
unwrap a **bare array**. M188 wrapped both as `{messages: […]}` and `{commands: […], count}`.
A pi client iterating the reply gets key *names* instead of messages — and `get_commands`'s
invented `count` key means a client reading `.length` gets **1**, the number of object keys,
rather than 35 commands.

## Why it passed 1,446 tests and two controls

Because the tests asserted **my** shape.

`rpc-types.ts` declares `tree: SessionTreeNode[]`. I checked the element type — and
`SessionTreeNode` has a `children` field, which I *did* see, and still produced a flat array.
The declaration gives you the field names of a node. It does not tell you the array is a list
of **trees** rather than a list of entries. Only the implementation does.

The controls were no better. "Bill at the first model" and "`get_tree` returns the whole file"
were both sound tests of *my* contract, and both passed against the broken one, because
`get_tree` never claimed to return the whole file in the first place.

**This is D26 one level up.** D26 was a mis-spelled wire name caught by a test that agreed with
the code by construction. D27 is a mis-shaped *payload*, caught by nothing, because a
declaration is not a description of behaviour.

So the rule now in memory: **reading a reference's TYPE proves the field names; reading its
IMPLEMENTATION proves the structure.** For any payload, the client code that *consumes* it is
the specification — `rpc-client.ts` is where `getData<T>(response).x` tells you whether `x` is
an object or an array, and no type declaration will.

## The fix

`build_tree` reproduces pi's rules rather than approximating them:

- every entry becomes `{entry, children, label: null, labelTimestamp: null}`;
- a root is `parentId == None`, `parentId == id`, **or a parent not in the file** — orphans
  become roots, so a truncated or hand-edited JSONL renders everything;
- children sorted by timestamp ascending at every level;
- **a parent that is an ancestor of the child is promoted to a root.** pi's walk would
  strand an `a → b → a` cycle, because no node in it is a root and the recursion never
  reaches it. Two entries silently lost. `reaches()` walks up from the proposed parent and
  turns that case into a root, so both entries render exactly once.

Labels are `null` rather than absent, so a client reading `node.label` sees null instead of
`undefined`.

`entry_as_pi` emits `parentId` camelCase and `epoch_millis_to_iso8601` produces a UTC
ISO-8601 string with millis and a `Z` — matching JavaScript's `toISOString()`. Hand-rolled
(Hinnant's days-from-civil, attributed in the doc comment) rather than taking a date
dependency for one formatting call.

The recursion is safe because the link pass breaks every cycle *before* the emitter runs, so
depth is bounded by the entry count. That is the same guarantee pi's explicit stack buys, by
a cheaper route: pi cannot mutate its input, so it has nowhere to record the decision.

## Key decisions

**`get_tree` reads `read_all()`, not `branch_entries()`.** The branch walk is the right
answer to a *different* question — what the current leaf's ancestry is — and `get_entries`
plus the leaf is enough to reconstruct that client-side. `get_tree` is the whole file's shape.

**`get_messages` and `get_commands` return bare arrays.** Adding a `data`-shaped wrapper was
M188's invention, and pi's client does not unwrap one.

## Verification

| Control | Result |
|---|---|
| **D** — `get_tree` reverted to M189's flat shape | **2 failed**: *"every node needs a children ARRAY"*; an entry in the file but missing from the tree |
| **E** — `get_messages`/`get_commands` back to M188's wrappers | **4 failed** |

- `epsilon-cli --lib`: **114 passed**
- Focused rpc: **49 passed**
- Battery: `m190` — see `audit/evidence/gate-m190.md`

## What nothing exercises

- **A parent cycle is handled by inspection, not by a test.** `reaches()` is the only
  protection against an `a → b → a` file silently losing two entries, and no test builds
  one. The orphan and self-parent root cases are the same untested code path — the branch
  they came from (pi's `getTree`) has no test in this repo either.
- **`epoch_millis_to_iso8601` is unverified against a reference.** No test compares its
  output to JavaScript's `toISOString()` for a known instant, and the leap-year and
  pre-epoch-adjacent cases are arithmetic I reasoned about rather than measured. A property
  test round-tripping through a parser would be the honest check; there is not one.
- **`get_tree` on a session with compaction entries** is untested. `read_all` returns every
  kind and `wire_tag()` is applied uniformly, so a compaction entry renders as a node — but
  nothing asserts that.
- **No installed-binary stdio probe has yet confirmed the new shapes on the wire.** The unit
  tests call `dispatch_line` in-process; the corrected release is not installed.
- **The other 12 unimplemented pi commands are untouched** — this milestone was a correction,
  not an advance in breadth.
