# M99 — `.gitignore` is honoured instead of merely promised

## What

`find`'s description has said "Respects .gitignore" for as long as the tool has existed. Nothing
read a `.gitignore`. `grep` said less and did the same. The audit's D-9 found two errors pointing
in opposite directions: a walk that lists every file in a build output directory, and — because
the walker skipped *every* dot-directory — `.github/`, `.vscode/` and `.agents/` unsearchable
even when they are exactly the content being asked about.

A coding agent that cannot trust a tool's description wastes a turn discovering the truth, and a
walk that lists a build output wastes far more context than the turn it cost.

`epsilon-core/src/ignore.rs` is the matcher: `IgnoreSet::load(root, max_files)` reads the
`.gitignore` files from `root` down, `IgnoreSet::is_ignored(relative, is_dir)` answers per path,
and `ALWAYS_SKIP` (`target`, `node_modules`, `dist`, `.git`) plus `skip_dir` are the directories
no ignore file can re-admit. `util::walk_files` loads an `IgnoreSet` and uses it;
`walk_files_with` takes one the caller already has, so `grep`'s several walks pay for one read.
Both descriptions now state what is implemented and where the edges are.

Nineteen tests in the module, plus `walk_files_actually_honours_gitignore` on the walker and a
new assertion in the existing walk test that a hidden directory is skipped by default.

## Key decisions

- **The matcher lives in `epsilon-core`, not in the tools.** Two callers need the same answer —
  the tool walkers (`find.rs`, `search.rs` both call `util::walk_files`) and the TUI's `@`-file
  completion — and two copies of a rule this fiddly drift. The symptom of drift is specific and
  bad: a file `grep` refuses to search appearing in the completion list, or the reverse. (The
  TUI still walks with its own local helper and its own hardcoded list — see Limits. The reason
  for the location is that `epsilon-tui` depending on `epsilon-tools` would be a cycle, so the
  shared module sits in the crate both can reach.)
- **Ancestor resolution, not whole-path matching.** Git resolves per path component and never
  descends into an ignored directory, so with
  ```text
  dir/
  !dir/
  ```
  `dir` is ignored and then re-included, and **everything under `dir` comes back too**.
  Testing only the full path gets that wrong twice: the rule `dir/` looks for a path that
  *equals* `dir`, so `dir/keep.txt` is missed, and the negation never reaches its children.
  `is_ignored` therefore resolves each ancestor in order and stops at the first ignored one.
  The walker was wrong in exactly this way until the test caught it.
- **The always-skip list is not a preference.** A build directory or a package manager's cache
  can hold hundreds of thousands of files, and walking one turns a fast tool into a stalled
  one; the entries there are skipped whatever any ignore file says.
- **Hidden directories are skipped, hidden files are not.** A `.venv` is not workspace content;
  `.env` is content the read tools must see. A `.gitignore` negation re-includes a hidden
  directory in principle — but see the known gap below, because the walker does not act on it.
- **Ignore-file reading is bounded** (`WALK_IGNORE_FILES = 256`). A monorepo can hold thousands
  of these, and a tool that spends its whole budget reading ignore files has none left for the
  work it was asked to do. When the cap is hit the rules found so far still apply, so the walk
  is narrower than ideal and never wrong about what it searched — a different failure from
  walking something it should have skipped.
- **The unsupported syntax is named in the module docs rather than discovered later**: character
  classes (`[a-z]`), escaped metacharacters (`\*`) and `core.excludesFile`. Each is rare in the
  files this walks, and a documented gap is a different thing from a silent wrong answer.
- **The flaky guard test is fixed rather than quarantined.** `read_log::tests::edits_are_guarded_
  too` failed roughly one run in three: it rewrote a file with a *same-length* string, so on
  NTFS two writes inside one clock tick left both the recorded length and the timestamp
  identical and the guard — correctly — saw no change. It now writes a different length, which
  is a change no clock resolution can hide. The property under test (an edit needs the same
  guard a write does) never depended on how the change was spelled.

## Gotchas

- **The last matching rule wins, and that is the piece that decides most real cases** — including
  across nested files. `a_nested_file_scopes_its_own_rules` and `the_last_matching_rule_wins`
  pin both halves.
- **`*` must not cross a separator and `**` must.** Getting this backwards makes `*.log` match
  `src/deep/file.log` or makes `**` useless; both directions have their own test.
