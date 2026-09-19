# M57 — Session discovery and the `/sessions` browser

Sessions existed on disk long before this milestone: `.epsilon/sessions/*.jsonl`
holds one file per session, plus every `/fork` and `/duplicate` branch and the
other frontends' files. Two things were missing — one mechanical, one human:

- **Discovery lived in the wrong brick.** "Is this file a session, and which is
  newest for this cwd" was a CLI-internal helper (`runtime.rs`), so the TUI
  could not ask the same question without a copy that would drift.
- **There was no way to see them.** `/resume <path>` demanded a path a human
  already knew; `/session` shows only the *current* file. Finding a fork or
  yesterday's session meant leaving the TUI and reading a directory listing.

## What landed

**`epsilon-session::discover`** — the brick that owns session files now owns
their discovery:

- `read_session_header(path)` (moved from the CLI verbatim: first non-blank line
  within 16 lines, `None` for foreign/unreadable files),
- `summarize(path)` → `SessionSummary { path, id, cwd, modified_ms, bytes, name,
  first_message, parent_session }`,
- `list_sessions(dir)` → newest-first summaries,
- `latest_session(dir, cwd)` → pi `findMostRecentSession` semantics (cwd filter
  applies only when the session carries a cwd; headerless v1 files stay
  eligible), which the CLI's `latest_session_file_in` now delegates to.

Everything is **bounded**: the first message comes from the first 64 lines, the
display name from the last 8 KB (it is written whenever `/name` runs, anywhere
in the file), and nothing else is parsed. Listing a directory of 40 MB
transcripts costs about the same as listing an empty one — a browser that
stalls on its own history is not a browser.

**The TUI browser** — `/sessions` (and bare `/resume`, which used to print a
usage line) opens an overlay: newest first, the attached session marked `●`,
each row `title · age · size · file`. Typing filters (fuzzy over title, file
name, and the session's own cwd — the same `fuzzy_match` the pickers use),
Up/Down move, Enter resumes, Esc closes. The filter *is* the normal input
buffer, so typing and backspace reuse the editor's grapheme-safe paths.

**One switch mechanism.** `/resume <path>`, `/import`, and the browser's Enter
now all call `App::switch_session_file`, which does the store swap, transcript
re-hydrate, prompt-history push, and session-name re-hydration. The three
call sites keep their own notice wording; the mechanism cannot diverge.

## Key decisions

- **Discovery belongs to the session brick, not the CLI.** Two frontends ask the
  same question about the same files; the dependency law puts `epsilon-session`
  below both, so it is the only place the answer can live once.
- **Bounded reads, not full parses.** A picker needs a title, an age, and a
  size — not the transcript. The milestone's contract is "cost of listing is
  independent of session length", which the tests pin with a 192-line session.
- **The current session is marked, not filtered out.** Resuming the file you are
  already on is harmless (`live_resume` re-reads it), and hiding rows would make
  the list disagree with the directory.
- **`/resume` keeps its path form.** Scripts and muscle memory keep working; the
  browser is what a bare `/resume` now does instead of printing usage.

## Gotchas

- The tail scan sees partial lines when a name entry is longer than the read
  window's start — parse failures there are expected and skipped, not errors.
  The name is still found because `/name` writes a single short line.
- `same_session_file` canonicalizes before comparing: the app is handed a path
  the CLI composed and the listing produces one from `read_dir`, and the two
  differ on Windows in separator and prefix spelling.
- A missing session directory and an empty one are different answers ("no
  directory yet" vs "nothing to resume"), and both are tested — a silent
  no-op browser would look like a broken command.

## Verification

- `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets`
  zero diagnostics.
- `cargo test --workspace`: **431 passed, 0 failed** (422 before: +5 session,
  +2 picker, +2 TUI). `--features real` lane: 125 passed, 0 failed.
- New coverage: five session-crate tests (newest-first + foreign-file skipping,
  title fallback chain, cwd filter, missing directory, bounded head/tail reads),
  two picker tests (rows/filter/current marking, age + size labels), two TUI
  tests (browse → filter → Enter switches the attached store; the two "nothing
  to browse" answers).
- The CLI's `latest_session_picks_newest_jsonl_only` test still passes over the
  moved implementation — the behavior contract did not change.
- `cargo run -q -p epsilon-cli -- --demo` completes.
