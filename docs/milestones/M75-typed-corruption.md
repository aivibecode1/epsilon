# M75 — typed corruption reasons: "damaged" and "newer than me" are different

**Scope:** port-plan item 13 (pi `harness/reducer.ts:22-44, 312-390`; DSH `core/session/src/types.ts:49`),
plus the piece M70 left open (the session-entry *kind* and the `/session` count).

## The problem

The store reported every unusable line as `corrupt session line #5`. Two unrelated situations
produced that message:

1. the line really is damaged — a partial write, an edit, hand-edited JSON;
2. the line is **perfectly valid and written by a newer Epsilon**, carrying an entry kind this build
   has never seen.

Case 2 is not corruption at all. It is a version skew, and the honest answer is "this file came from a
build that knows more than I do", not "your session is corrupt" — a message that sends a user looking
for a fault that is not there, and (worse) makes deleting a healthy session look like the fix. pi
classifies a contradiction into typed reasons for the same reason; DSH refuses an unknown event
*type* rather than skipping it. This is that rule at the line level.

## What landed

**Typed line diagnoses** (`epsilon_session::LineDiagnosis`, exported from the crate root):

| Diagnosis | Meaning | Reported |
|---|---|---|
| `NotJson` | partial write or damage | refused, line named |
| `UnknownEntryType { entry_type }` | a session from a newer build | **refused by name**, with the type quoted |
| `SecondHeader` | a second header line, mid-file | refused |
| `LegacyInCurrentFile` | a v1-shaped entry in a current-version file | refused (healing it in place would change its meaning) |

`StoreError::Corrupt { line, diagnosis }` replaces `CorruptLine(usize)`, and `Display` now explains:
*"session line #2: entry type `telemetry_v9` is not known to this build — the session was probably
written by a newer Epsilon. Refusing rather than dropping the line, because skipping it would change
the transcript silently."* The refusal is unchanged in behaviour (the file is left intact, as before);
what changed is that the reason is now checkable in code, not just readable in a string.

**Typed repair kinds** (`epsilon_session::repair::RepairKind`): `DanglingToolCall` and
`MissingTurnEnd`, with `RepairReport::kinds()` (kind + count) and `summary()` (one readable line per
kind). The synthesized tool result now carries `details.repair = "dangling_tool_call"` beside the
`interrupted` marker frontends already read, so the *entry itself* says which repair produced it.

**Surfaced, not buried.** `Agent` keeps the resume's repair report (`Agent::repair_report()`), `/session`
prints it, and the CLI's resume path prints the same lines instead of a bare count. "3 tool results
synthesized" and "the session ended mid-run" are different things to have lost, and a count cannot
say which.

## Key decisions

- **Only two repair kinds exist**, because only two shapes can be produced. pi's twelve reasons
  describe a reducer with more states; a reason type for a state that cannot occur is a claim nobody
  can test. The enum exists so callers report the *kind* rather than a count.
- **A torn tail is not corruption, and that distinction is the point.** A crash mid-write leaves a
  partial final line; that is expected, so it is dropped silently as before — even when the surviving
  JSON happens to be an unknown type, because a torn line cannot be trusted to mean anything. Damage
  *mid-file* is refused; a partial *last* line is normal.
- **`LegacyInCurrentFile` only fires with a current-version header present.** A headerless file is a
  v1 file and its legacy lines are healed — that behaviour is unchanged, and the mutation pass proved
  the test had to carry a real header to reach the refusal at all (my first version of the test
  asserted a refusal the store correctly does not perform).
- **The refusal keeps the file byte-identical** — asserted, not assumed.
- **`read_entry` diagnoses the same way a scan does**, so reading one entry back reports the type
  rather than only the line number.

## Verification

| Mutation | Result |
|---|---|
| diagnosis always `NotJson` | 2 tests fail (the type is no longer named) |
| corruption never reported | 4 tests fail — including the two pre-existing ones, which is the check that this refactor did not weaken them |
| `kinds()` always empty | the repair-report test fails |
| the synthesized entry stops naming its repair | the repair-report test fails |

New tests: 3 in `crates/epsilon-session/tests/session_store.rs` (the unknown-type refusal with the
type asserted and the file verified untouched; the three diagnoses distinguished from each other;
a torn tail still tolerated in both shapes) plus the repair-kind/marker test, and 1 in the TUI pinning
that a clean session prints nothing about repairs.

Gate: `scripts/gate.sh m75` — fmt clean, clippy zero warnings, **694 tests, 0 failed**, demo
completes, `real`-feature build compiles.

## Plan status

Item 13 is **ported**. That closes **Tier 3**. Tier 1, 2 and 3 are now complete; Tier 5's product
surface items (LSP diagnostics, workspace checkpoints, prompt templates, session stats, persistent
PTY) remain, each an *L*-sized design question rather than a slice.