- **A directory pattern must not match a prefix of a name.** `dir/` does not ignore `directory`;
  `an_ignored_parent_decides_for_its_children` and `a_directory_pattern_matches_that_directory_
  and_not_a_prefix_of_a_name` pin the two halves of that.
- **A glob can backtrack forever if it is written naively.** `a_glob_that_could_backtrack_forever_
  terminates` is in the suite because the matcher is hand-written, not because a case was
  observed.
- **`load` must not descend into an always-skipped directory while looking for ignore files
  either** — `loading_never_descends_into_always_skipped_directories` — or the walk pays the
  cost it exists to avoid before the walk even starts.
- **The old skip list was compiled into the walker as `SKIP_DIRS`.** It is still exported
  (`epsilon-tools::util::SKIP_DIRS` = `.git`, `target`, `node_modules`, `dist`, `.epsilon`) while
  the walker's own decisions now come from `epsilon_core::ignore::ALWAYS_SKIP`/`skip_dir`
  (`.git`, `target`, `node_modules`, `dist`). The two lists are no longer the same list: the new
  one has no `.epsilon`. It stays un-walked by the hidden-directory rule — unless a `.gitignore`
  negation re-includes it, which is the one way the removed entry could become visible again. A
  reader who assumes the two constants agree will be wrong.
- **The walker does not descend into a hidden directory even when a negation re-includes it.**
  The matcher resolves that case correctly; the walker's hidden-directory rule runs first. The
  test says so in a comment rather than asserting behaviour that is not there, and `audit/11`
  carries it under "Known and deliberately not claimed".

## Verification

No battery log for this slice is preserved, and no gate log exists for M99 (`audit/evidence/`
holds `gate-m93b`, `gate-m94c`, `gate-m95b` and `m101-gates.log`; none of them is these bytes).
The evidence is the commit message's own measurements, reproduced here as the commit states
them:

- `cargo test -p epsilon-core` — **19 ignore tests + 112 filtered green**.
- `cargo test -p epsilon-tools` — **92 passed, 0 failed**.
- `cargo clippy` on both crates with `-D warnings` — exit 0.
- `cargo fmt --all -- --check` — exit 0.
- The flaky test's fix is its own measurement: **six consecutive runs green**, against a
  pre-fix failure rate of roughly one run in three.

## Limits

- **The TUI's `@`-file completion does not use the matcher.** `app.rs::at_file_rows` keeps a
  local walker and a hardcoded `[".git", "target", "node_modules", "dist", ".epsilon"]` list
  (with a comment explaining that a TUI→tools dependency would be a cycle). So the completion
  list honours neither `.gitignore` nor the shared list, and the exact drift the module was
  placed in `epsilon-core` to prevent is still present on that one path. Moving the walker into
  `epsilon-core` is the obvious follow-up and is not done here.
- **A negation that re-includes a hidden directory is matcher-correct and walker-wrong.** Git
  would descend into it; our walker skips it before asking. Named in the module docs, in the
  walker's test as a comment, and in `audit/11`.
- **Character classes, escaped metacharacters and `core.excludesFile` are not implemented.** A
  file relying on `[a-z]*.rs` or on a global excludes file is walked as if the pattern were not
  there — the pattern is ignored, not the file. That is the safe direction but it is still a
  difference from git.
- **`.gitignore` is the only ignore file read.** `.ignore`, `.git/info/exclude` and git's
  `core.excludesFile` are not consulted, so a repository that relies on them gets a wider walk
  than git would.
- **The ignore set is built per walk.** `walk_files` re-reads the `.gitignore` files every call;
  `walk_files_with` exists for callers doing several walks, and `grep` uses it, but a caller that
  simply calls `walk_files` twice pays twice.
- **Anchoring is relative to the workspace root the walk was given**, which is the tool's search
  directory, not necessarily the repository root. A search rooted in a subdirectory therefore
  applies only the ignore files at or below it — matching git's behaviour for a path-limited
  invocation, but worth knowing when a rule at the repository root seems to have no effect.
- **Windows-only verification.** The matcher is path-shape logic over `/`-separated relative
  paths and the tests ran here; the Linux CI job has not run on these bytes.
- **The old `.tmp-console/bashtest` probe and the M98 measurement are unrelated** — noted only
  because both milestones landed within an hour of each other and cite different artifacts.
