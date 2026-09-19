# M38 — pi-parity P1 batch 2: grep gains `literal`/`glob`/`context`/`limit`

Sixth port batch against the 2026-09-03 parity audit (tools finding T5).
Grep only spoke `{pattern, path, ignore_case}` with a hardcoded 200-match cap;
pi's grep speaks `{pattern, path, glob, ignoreCase, literal, context, limit}`
with actionable notices. No ripgrep backend — the Rust walker stays (no new
dependency, confinement-safe); the wire contract is what gets ported.

## What landed (`crates/epsilon-tools/src/search.rs`)

1. **`literal`** — pattern escaped via `regex::escape` before compile
   (pi `--fixed-strings`). Proven by divergence: `b.ta` matches `beta` as
   regex, matches nothing as a literal.
2. **`glob`** — file filter reusing the glob tool's own dialect compiler
   (`glob_to_regex`, bare names match at any depth), compiled once per call;
   bad globs report `bad glob` instead of silently matching nothing.
3. **`context`** — N lines each side, rendered pi-style (`path:line:` for
   matches, `path-line- ` for context rows), excluded from the match cap,
   CRLF-normalized via the existing `.lines()` path like pi's re-read.
4. **`limit`** — default 100 (pi `DEFAULT_LIMIT`, was a hardcoded 200), `max(1)`
   so `limit: 0` means 1 instead of an instant empty result. Reaching it emits
   pi's actionable notice (`N matches limit reached. Use limit=2N for more…`)
   plus `match_limit_reached` in `details`.
5. **Line truncation** — `truncate_grep_line` ports pi's `truncateLine` (500
   chars + `... [truncated]`, char-boundary cut so multi-byte text never
   splits); the notice points at the read tool, and overlong lines set
   `lines_truncated` in `details`.

## Key decisions

- Cap counts MATCHES, not rows: context rows ride free, matching pi (its
  `matchCount` increments only on `event.type === "match"`). A `limit: 2`
  query with context returns 2 matches plus their surroundings.
- Output keeps the existing `[N files scanned]` tail and the secret-class
  skip row — both are Epsilon additions pi lacks; the new notices join them
  as a `[notice. notice]` paragraph above the tail, pi's shape.
- `details` now carries `truncated`/`lines_truncated`/`match_limit_reached`
  so frontends can render badges instead of parsing text.

## Gotchas

- First test draft asserted `{}` is invalid regex — it isn't (literal braces
  in most engines, including Rust's). Switched the literal-proof to `.` vs
  escaped `.`, which genuinely diverges.
- A bad patch-tool edit briefly rewrote `seeded_workspace` with a wrong
  anchor; the file looked reverted and the test target vanished from git
  status. Recovered by re-reading and re-applying against the true text —
  when a patch lands somewhere unexpected, `git diff` the file before
  assuming the tool is broken.

## Verification

- Extended `grep_glob_ls_find_seeded_content`: literal divergence, unbalanced-
  paren literal safety, glob restriction, limit notice, context markers,
  truncation suffix + guidance — green.
- Full `cargo test -p epsilon-tools`: 32 lib + 10 integration, green.
- `cargo fmt --check`: clean. `cargo clippy -p epsilon-tools --all-targets`:
  zero warnings.
