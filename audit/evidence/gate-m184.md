# Gate evidence — m184

Prompt templates: the substitution engine (`3902461`) and the TUI wiring (`d9efe9f`), plus
the user guide (`44a4433`) and the audit updates (`940f710`).

- **Source commit:** `940f710`
- **Log:** [`gate-m184.log`](gate-m184.log) — tracked, complete, ends in `GATE PASS`
- **Build:** `CARGO_BUILD_JOBS=1`, `CARGO_TARGET_DIR=D:/eps-target`, Windows via
  `pwsh -File scripts/gate.ps1`

## The five gates

| Gate | Exit | Time |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | 3 s |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | 67 s |
| `cargo test --workspace` | 0 | 636 s |
| `cargo run -q -p epsilon-cli -- --demo` | 0 | 98 s |
| `cargo check -p epsilon-cli --features real` | 0 | 19 s |

## Test totals

**63 targets, 1,406 results, 0 failed** — 51 test binaries + 12 zero-test doc-test targets.

`m183b` measured **1,361** across the identical 63-target structure. The delta is **exactly
+45**, fully attributed:

| Crate | Before | After | Delta |
|---|---|---|---|
| `epsilon-contracts` | 34 | 74 | **+40** (39 template tests + 1 pre-existing) |
| `epsilon-tui` | 380 | 385 | **+5** (4 template tests + 1 pre-existing) |

No per-binary subtraction was needed to establish that, so none was performed.

## Negative controls

### The substitution engine

Dropping the 1-based to 0-based conversion in the slice parser:

```
real     -> 40 passed
control  -> 5 FAILED, all slice tests
```

A **first** control that changed the parse default **passed**, and that is worth recording:
the default is unreachable, because the digits predicate already guarantees the first part
parses. A control that changes dead code proves nothing. The comment at that line now says so
rather than describing the fallback as if it were live.

### The TUI wiring — the one that mattered

Removing the `expand_template` call from `submit_editor`:

```
real     -> 4 passed
control  -> 1 FAILED, 3 passed
the expanded body must reach the transcript: "unknown command '/audit' — /help lists commands"
```

**Three of the four tests passed against the control.** They call `expand_template` directly,
so the engine was reachable from a test and from nowhere else — the `ToolHooks`/`TurnHooks`
defect reproduced in new code, on the very slice whose whole purpose was to avoid it.

The fix is one test that types the command and submits through `type_and_enter`, because
that is the only path a user takes. The three direct tests are **kept**, not deleted: they
test the engine's contract, which is worth having. But they are not wiring evidence, and the
commit message and the test's own doc comment now say so.

## Bugs the tests caught, all before anything was wired

**1. `${@:2}` and `${@:-default}` both begin `@:`, and I treated form-detection as a question
about branch *ordering*.** Three attempts, each fixing one form by breaking another:

| Attempt | Result |
|---|---|
| defaults first | `strip_default` took the `2` as its fallback; every slice test failed |
| slices first | `${@:-default}` became unreadable |
| digit bail-out | the default returned its own literal text |

The question was never "which branch first" — it is "is this a slice", and a slice is `@` `:` DIGIT. One predicate at the top. The original tests covered each form *in isolation*, which is why reordering looked like a fix each time; the five collision tests now sit together.

**2. `${@:2}` skipped nothing.** After the `'@'` was stripped the remainder is `":2"`, so splitting on `:` put the *empty* string first, which defaulted to 1 and skipped nothing. Caught by a temporary probe printing intermediates, not by reading.

**3. `$$` was never implemented as an escape.** Both dollars were each treated as unrecognised, so `$$HOME` came back verbatim.

**4. Two of my own test expectations were wrong and the implementation was right.** A raw string `r#"..."#` cannot contain its own delimiter, so my "quoted argument" test contained no quotes at all; and `lines().join("\n")` drops a trailing newline. Both corrected to assert what the code actually promises, with the reason kept.

**5. A trailing-newline expectation in the TUI test** — the same fact the contracts test had already established, missed on the second consumer.

## Deliberate supersets of pi

`${1}` and `${@}` are braced forms pi's regex does not match. Sending someone `${1}` to the
model verbatim is worse than accepting a form the reference happens to reject, so both are
supported. Recorded in `audit/02-port-fidelity.md`.

## What was not verified

- ~~No template was exercised in a real terminal.~~ **CLOSED after the m184 gate**, in
  `debcef2`'s follow-up: a `.epsilon/prompts/zzprobe.md` was driven through
  `scripts/console-probe.py` in a real conhost window.
  - `/templates` -> `prompt templates: /zzprobe — Probe template for live verification (takes <topic>)`
  - `/zzprobe auth-tokens` -> `❮ ❯ ZZPROBE_MARKER reviewed auth-tokens for correctness.`

  The argument was substituted from real keystrokes, and the marker is a string that exists
  only in the template body — so the screen proves expansion, not typing. The probe
  directory was removed afterwards.
- **First probe run was inconclusive and the cause was mine**: the probe typed `/templates`
  without `--enter`, so nothing was submitted and the notice never appeared. The absence of
  output was a measurement mistake, not a product defect.
- **The installed release binary was not rebuilt.** `C:/Users/Hp/.cargo/bin/epsilon.exe` is
  the 06:22 build, which has neither the turn hooks nor templates.
- **The template body is not shown to the user before it is sent.** Expansion is silent, so
  a template that produces a surprising prompt is invisible until the model responds to it.
  Worth a preview, not built.
- **No `/templates` command lists what is installed.** `/reload` reports the count only
  during a reload, and nothing lists names, descriptions, or argument hints. A user who
  forgot a template's name has no way to recall it.
