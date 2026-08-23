# M4 — epsilon-session (JSONL tree store, shadow views, leases, crash repair)

Date: 2026-08-23 · Status: complete

## What was built

| Module | Contents |
|---|---|
| `store.rs` | `SessionStore`: open/append/read, offset index (`Vec<EntryLocation>`), branch walk via parent chain, tree navigation (`navigate_to`), **model view** with buffered-window compaction shadowing |
| `lease.rs` | Cross-process turn leases: atomic `create_new` lock file + PID record + mtime-based stale steal (30s), released on Drop |
| `repair.rs` | Crash recovery: dangling tool calls get synthetic error results; failure-stopped turns get a `TurnEnd{Aborted}` marker. Idempotent by construction |
| `tests/session_store.rs` | 6 integration tests: roundtrip, fork/navigate, model-view shadowing, truncated-tail recovery, dangling-tool repair, lease exclusivity |

Workspace totals: 34 tests.

## Key decisions made while coding

1. **Model-view shadowing needs a buffer, not a cursor** (test-caught design bug): eager
   emission of messages followed by "shadowed_until" bookkeeping is wrong because a compaction
   entry shadows everything *before* it — you only know retroactively. Fix: buffer messages in
   a pending window; on compaction, drop the window and emit summary+tail instead.
2. **Crash recovery is the expected path, not an error**: `SessionStore::open` now auto-detects
   and removes a crash-truncated tail line. The first design made callers handle
   `CorruptLine(1)` on every post-crash open — an error tax on the common case.
3. **Zero new dependencies for locking**: fs4/flock would be more robust, but the create-new +
   stale-steal protocol is ~60 lines, portable, and adequate because concurrent writers are
   already user error we surface loudly. Revisit if multi-writer UX becomes real.

## The Windows debugging story (worth reading twice)

The crash-recovery test failed with `PermissionDenied` from deep inside file handling. Three
attempts failed before instrumentation settled it:

1. **Guess 1**: append-mode handles can't `set_len` on Windows → TRUE (FILE_APPEND_DATA ≠
   FILE_WRITE_DATA) but reopening write-only wasn't enough...
2. **Guess 2**: reopened with `.write(true)` only → set_len worked, but the NEXT `reindex`
   then failed reading: a write-only handle can't `read_line`. Fixed one bug, revealed its
   sibling.
3. **Fix**: reopen with `read(true).write(true)` (no append) for truncation and adopt that
   handle as the store's reader/writer going forward. All green.

Lessons encoded into the code as doc-comments:
- Windows file rights are per-access-bit, not per-file: an append handle is not a weaker
  writer, it's a different capability class.
- When two syscalls fight over one flag, open a purpose-built handle instead of contorting
  the original.
- Debug prints at the syscall boundary beat three rounds of theory.

## Gotchas hit

- PowerShell string surgery kept missing CRLF vs LF variants — repeated small edits through
  proper tools would have been faster than clever bulk replaces (twice).
- clippy flagged `.write(true)` as redundant next to `.append(true)` — correct for `open()`,
  but the same lint would have been wrong inside `truncate_to_index`, where the combination
  is precisely the point. Context beats lints; both sites now carry comments.

## Verification

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # 34 passed (15 contracts, 12 llm, 7 session)
```

## Next

M5 fills `epsilon-core`: pi's two-level loop verbatim + bounded parallel tool pool +
doom-loop brake, integration-tested against the faux provider, plus the epsilon-cli demo bin.
