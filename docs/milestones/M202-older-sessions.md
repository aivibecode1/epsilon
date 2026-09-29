# M202 — an older session is old, not damaged

**Status:** landed. GATE PASS `m202` — see [Verification](#verification) for the measured
counts. Reinstall: **required** — this is the difference between `epsilon` starting and not
starting in a workspace that has an older session, which is the installed binary's first act.

## What

`epsilon` refused to start in **this repository's own working directory**, with:

```text
open session store: session line #3: entry type `message` is one this build knows, but its
body cannot be read — the entry is damaged rather than foreign. …
```

The entry was not damaged. `Usage::cache_reported` was added in M194 **without a serde
default**, and `Usage` sits inside every assistant message — so every session file written
before that milestone failed to decode. Because the store refuses a known entry kind whose body
it cannot read (M108's rule, and the right rule), the refusal took the whole file down, and the
TUI with it. That is the class M108's `Limits` recorded as owed, one level down: the *kind* was
known, the *field* was new, and nothing distinguished the two.

Three changes:

| | Before | After |
|---|---|---|
| **The field** | absent → the file is refused | `#[serde(default)]` on `cache_reported`, whose own semantics make `false` the honest value for "this writer did not report it" — the same value an unannotated decoder produces, which is what the header renders as `CH n/a` rather than `0%` (K1) |
| **The diagnosis** | "its body cannot be read", full stop | `missing field \`cache_reported\`` — the decoder's own sentence, quoted |
| **The message** | `open session store: session line #3: …` | `open session store <path>: …` — a line number inside a file the reader was never told the name of is not a location |

## How it was found, and why nothing in the tree could have

By driving the **installed binary** through `scripts/console-probe.py` in a real conhost
window — the only check in this repository that runs what a user runs.

Every fixture in `crates/epsilon-session/tests/` is built with the current shape: `Usage`
literals and `append()` calls. So all 69 session tests agreed with the bug, and the file that
carried the old shape was an artifact written by an earlier build, sitting in this repository's
own `.epsilon/sessions/`. The regression test now carries that artifact's exact bytes — 509 of
them, copied verbatim — and its doc comment says why a hand-built fixture could not do.

## The diagnosis needed a second decode

The first version of the improvement passed the failing decoder's error out of `SessionLine::parse`
and printed it. It read:

```text
data did not match any variant of untagged enum SessionLine
```

`SessionLine` is **untagged** — it has to be, because a session file is a mix of headers, current
entries and legacy shapes — and serde reports the outermost failure for an untagged enum, throwing
away the field-level one. The fix is that `diagnose_line` re-decodes the same `serde_json::Value`
through `SessionEntry` directly, which is the tagged parser, and quotes *that* error. Two decodes,
one of them only on the failure path, which is why it can afford to.

The general rule this is an instance of: **an error message is part of the interface, and an
untagged enum cannot produce a good one.** The `MalformedEntry` variant carries `detail` now, and
the negative control shows the sentence it buys.

## Limits — what this does not prove

- **The fix covers one field.** `cache_reported` is the field that broke; the *rule* — a field
  added to a durable session type must be readable when it is absent — is on that field's doc
  comment and in an ADR-006 amendment, but nothing enforces it. A future field added without a
  default will break old sessions the same way, and the only thing standing in the way is the
  next author reading the amendment. A test cannot enumerate fields it does not know about; what
  would catch it is a committed *old* session fixture replayed by the suite, which is what the
  new test is for the one shape we have.
- **Only the one old artifact on this machine was exercised.** A session written by a build
  between M1 and M194 with a different drift would not be covered; nothing on disk has one.
- **The welcome screen still does not mention that a session exists.** A plain launch in a
  workspace with a long `tui.jsonl` shows an empty transcript and the `-c` flag is the only way
  to get it back. That is the documented design (`--resume` seeds history; a plain launch does
  not), and the header's `ctx 4%` is the *baseline* — the system prompt and tool schemas — not
  hidden history. Verified at `main.rs:1924`, where `restore_history` is called only on the
  resume path. Recorded because it looks like a defect from the screen and is not one.
- **`reset_leaf` still has no production caller.** `SessionStore::reset_leaf` exists, is
  documented as the way to make the next append a new root, and is called only from tests —
  the repository's own "faithful port, no production caller" class, now at its seventh
  occurrence. Whether a plain TUI launch *should* start a new root in an existing file is a
  design decision about what `tui.jsonl` is (one rolling session per frontend, or one session
  per launch), and this slice deliberately did not take it.

## Verification

GATE PASS, 2026-09-29 — battery **`m202`**, run against this commit's contents with the
documentation already final. Seven gates, exit 0 each: `check-links` 0 s (437 relative links,
0 broken), `check-lint-tables` 0 s, `fmt --check` 2 s, `clippy --workspace --all-targets` 38 s,
`test --workspace` 721 s, `demo` 53 s, `real-check` 28 s.
**1,579 tests across 63 binaries, 0 failed** — summed from the 63 `test result: ok.` lines in
the log. M201 measured 1,577 across the same 63, so this slice adds the two tests above
(`a_session_written_before_a_field_existed_still_opens` and
`a_missing_field_this_build_always_had_is_refused_by_name`).

Evidence: [`audit/evidence/gate-m202.log`](../../audit/evidence/gate-m202.log) (the full
battery); [`m202-negative-control.md`](../../audit/evidence/m202-negative-control.md) (the
control, its output, and the sha256-verified restore);
[`console-probe-m202-before-refusal.json`](../../audit/evidence/console-probe-m202-before-refusal.json),
[`console-probe-m202-after.json`](../../audit/evidence/console-probe-m202-after.json) and
[`console-probe-m202-installed.json`](../../audit/evidence/console-probe-m202-installed.json)
(real-console windows: the refusal and the debug build at 60×20, the reinstalled binary at 80×24).

**Reinstall:** required. The whole point of the slice is what the *installed* binary does on
startup, so `cargo install --path crates/epsilon-cli --features real --force` was run after the
battery passed and the installed binary was driven through the console probe.

**What the battery could not have caught.** The regression test carries a real 509-byte artifact,
so it covers the shape that broke — but the battery cannot tell whether *another* field has
drifted the same way, because it has no old session to replay beyond that one. That is in
`Limits`, not hidden.
