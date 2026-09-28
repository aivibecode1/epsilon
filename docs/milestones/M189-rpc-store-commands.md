# M189 — Three store-backed RPC commands, and two controls that taught the same lesson

## What

`get_entries`, `get_tree`, `set_session_name`. **18 of pi's 33 RPC commands now work.**

M188 closed by naming two things it had not verified. This milestone closed one of them —
the two-model session — and then found, in the other, that two of my own tests were not
testing what I thought.

## The two-model session: the gap M188 named

M188 shipped `session_usage_by_model` on the argument that *price belongs to the model that
consumed the tokens*, and asserted it through one model's worth of rows. That is half the
claim. This is the other half: two runs, two models, two rows, each priced with its own rate.

The faux provider reports whatever `ModelRef` it is handed (`faux.rs`:
`build_events(&model.id, step)`), so switching `Agent::set_model_id` between runs produces
genuinely differently-billed messages with no credential and no network. `faux-1` is priced
at 0/Mtok and `deepseek/deepseek-chat-v3.1` at a real rate, so the session's cost must be
exactly the priced row — and `0.0` would be the wrong answer in a way a single-model test
cannot see.

**The first version of this test failed, and the cause was its own precondition.** Both runs
came back billed to `faux-1`, so the map had one row. `Agent::new` defaults to `faux-1`
(`epsilon-core/src/lib.rs`) — the model is priced at *zero* — so "run 1 on the priced model"
was a comment rather than a fact. That is the M186 lesson again, in a new costume: **the
precondition a test depends on must be the thing the control would break.**

## `clear_queue` returned the wrong shape, and reading pi properly is what showed it

pi's `RpcResponse` declares:

```ts
command: "clear_queue"; data: { steering: string[]; followUp: string[] };
```

**`string[]` — the cleared messages' text, not a count.** I had written `len()` for both,
which satisfies "both keys are present" and is what anyone would expect from a clear. It is
also useless: a client that wants to show the user what it just dropped has nothing to
show, and a user who regrets a clear cannot put the work back.

Corrected to the text. The assertion got stronger for free — a count of 1 is satisfied by
the *wrong* message; `["actually stop"]` is not.

This needed `Message::text()` in `epsilon-contracts`, which did not exist. It joins every
text block rather than returning the first: a two-paragraph message is one message, and half
of it in front of a user deciding whether to restore it would be worse than none. Image
blocks contribute nothing — a base64 payload rendered into a text field is not something to
hand a client for display.

## Two controls that passed, and what that meant

This is the part worth keeping.

### Control A: billed every response to `self.model_id()`

The obvious wrong implementation: use the session's current model rather than the message's
own. **All 48 tests passed.**

The production code was correct. My control was a no-op, because the fold happens *during*
each run, when `model_id` is already that run's model — so the two expressions are the same
value. The replacement that genuinely differs is "bill at the session's **first** model",
and it failed immediately, naming the merged row:

```
{"deepseek/deepseek-chat-v3.1": {input: 200, output: 100}}   // one row, both models
```

### Control C: `get_tree` returning the whole file

Replacing `branch_entries()` with `read_all()` also left all 48 green. **On a linear session
the branch *is* the file**, so `get_tree` and `get_entries` are indistinguishable and no test
can tell them apart.

The fix is a real fork, and getting one took three attempts:

| Target navigated to | branch | file | Why it failed |
|---|---|---|---|
| first message entry | 6 | 6 | everything after it is still on the branch |
| last message entry | 6 | 6 | nothing is stranded |
| **middle message entry** | 5 | 6 | entries on both sides — the ops differ |

Confirmed by probe before rewriting the test:

```
before nav: (file=4, branch=4, leaf=ent_…4deba)
after nav:  (file=4, branch=3, leaf=ent_…4ddd7)
after run3: (file=6, branch=5, leaf=ent_…4dfbf)
```

Then control C failed, with the test reporting *its own* weakness rather than a bare number:

> the fork must make the branch SHORTER than the file, or this test cannot distinguish the
> two ops: branch=6 file=6

**A test that cannot fail should say so in its failure message.** That string is why the
second and third attempts were diagnosable at all.

## Key decisions

**`get_tree` reads the store's own branch walk**, not a second parent-follower here.
`branch_entries` already handles parent cycles, `seq`/line mismatches and truncated chains —
all real conditions in a hand-edited or half-written JSONL file. A second walker would be a
second set of answers about the same data, disagreeing exactly when it mattered.

**`get_tree` nodes carry linkage, not payloads.** `{id, parentId, seq, timestampMs, kind}`;
the entries themselves are one `get_entries` away. A tree of a thousand messages should not
re-send a thousand bodies to render a thousand lines.

**An unresolvable `since` returns everything, not nothing.** A `since` the client cannot
resolve means it has fallen behind, and an empty reply would have it conclude the session is
empty. Over-sending is recoverable; under-sending looks like data loss.

**`set_session_name` writes a `Custom { "session_name" }` entry**, the same shape `/name`
writes and `list_sessions` reads. A parallel header field would be invisible to both. The
test reads it back through `epsilon_session::list_sessions` — the call the resume picker
makes — so an entry the discoverer cannot read fails there rather than in a hand-rolled parse.

**The three store-backed ops refuse by name when no store is attached.** `entries: []` reads
as "this session has no history" and sends a client hunting a bug that is not there.

## Verification

Three controls, each failing on the defect it names:

| Control | Result |
|---|---|
| bill at the session's **first** model | FAILED — one row, both models merged |
| `get_tree` returns the whole file | FAILED — `branch=6 file=6`, test names why |
| (`get_tree` on a **linear** session — control C's first run) | **PASSED — control invalid** |
| (bill to `self.model_id()` — control A's first run) | **PASSED — control a no-op** |

- `epsilon-cli --lib`: **113 passed** (105 + 8)
- `epsilon-cli` rpc-focused: **48 passed** (40 + 8)
- Battery `m189`: see `audit/evidence/gate-m189.md`

## What nothing exercises

- **No real editor client was driven over stdio for these three.** `dispatch_line` is
  in-process. `get_entries` on a large session is the framing risk — the two-message case is
  ~1 KB on one line, and the largest reply actually measured remains the 365 KB model list.
- **No session with a real compaction entry was put through `get_entries`.** The store's
  `read_all` returns every kind, and the `kind` tag round-trip is asserted only for messages.
- **The `since` filter is tested at two positions** (first entry, and an unresolvable id).
  A `since` naming the *last* entry — the case a polling client hits most — returns an empty
  array, which is correct but unasserted.
- **`get_tree` is tested on a linear-then-forked session, not a deep multi-branch tree.** The
  cycle and `seq`-mismatch handling is `branch_entries`' own tested behaviour; this checks
  that the RPC surface *uses* it, not that the walk is correct.
- **Cache columns remain unasserted** — the faux provider does not bill cache traffic.
