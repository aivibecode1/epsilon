# M228 — two rows that were wrong about the code, and one of them wrong about the reference

**No production code changed.** This round closed A7 and reclassified A23, both by measuring the tree
rather than by acting on the rows' descriptions. Three ledger rows in a row have now turned out to be
stale (A15 last round, A7 and A23 here), which is itself a finding worth recording: **the ledger's
prose ages faster than the code it describes**, and a row that has not been re-read is a claim about a
program that no longer exists.

## A7 — "a killed TUI blocks the workspace for 30 s"

Both clauses were stale.

**The 30 seconds is a ceiling, not the cost.** `STALE_AFTER = 30s` (`lease.rs:88`) bounds the wait
when a holder **is** heartbeating — a second window genuinely using this session. A holder that is not
heartbeating is taken after `DEAD_HOLDER_SETTLE = 4s` (`lease.rs:129`), and a lease test asserts the
steal finishes in under five seconds, in those words:

```rust
assert!(elapsed < Duration::from_secs(5),
        "took {elapsed:?} — the point is that it is nowhere near the 30s window");
```

So what the row describes — a relaunch straight after End Task — costs about four seconds. That is
M210/M211's work, recorded and tested; the row simply never learned it.

**"The TUI shows nothing" was A18, not A7**, and it was closed at M214: the wait notice is built at
`main.rs:2175`, pushed into `startup_notices` at `main.rs:2393-2397`, and drained into the transcript
by the event loop at `app/event.rs:73`. The row was describing a defect that had been fixed two
milestones earlier and filed under the wrong number.

## A23 — the em-dash sweep would have been a downgrade

The row said "~200 drawn strings — help text, notices, tool descriptions — contain `—`" and asked for a
sweep. Measured before acting:

| claim | measurement |
|---|---|
| ~200 drawn strings | **133 lines** carry one inside a string literal (the "~200" counted comments; 13 of the 133 are test fixtures) |
| implicit: this harness should not use one | **the reference uses them too** — 103 occurrences across 21 files of `references/pi-0.85.1` |

So the em dash is this family's **prose convention**, not a legacy artefact, and replacing 133 drawn
occurrences with `--` would degrade what every modern terminal displays in order to tidy a console the
project no longer targets as a default (M219 made ASCII the default there for a reason).

The ASCII promise M219/M220 established is about **structural** characters — borders, markers, arrows,
truncation marks, the panel dash — precisely because prose is not structure. A sentence is allowed to
be a sentence.

**The real fix, recorded but not done:** map unsupported punctuation at *draw* time, in one place where
the repertoire is known. `Glyphs` already substitutes `dash`, `ellipsis` and `up`/`down`, and the M220
frame check is the hook: a test that paints a notice in the ASCII repertoire would fail on the
characters the renderer cannot substitute, and the substitution belongs next to the drawing rather
than in 133 string literals.

## Verification

Docs-only: `check-links.py` PASS (**551 relative links, 0 broken**). No battery is claimed because no
code changed — and the measurements behind both rows are cited by file and line in the ledger, so a
reader can check them in one look:

- `lease.rs:88` (`STALE_AFTER`), `:129` (`DEAD_HOLDER_SETTLE`), `:941-948` (the timing assertion)
- `main.rs:2175` (notice text), `:2393-2397` (into `startup_notices`), `app/event.rs:73` (drained)

## Limits

- **A7's closure is a reading, not a run.** The lease test asserts the dead-holder steal is under five
  seconds on a healthy machine; I did not re-time a real killed TUI this round. The row says so, and
  the *test* is the standing evidence.
- **A23's reclassification depends on the reference tree being present.** It is, and it was measured
  in this round; a contributor without `references/` would have to re-derive the 103 occurrences
  before agreeing.
- **No prose substitution exists**, so a legacy console still shows a box inside a notice. That is
  accepted rather than hidden — and it is the smaller half of the original report, the structural half
  having been fixed at M219/M220.
