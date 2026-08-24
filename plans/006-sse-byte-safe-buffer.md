# Plan 006: SSE byte-safe line buffer (fix multi-byte corruption at TCP boundaries)

> **Drift check**: `git diff --stat 64e927f..HEAD -- crates/epsilon-llm/src/providers/mod.rs`

## Status

- **Priority**: P2 · **Effort**: S · **Risk**: LOW
- **Depends on**: plan 005 (fixtures pin the fix) · **Category**: bug
- **Planned at**: commit `64e927f`, 2026-08-24

## Why this matters

CORRECTNESS-12: `providers/mod.rs:331` does
`self.line_buf.push_str(&String::from_utf8_lossy(chunk))` per network chunk. A
multi-byte char split across TCP chunks becomes U+FFFD garbage mid-stream — corrupting
assistant text and, worse, `input_json_delta` fragments so assembled tool arguments
fail to parse. Classic streaming pitfall; certain to bite on long CJK/emoji outputs.

## Current state

- `crates/epsilon-llm/src/providers/mod.rs:331-332` — line_buf is a `String`;
  splitting only on `\n`.

## Scope

**In scope**: `HttpEventStream::chunk`/ingest buffering. **Out of scope**: decoder
event semantics, provider mappings.

## Steps

1. Change `line_buf` to `Vec<u8>`; append raw chunk bytes; extract complete
   `\n`-terminated lines as byte slices; run `String::from_utf8_lossy` ONLY on
   complete lines (a lossy replacement inside one line is provider-side damage, not
   ours). Handle CRLF before decode.
2. Fixture test: "héllo 🌍" JSON payload split byte-by-byte across 20 chunks decodes
   identically to single-chunk delivery (add to plan 005 harness if present, else
   inline in mod.rs tests).

## Done criteria

- [ ] Split-multi-byte fixture passes; full gates green.

## STOP conditions

- line_buf is shared with code paths assuming String beyond ingest (report sites).
