# epsilon-session

## What this crate is

The session store. One append-only `.jsonl` file per conversation; each line is a
`SessionEntry` with a sequence number, a parent link and a timestamp, so the file is a
branch tree. From that tree this crate derives the model-visible view and its compaction
shadowing, and owns the cross-process write lease, crash repair and discovery of past
sessions. It owns storage, not the agent loop, and it draws nothing.

## How it connects

- **Depends on:** `epsilon-contracts` only, plus `serde` and `serde_json`.
- **Depended on by:** `epsilon-core` — which re-exports its session types so the front
  ends need no direct edge — and through core, `epsilon-runtime`, `epsilon-tui`,
  `epsilon-desktop`, `epsilon-cli`.

## Entry points

- `store` — `SessionStore` (`src/store.rs:325`), opened by `open` (`:473`), `open_in`
  (`:479`) or `open_in_with_progress` (`:485`). Everything that touches the file is here:
  `append` (`:1222`), `branch_entries` (`:1373`), `tree` (`:1486`), `navigate_to` (`:1580`),
  `model_view` (`:1873`), `context_settings` (`:1946`), `reindex` (`:857`).
- `lease` — `SessionLease` (`src/lease.rs:198`), one writer per file across the machine;
  `sweep_stale_locks` (`:158`) reaps locks whose holder is provably gone.
- `repair` — `repair_on_open` (`src/repair.rs:92`) and `RepairReport` (`:59`).
- `discover` — `list_sessions` (`src/discover.rs:224`), `latest_session_for_cwd` (`:350`),
  `summarize` (`:190`), `read_session_header` (`:83`).
- `import` — `import_pi_session` (`src/import.rs:231`): copies a current pi session file
  into a new Epsilon file rather than editing the original.
- `search` — `search_session` (`src/search.rs:139`). `time` — the one timestamp and
  session-file-name formatter (`src/time.rs:25`), shared by the wire and by file naming.

## How to run and test it

No binary of its own — the runtime opens a store against a session path. Integration
tests: `tests/session_store.rs`, `tests/pi_import_reconciliation.rs`.

```sh
cargo check -p epsilon-session
cargo test -p epsilon-session
```

## Things worth knowing

- The file is the human transcript. The model sees `model_view()` instead, a derived
  projection in which a compaction entry *replaces* everything before it rather than adding
  to it, so a later compaction supersedes an earlier one's summary (`src/store.rs:1873`).
- Exactly one process writes a file. The holder heartbeats every 2 s and a lock older than
  30 s is stealable, so a crash does not wedge the session (`src/lease.rs:88`).
  Acquisition waits the window out rather than refusing, because the ordinary restart is
  the case that matters (`src/lease.rs:35`).
- `LineDiagnosis` separates "this file is damaged" from "this file was written by a newer
  build" (`src/store.rs:80`). They need different action from the reader, and collapsing
  them into one message is how a good session gets deleted. Migration rewrites in place and
  keeps the previous bytes at the sibling named by `original_backup_path` (`:2024`).
- `repair_on_open` is idempotent: a second pass over a repaired branch does nothing
  (`src/repair.rs:90`).
- Sessions are filed per working directory under the sessions root, and resume also looks
  in the flat root that older launches wrote to (`src/time.rs:120`).