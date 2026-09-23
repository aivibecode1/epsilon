# M119 — importing pi's current session files

## What

Slice 4B, the last piece of the session-format port gap. M114 and M115 made pi's **v1** files
readable — roles, content arrays, timestamps, header fields — but a *current* pi file, whose
entries carry `id`/`parentId` (what pi's own `migrateV1ToV2` adds), was refused at line 2:

```
open …: session line #2: valid JSON that is not an entry, a header or a legacy entry —
a shape this build does not know
```

`epsilon --import <pi-file>` now converts one:

```
$ epsilon --import pi-v2.jsonl
imported 1018 entries into …\pi-v2.jsonl.epsilon.jsonl (from …\pi-v2.jsonl)
the source was not modified — delete it yourself once you are happy with the copy
```

## Key decisions

- **Import is a copy, never a conversion in place.** The output is `<file>.epsilon.jsonl` beside
  the source, the source is left byte-identical (hash-verified in the test *and* in the run
  above), and the message says plainly that the original is still there and theirs to delete.
  This is the rule the importer plan asked for and the opposite of what `--resume` does with a
  v1 file — which is exactly why it is an explicit command rather than a side effect.
- **The refusal now carries its remedy.** `--resume` on such a file prints the diagnosis *and*
  the `epsilon --import <path>` command to run, with the path quoted. A refusal a user cannot
  act on is half a feature; this is the same principle as M115 naming where the previous bytes
  went.
- **One converter, not two.** The importer reuses `heal_legacy_message` and the typed-kind
  mappings rather than writing a second v2 converter. A second converter is how the two drift
  and one of them starts emptying messages again — which is the defect M114 existed to fix.
- **No model, no runtime.** `--import` exits before the runtime is built: an import reads one
  file and writes another, and paying for an agent to do it would be absurd.

## Verification

Battery `m119a`: **all five gates exit 0, 1,077 passed / 0 failed / 1 ignored across 60 test
binaries** — one binary and eleven tests more than M118's 1,066. `epsilon-session` 18 unit + 50
integration (up from 16 + 44).

End-to-end, on a real pi file built from pi's own shipped fixture by adding the v2 `id`/
`parentId` envelope (1019 lines):

| | result |
|---|---|
| import | `imported 1018 entries into …pi-v2.jsonl.epsilon.jsonl` |
| source | **byte-identical** (sha256 compared before and after) |
| output | 921.9 KB session beside the 996.4 KB source |

## Limits

- **Branch selection is not audited here.** The plan asked for a decision about sibling branches
  (which chain becomes the imported session when two entries share a parent) and this run does
  not exercise a branched file — pi's fixture is linear. Whatever rule the importer uses, it is
  not proven by this evidence.
- **`label`/`session_info` still heal to `Custom`**, and an unknown kind inside a v2 file is
  preserved rather than refused (the M113 rule carries over) — but only the kinds pi's fixture
  contains were exercised.
- **The importer was written by a delegated agent that failed before reporting.** Its work was
  verified here — it compiles, the full battery passes on it, and the end-to-end run above was
  performed by the parent on the committed bytes — but the slice had no independent read-only
  review, so any residual the author would have disclosed is not on the record.
- Nothing *consumes* `model_change`/`thinking_level_change` on resume (M116's open item), so an
  imported session's model switch is carried faithfully and still does not take effect.
