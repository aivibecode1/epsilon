# M184 — Prompt templates

Phase 4 item 4.2, and the cheapest genuinely-missing pi feature in the port.

## What

Type `/review src/parser.rs` and the model receives the body of your `review.md` with `$1`
replaced. A template is a saved prompt you send by name.

The whole feature is three pieces:

| Piece | Where | Commit |
|---|---|---|
| Substitution engine | `epsilon-contracts/src/prompt_template.rs` | `3902461` |
| TUI wiring | `epsilon-tui/src/app.rs`, `app/event.rs` | `d9efe9f` |
| `/templates` lookup | `epsilon-tui/src/app/commands.rs` | `debcef2` |
| User guide | `docs/user/prompt-templates.md` | `44a4433` |
| Doc-drift tests | `epsilon-tui/src/app/tests.rs` | `c6eee08` |

## Key decisions

**The engine lives in `epsilon-contracts`, not the TUI.** Two callers need it — the TUI
expanding what you typed, and eventually a CLI flag — and the substitution rules are the
part that must agree between them. A second copy of `substitute_args` would be a second set
of semantics to keep in step, which is exactly how a "faithful port" quietly stops being
one. The crate gains no dependency: the scanner is hand-written and the frontmatter parser
is the flat one `skills.rs` already uses.

**Expansion happens before slash-command parsing.** Expanding after would make every
template name a candidate for a command name, `/help` would have to arbitrate between two
things legitimately called `/help`, and a repository could never ship a `review` that does
something the built-ins cannot. A text naming no template is returned unchanged, so every
existing path is untouched when no templates are installed.

**Discovery mirrors skills exactly** — `~/.config/epsilon/prompts/` plus
`<workspace>/{.agents,.epsilon}/prompts/`. A user who has learned where their skills live
has learned where everything project-scoped lives. Project scope wins on a name clash.

**Cached, not read per submit.** Expansion runs on every message; walking two directories
for files that have not changed would tax the one path that must stay fast. `/reload`
refreshes, same as for skills.

**Two deliberate supersets of pi.** `${1}` and `${@}` are braced forms pi's regex does not
match. Sending someone `${1}` to the model verbatim is worse than accepting a form the
reference happens to reject.

## Gotchas

### `@:` is two different things, and I got it wrong three times

`${@:2}` is a slice. `${@:-default}` is a default. Both begin `@:`.

I treated form-detection as a question about branch **ordering**. It is a question about the
**text** — a slice is `@` `:` DIGIT.

| Attempt | What broke |
|---|---|
| defaults first | `strip_default` takes the `2` as its fallback; every slice test failed |
| slices first | `${@:-default}` becomes unreadable |
| digit bail-out | the default returns its own literal text |

Each fixed one form by breaking another. **The original tests covered each form in
isolation**, and an isolated test cannot see a collision — which is why reordering looked
like a fix every time. The fix is one predicate at the top of `expand_braced`, and the five
collision tests now sit together rather than scattered.

### Two more found by tests, before anything was wired

- **`${@:2}` skipped nothing.** After the `'@'` is stripped the remainder is `":2"`, so
  splitting on `:` put the *empty* string first, which defaulted to 1. Found by a temporary
  probe printing intermediates — not by reading the code, which looked right.
- **`$$` was never implemented.** Both dollars were each treated as unrecognised, so `$$HOME`
  came back verbatim. The escape is checked before the general parse, so `$$1` is one dollar
  followed by argument 1 rather than argument 1 twice.

### Two of my own test expectations were wrong, and the implementation was right

- A raw string `r#"..."#` **cannot contain its own delimiter**, so my "quoted argument is one
  argument" test contained no quotes at all and was asserting the wrong thing.
- `lines().join("\n")` drops a trailing newline, so expecting a trailing `\n` could never
  hold. Normalising is right for a prompt; the test now says why.

## Test-caught bugs in the *tests*

This slice produced the fourth instance of a defect class this repository has now hit four
times: **a test that calls the new function directly cannot see whether anything calls it.**

The first three TUI tests called `expand_template` directly. A control that removed the call
from `submit_editor` left **all three passing** — the engine was reachable from a test and
from nowhere else, which is the `ToolHooks`/`TurnHooks` defect reproduced on the very slice
whose purpose was to avoid it.

The control:

```
real     -> 4 passed
control  -> 1 FAILED, 3 passed
  the expanded body must reach the transcript:
  "unknown command '/audit' — /help lists commands"
```

The fix is one test that types the command and submits through `type_and_enter`, because
that is the only path a user takes. **The three direct tests are kept** — they test the
engine's contract, which is worth having — but they are not wiring evidence, and the commit
message and the test's own doc comment now say so.

## Verification

- `epsilon-contracts`: 74 passed (39 template tests)
- `epsilon-tui`: 390 passed
- Battery `m184`: **1,406 results, 0 failed** across 63 targets — exactly +45
- Battery `m185`: **1,411 results, 0 failed** — exactly +5
- Live, in a real conhost window: `/zzprobe auth-tokens` → `ZZPROBE_MARKER reviewed
  auth-tokens for correctness`, where the marker exists only in the template body

## What nothing exercises

- **The template body is not shown before it is sent.** Expansion is silent, so a template
  producing a surprising prompt is invisible until the model responds. A preview is worth
  building.
- **Expansion was not exercised during a live *streaming* turn.** The probe confirmed the
  prompt reaches the agent; it did not confirm the frame stays responsive while a
  template-expanded turn streams.
- **No template shipped as a default.** `prompts/` is empty everywhere, so every claim rests
  on `ext-fixture`-style probes and unit tests rather than a template a user would actually
  have written.
