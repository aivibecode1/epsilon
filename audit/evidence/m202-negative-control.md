# M202 — the negative control, and why this slice needed one

The slice's central claim is: **a session file written before `Usage::cache_reported` existed
opens again.** That claim has a clean pre-fix RED, so a control was run rather than argued.

## The control

`#[serde(default)]` was removed from `crates/epsilon-contracts/src/message.rs` (leaving the doc
comment intact), and the new test run against that tree:

```
$ cargo test -p epsilon-session --test session_store a_session_written_before_a_field_existed_still_opens
test a_session_written_before_a_field_existed_still_opens ... FAILED
thread '...' panicked at crates/epsilon-session/tests/session_store.rs:2091:10:
a session written before `cache_reported` existed must still open:
  Corrupt {
    line: 2,
    diagnosis: MalformedEntry {
      entry_type: "message",
      detail: Some("missing field `cache_reported`"),
    },
  }
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 70 filtered out
```

It fails **for the reason the test names** — the store refuses the whole file, which is what
`epsilon` does at startup — and it also demonstrates the second half of the slice: the refusal
now says ``missing field `cache_reported` `` where the version before this change said only
"its body cannot be read".

Restored by editing the file back, then verified byte-identical:

```
sha256 7E4DF3DDAB4969703206C902D5429B553C69FDD40A8E3B6E65B72547277BB6FF
```

(Same hash before the control and after the restore. The file was rewritten by the editor rather
than copied, so cargo recompiled it — a `Copy-Item` restore preserves the backup's mtime and
cargo will happily run the *control* binary, which has cost this repository two rounds.)

## The end-to-end reproduction, before and after

Both are real conhost windows at 60×20, read back by `scripts/console-probe.py`:

| | Binary | Screen |
|---|---|---|
| before | the installed M201 `epsilon.exe`, in this repository | the refusal: `open session store: session line #3: … its body cannot be read …`, exit 1 |
| after | the M202 debug build, same directory, same session file | `idle     ctx 4%*/164k · ↑0 ↓0 · R0 W0 CH n/a · DeepSeek V3.1` and the welcome panel |

Full probes: [`console-probe-m202-before-refusal.json`](console-probe-m202-before-refusal.json)
and [`console-probe-m202-after.json`](console-probe-m202-after.json).

**Why no test found this.** Every fixture in `crates/epsilon-session/tests/` was built with the
current shape — `Usage` literals and `append()` calls — so all 69 tests agreed with the bug. The
only artifact carrying the old shape was a session file written by an earlier build, sitting in
this repository's own `.epsilon/sessions/`, and the only way to reach it was to run the shipped
binary. The regression test now carries that artifact's exact bytes.

## A thing that looked like a defect and is not

The "after" frame shows `ctx 4%*/164k` on a session whose transcript is empty, which reads as
hidden history: the file holds 66 messages. It is not. `agent.restore_history(msgs)` is called
**only** on the `--resume`/`-c` path (`main.rs:1924`), so a plain launch starts with an empty
in-memory history and the model does not see the older turns. The 4% is the baseline M194's C2
fix made visible — the system prompt and tool schemas counted on the no-anchor estimate — which
is exactly what the gauge is for. Verified by reading the call site, not inferred from the
gauge.
