# Gate evidence — m185

Closes the two limits m184 recorded against itself: `/templates`, and the live verification
of template expansion that m184 could not perform at the time.

- **Source commit:** `24247f2`
- **Log:** [`gate-m185.log`](gate-m185.log) — tracked, complete, ends in `GATE PASS`
- **Build:** `CARGO_BUILD_JOBS=1`, `CARGO_TARGET_DIR=D:/eps-target`, Windows via
  `pwsh -File scripts/gate.ps1`

## The five gates

| Gate | Exit | Time |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | 2 s |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | 11 s |
| `cargo test --workspace` | 0 | 295 s |
| `cargo run -q -p epsilon-cli -- --demo` | 0 | 5 s |
| `cargo check -p epsilon-cli --features real` | 0 | 3 s |

## Test totals

**63 targets, 1,411 results, 0 failed** — 51 test binaries + 12 zero-test doc-test targets.

`m184` measured **1,406** across the identical 63-target structure. Delta **+5**, fully
attributed:

| Crate | Before | After | Delta |
|---|---|---|---|
| `epsilon-tui` | 385 | 390 | **+5** (2 `/templates` tests + 3 doc-drift tests) |

## What was added after m184 passed

### `/templates` — the lookup the feature was missing

m184's evidence file said a user who forgot a template's name had no way to recall it. A
count is not recall, so `/templates` lists names **with descriptions and argument hints**.
Descriptions, because the purpose of a template is that you can forget its body.

Control — suppressing the listing:

```
templates_command_lists_names_descriptions_and_hints  FAILED
  -> "the name must be listed: \"\""
templates_command_explains_where_to_put_files_when_empty  PASSED
```

The empty-case test passing is **correct, not a gap**: it asserts the empty message and
cannot see a populated one. Recorded because a control that breaks only some of a feature's
tests is a weaker signal than one that breaks all of them, and the cause is per-test scope
rather than a dishonest test.

### Doc drift — three tests over `docs/user/prompt-templates.md`

Every placeholder the code implements is documented; the directories match what discovery
scans; the "does not work" section still says so. The last is load-bearing: a feature page
listing only what works invites people to build on capabilities that do not exist, and when
`--prompt` is added the test fails so the line is removed deliberately.

All three controls caught:

| Control | Failure message |
|---|---|
| `${@:2}` → `${@:3}` | `does not document ${@:2}` |
| `.agents/prompts/` → `.agents/prompt/` | `does not mention .agents/prompts/` |
| "cannot be nested" removed | `the guide no longer states: cannot be nested` |

**A first run of the directory control passed, and the cause was mine.** It replaced one of
two occurrences, so the correct path survived elsewhere in the file. A control that does not
actually remove what it claims to remove proves nothing about the test. Renaming every
occurrence failed it, which is the result that means something.

## Live verification — the m184 limit, now closed

A `.epsilon/prompts/zzprobe.md` driven through `scripts/console-probe.py` in a real conhost
window against the debug binary:

| Typed | Screen |
|---|---|
| `/templates` | `prompt templates: /zzprobe — Probe template for live verification (takes <topic>)` |
| `/zzprobe auth-tokens` | `❮ ❯ ZZPROBE_MARKER reviewed auth-tokens for correctness.` |

`ZZPROBE_MARKER` exists only in the template body, so the screen proves **expansion**, not
echoing, and `auth-tokens` came from real keystrokes. The probe directory was removed
afterwards.

**The first probe run was inconclusive and the cause was mine**: the probe typed
`/templates` without `--enter`, so nothing was submitted and the notice never appeared. An
absent result is not evidence of a defect until the instrument has been checked.

## Bugs found, all before anything was user-visible

Carried forward from m184 for completeness — all in the substitution grammar, all caught
before the TUI was wired:

- `${@:2}` and `${@:-default}` both begin `@:`; treating form-detection as a question about
  branch **ordering** took three attempts, each fixing one form by breaking another. The
  original tests covered each form in isolation, which is why reordering looked like a fix
  each time. The form is decided by one predicate: a slice is `@` `:` DIGIT.
- `${@:2}` skipped nothing — after `'@'` the remainder is `":2"`, so splitting on `:` put
  the empty string first, which defaulted to 1.
- `$$` was never implemented as an escape.
- Two of my own test expectations were wrong and the implementation was right: a raw string
  `r#"..."#` cannot contain its own delimiter, so my "quoted argument" test had no quotes in
  it; and `lines().join("\n")` drops a trailing newline.
- A trailing-newline expectation in the TUI test — the same fact the contracts test had
  already established, missed on the second consumer.

## What was not verified

- **The installed release binary was not rebuilt.** `C:/Users/Hp/.cargo/bin/epsilon.exe` is
  the 06:22 build, which has neither the turn hooks nor prompt templates.
- **The template body is not shown to the user before it is sent.** Expansion is silent, so
  a template that produces a surprising prompt is invisible until the model responds to it.
  A preview is worth building and is not built.
- **Template expansion was not exercised in a live *streaming* turn.** The probe submitted
  with `--script` and confirmed the prompt reached the agent; it did not confirm the frame
  stays responsive while a template-expanded turn streams.
- **No third-party extension has been used.** Every extension proof still uses
  `ext-fixture`, a binary in this repository.
