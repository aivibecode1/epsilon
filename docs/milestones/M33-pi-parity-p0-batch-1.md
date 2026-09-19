# M33 — pi-parity P0 correctness batch 1: session corruption, edit/bash wire tolerance

First execution batch against the 2026-09-03 five-scout parity audit
(loop/harness, tools, session/auth/config, TUI, providers/server/extensions).
Scope rule for this milestone: P0 correctness and data-loss items with no new
dependencies and no loop-control-flow changes. Bigger P0s (OAuth, Bedrock/Vertex,
Responses-family wires, abort-during-stream, window-relative compaction) are
sequenced next; intentional forks are recorded in ADR amendments instead of ported.

## What landed

1. **Session corrupt-file semantics (audit S3, P0 data-loss).**
   `SessionStore::reindex` treated *any* unparsable line as a crash-truncated
   tail: it stopped indexing there and `open()` truncated everything after it,
   so one corrupt mid-file line destroyed the whole good suffix on next open.
   Pi's `storage.ts::load` only repairs a bad *last* line (torn tail) and throws
   `invalid_entry` naming the line otherwise. The rewrite collects non-blank
   lines with offsets first, then classifies: bad last line → torn tail
   (`Ok(false)`, existing truncate path, return contract unchanged); any other
   bad line → every well-formed line still indexed, file left byte-identical,
   `Err(StoreError::CorruptLine(physical_line_no))`. `CorruptLine`'s display no
   longer claims a crash tail.
2. **Edit wire tolerance (audit T3, P0 interop).** Pi speaks
   `edits:[{oldText,newText}]` plus legacy `oldText`/`newText`, and tolerates
   `edits` sent as a JSON string (`prepareEditArguments`). Epsilon only spoke
   `old_string`/`new_string`, so a pi-shaped call failed argument parsing
   outright. `EditEntry`/`EditArgs` gained `oldText`/`newText` serde aliases
   and `execute` pre-parses stringified `edits` (parse failure falls through to
   normal schema validation, matching pi's silent `catch {}`).
3. **Bash timeout contract (audit T1, P0 compat half).** A pi-shaped
   `{timeout: 30}` was silently ignored (serde default → 120s), and
   `{timeout: 0}` silently clamped to 1s. `timeout` is now an alias for
   `timeout_secs`; zero is a validation error per pi `resolveTimeoutMs`.
   Deliberately NOT ported: non-zero exit as thrown error (Epsilon returns `Ok`
   with `exit_code` in details — model information, not run control) and
   no-default/unbounded timeout (Epsilon keeps 120s default + 600s cap).
   Both recorded as intentional forks in `docs/design/epsilon-decisions.md` (M33
   amendment), not silent divergence.
4. **Working-tree stabilization.** The tree was red: `state.rs` used
   `StopReason` without importing it, then borrowed a moved `text` (fixed with
   a hoisted `text_empty`); `latex.rs` carried four
   `empty_line_after_doc_comments` warnings (blank lines inside `///` blocks).
   Full-workspace clippy is back to zero errors; warning count re-verified.

## Key decisions

- Mid-file corruption fails `open()` (pi throws `invalid_entry`) rather than
  opening degraded: every consumer (`--continue`, TUI `/resume`, RPC) already
  handles `open` errors, while a degraded-open mode would need a new API on
  every read path. File preservation is the data-loss fix; operability is the
  error naming the exact line.
- `reindex` keeps its `Result<bool>` signature (torn-tail `Ok(false)` path
  unchanged, existing crash test untouched) — the new behavior only adds the
  `Err` case that never occurred before.
- Bash/exit-code and timeout-default forks get an ADR amendment now, while the
  reasoning is fresh, instead of during the Phase-4 docs rewrite.

## Gotchas

- The patch tool runs its own lint with a default (pre-2024) edition, so every
  `&& let` chain in edition-2024 files reports phantom "let chains" errors.
  Real `cargo check`/`clippy` is the arbiter; ignore the tool lint for those.
- The patch tool rewrote `latex.rs` line endings file-wide on one edit; the
  file is untracked so no git damage, and `cargo fmt --check` stays green, but
  prefer confirming `git diff --stat` after edits to mixed-ending files.
- `cargo test` runs started before an edit can race it: a green run that
  overlapped an edit must be re-run (happened once with the tools suite).

## Verification

- `cargo fmt --all -- --check`: clean.
- `cargo test -p epsilon-session --test session_store mid_file`: 2 new tests pass
  (`mid_file_corruption_reports_line_and_preserves_file`,
  `mid_file_corruption_plus_torn_tail_still_reports_line`).
- `cargo test -p epsilon-tools`: full crate green, including
  `edit_accepts_pi_wire_names_and_stringified_edits` and
  `bash_accepts_pi_timeout_name_and_rejects_zero`.
- Pre-existing suites (`crash_truncated_final_line_is_detected_and_truncated`,
  all bash/edit tests) unchanged and green — no behavior regressed on the
  previously-covered paths.
- Flake note: one full-crate run showed
  `bash_runs_powershell_with_exit_codes_and_caps` FAILED while the rest passed.
  The test uses `timeout_secs: 5` and defaults, for which old and new timeout
  code compute identical values, and it passes alone and on full-binary retry
  (14/14). Verdict: parallel powershell-spawn flake under load, not a
  regression — but worth watching if it recurs.
