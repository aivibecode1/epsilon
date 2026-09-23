# M121 — the signature pi wrote, and the cap `glob` was missing

Two follow-ups that earlier rounds found and queued rather than rushed: a field-level loss the
import reconciliation turned up, and the output bound M118 deliberately left alone.

## What

**`thinkingSignature` is carried.** When pi's thinking parts were imported, the signature was
dropped — `ContentBlock::Thinking` had no field for it. Measured before the fix: 1 occurrence in
`large-session.jsonl` and 49 in `before-compaction.jsonl` (47 non-empty, 2 written empty).
`Thinking` now has an optional `signature`, carried through the **one** content-part conversion
both readers use (`legacy_block`: `heal_legacy_message` for v1 files, `heal_pi_entry_kind` for the
importer). The importer itself needed no change — which is the payoff of M119's decision to reuse
the healing path instead of writing a second converter.

**`glob` has a byte cap.** M118 capped `find` and `ls` at 50 KiB and left `glob` on the grounds
that it advertised no bound — true, and still an asymmetry: 5000 matches reached the model as
~150 KB where `find` stops at 50 KB.

```
[50.0KB output limit reached: 84 more file(s) match and are not shown. Use a narrower pattern to see the rest]
```

## Key decisions

- **Absent and empty are different.** pi writes `thinkingSignature: ""` twice in its own fixture;
  `Some("")` is carried as written rather than folded into `None`. A signature that was empty is
  a fact about the source, and normalising it away loses the distinction between "pi wrote
  nothing" and "this block predates signatures".
- **`skip_serializing_if` is a promise, so it gets a byte-level test.** A thinking block with no
  signature must serialize *exactly* as it did before the field existed. The test asserts the
  literal bytes, and the negative control (removing the skip) shows the failure it prevents:
  `{"type":"thinking","text":"…","signature":null}`.
- **Only the bound that actually fired is announced — and for `glob` the *walk* wins.** This is
  the opposite winner from `find`, deliberately: when the walk stopped early, the byte notice's
  own claims ("N more file(s) match", "use a narrower pattern to see the rest") are **false about
  files that were never visited**. The cap is still applied so the count stays honest. In `find`
  both bounds are display bounds, so there the more specific one can speak.
- **A report that has stopped being true is a bug.** The reconciliation test used to print
  "thinkingSignature this build has no slot for: N". Once the slot existed that sentence would
  have been the *evidence* lying, so it now compares signature values and asserts the source
  counts, which also stops an empty mismatch list from being vacuous.
- **The field carries session-file values only.** Nothing populates it from our own provider
  streams yet — Anthropic's `signature_delta` is unparsed and thinking blocks are not replayed to
  the API. Said plainly in the code, because an unset field that looks supported is worse than a
  documented gap.

## Verification

Battery `m121a`: all five gates exit 0. Focused suites before it: `epsilon-contracts` 34,
`epsilon-session` 18 + 5 + 52, `epsilon-tools` 100 + 57 — 266 passed, 0 failed.

**Four negative controls**, each reverting a fix in place, running the test that names it, and
restoring byte-identically (hashes recorded) with the files **touched** so cargo could not run the
control binary — the trap `AGENTS.md` now warns about:

| control | what failed |
|---|---|
| signature extraction → `None` | the two session tests: `pi's thinkingSignature must arrive on the block` (`left: None, right: Some("EpEQ…")`) |
| `skip_serializing_if` removed | the byte assertion: `…,"signature":null}` versus the old bytes |
| glob cap removed | `the byte bound must name itself` (all 400 rows returned) |
| glob precedence flipped | `the walk budget must be announced, not silent` |

External proof on disk: a fresh import now contains `"signature":"` once in `large-session` and
49 times in `before-compaction`.

## Limits

- **Nothing sets `signature` from a live provider stream.** An imported session carries it; a
  session this build produced will not. Anthropic's `signature_delta` is unparsed.
- **`find`'s own walk-bound truncation is still unannounced** — M118 fixed `find`'s result cap,
  not its walker. `glob` is now ahead of `find` on this one narrow point.
- **`glob` cannot say how many rows the byte cap hid** when the walk also stopped — the same
  accepted limit `ls` has when bytes cut inside its entry cap.
- **A thinking block in *user* content drops its signature**: `user_content_from_block` folds it
  into `<thinking>` text and there is no thinking variant in `UserContent`. Not reachable for pi
  assistant content, which is where signatures live.
- No ADR accompanies this change. M117–M119 added none either, and the field is additive and
  optional in both directions — but a reader looking for the decision will find it here and in
  the code comment, not in `docs/design/epsilon-decisions.md`.
