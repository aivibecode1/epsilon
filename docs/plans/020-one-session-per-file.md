# 020 — one session file per session (A16)

**Ledger row:** A16. **Status:** decided; execution not started. **Decision:** the fixed
`tui.jsonl` is a divergence from the reference and is wrong; a launch gets its own file.

## The question, and why it is not a detail

`SessionStore::reset_leaf` exists ("the next append is a new root"), is careful, is documented, and
**has no production caller** — the sixth instance of this repository's signature class. Its
existence is the symptom: the code contains two answers to *what is a session file?*, and ships the
one that makes `-c` wrong.

| | Today | What it means for a user |
|---|---|---|
| the file | `.epsilon/sessions/tui.jsonl`, one per frontend per workspace, opened by every launch | three launches append three conversations to **one lane** |
| `-c` / `--resume` | restores the file's active branch | the model sees **all three conversations** as its context, with no boundary between them |
| the header | written once, so it carries **one** `id` and one `cwd` | the id names the first conversation, and every later one is a stowaway in it |
| `/fork`, `export`, `discover` | all assume file ↔ conversation | a fork of a three-conversation file branches all three |
| growth | unbounded | the file is folded from the start on every `goal()` read, every label read, every read-all |

## The reference answers it, and this harness diverged

pi names one file per session, in a directory per working copy
(`references/pi-0.85.1/packages/agent/src/harness/session/jsonl/repo.ts:42-45`):

```ts
function sessionDirectoryName(cwd: string): string {
	return `--${cwd.replace(/^[/\\]/, "").replace(/[/\\:]/g, "-")}--`;
}

function sessionFileName(createdAt: number, id: string): string {
	const timestamp = new Date(createdAt).toISOString().replace(/[:.]/g, "-");
	return `${timestamp}_${encodeURIComponent(id)}.jsonl`;
}
```

and resolves that path on **create** (`repo.ts:77`, `repo.ts:171`, `resolveNewSessionPath` at
`:273`). So in pi, "the newest session in this workspace" is a meaningful phrase — which is exactly
what `--continue` means here too (`latest_session_file_in(dir, cwd)`). A harness that writes one
fixed file per frontend makes its own `-c` semantics empty.

**Decision: match the reference.** A launch creates a session file; `-c` reopens the newest for
this workspace; `--resume <file>` opens a named one; forks and clones keep creating their own, as
they already do.

## What this fixes, and what it costs

**Fixes:** `-c` reopens *one* conversation. `session_id`, `export` and the sessions browser stop
lying. The transcript matches the file, so M205's "this session already holds N entries" notice
stops being the normal case — it becomes what it was written for, a *resumed* file whose history
you chose not to load. And the file stops growing without bound, which removes a fold cost from
every `goal()` read.

**Costs, stated rather than discovered later:**

- **A file per launch.** A workspace used daily accumulates hundreds of small files. `discover`
  already lists many, and pi has the same shape, so this is the accepted price of the model — but
  nothing prunes them today, and *that* becomes the new open question rather than staying hidden
  behind one big file.
- **The per-cwd directory is a second change.** pi writes into `sessions/--<cwd>--/`; this harness
  writes into `sessions/`. Adopting the directory naming is what makes `-c` correct *across*
  workspaces rather than only within one, so it belongs in the same slice — but it migrates every
  existing path, and the sessions browser's grouping depends on it.
- **Existing files are not rewritten.** `tui.jsonl` and friends keep working when named
  explicitly; they simply stop being created. No migration is attempted, and the honest wording in
  the docs is "old files stay readable", not "history is migrated".

## Execution, in order

### Step 1 — the naming rule gets an owner, with the reference cited

`epsilon-session` gains the one function that decides where a session file goes
(`session_file_for(dir, cwd, created_at, id)`), and `discover.rs`'s naming search reuses it instead
of keeping its own pattern. **Evidence:** a test that the name for a fixed (cwd, created_at, id) is
the string pi's two functions produce for the same inputs — computed from the TypeScript above, not
from this implementation.

### Step 2 — a launch creates a file

**Split in two, because the second half is a migration and the first is not.** Doing them separately
means the user-visible win lands without waiting for a directory move.

**2a — a new file name in the existing directory (no migration).** `build_runtime` stops passing the
fixed `tui.jsonl` (and `cli.jsonl`/`serve.jsonl`/`rpc.jsonl`) and passes
`session_file_name(now_ms, id)` instead, still inside `.epsilon/sessions/`. Read the blast radius
before starting: `RunParams.session_file` is `&'a str` today, so it becomes owned (5 literal call
sites at `main.rs:1508/2114/2254/2304/2579`), and everything that *reads* the directory already
works unchanged — `latest_session_file_in` picks the newest `*.jsonl`, `list_sessions` scans the
same dir, and `doctor`'s seeded `tui.jsonl` stays a readable old file rather than a special case.
**The chicken-and-egg is already solved — checked, not assumed.** The name needs the session id, and
the id is minted by the store when the header is first written, so 2a needs the *caller* to mint it.
That generator exists and is shared: `epsilon_contracts::new_id(prefix)` (`ids.rs:14`), monotonic and
sortable (`{prefix}_{unix_millis:x}_{counter:x}`), dependency-free, and already used across crate
boundaries for message and tool-call ids. So 2a is `let id = new_id("session")` at the call site,
passed to both the name and the store — **no new id rule, no new edge, and nothing to write**.

**2b — the per-cwd directory.** Point the same call at `session_file_for(root, cwd, …)` (M209's
rule) so `.epsilon/sessions/--C--Users-Hp-Epsilon--/` holds this workspace's sessions and `-c`
becomes correct *across* workspaces. This is the half that migrates every existing path, and old
files stay where they are: readable, never created again.

> **2b's hidden half, checked 2026-09-30 — the readers do not follow the writers.** `list_sessions`
> is a **single** `std::fs::read_dir` (`discover.rs:223`), not a walk, and every reader is given the
> sessions *root*:
>
> | Reader | Site |
> |---|---|
> | `-c` / `--resume` resolution | `main.rs:1823` — `latest_session_file(&workspace.join(".epsilon").join("sessions"))` |
> | RPC session discovery | `rpc_mode.rs:3113` — `list_sessions(&dir)` |
> | `doctor` | `doctor.rs:404` — the same root |
> | the TUI `/sessions` picker | `picker.rs:1154` — `epsilon_session::list_sessions(dir)` |
>
> So moving the *writer* into a subdirectory would leave every one of them scanning a directory that
> now contains only subdirectories: `-c` would find nothing, `/sessions` would list nothing, and
> `doctor` would report an empty workspace — with no error anywhere, which is the shape this
> repository keeps finding. **2b is therefore one change in two halves, and the read half is the
> larger one.**
>
> It also has a design consequence worth taking rather than working around: with one directory per
> working copy, the **`cwd` header filter becomes redundant**. `latest_session_file_in(dir, Some(cwd))`
> exists to answer "which of these files belongs to this workspace" by reading headers; if the
> directory already answers it, readers should resolve the *same* directory the writer uses. Which
> means one function — `session_dir_for(root, cwd)` — called by both sides, rather than the writer
> computing a directory and four readers each remembering to descend. That is A12's rule (one rule,
> one place) applied before the defect rather than after it.
>
> **The migration answer, stated here so it is not discovered late:** readers must also look in the
> *flat* directory, or every existing session becomes invisible on upgrade. Cheapest honest form:
> resolve the per-cwd directory first, then fall back to the flat root for `*.jsonl` files whose
> header `cwd` matches — which is exactly what `latest_session_file_in`'s filter already does today,
> so the fallback is the current behaviour kept as a compatibility path rather than new logic.

**Evidence for both:** two launches in a temp workspace produce two files whose headers carry two
different ids; `-c` after the second restores the second conversation only, asserted on `history()`
(one user message, not three) — and for 2b, that the second launch's file lands in the per-cwd
directory while the old `tui.jsonl` is still openable by name.

### Step 3 — `reset_leaf`'s disposition

With one file per session it has no caller *by design* — and something must still answer `/new`,
which today clears the transcript and keeps the file. Either `/new` becomes "start a new file" (a
store swap rather than a leaf reset) and `reset_leaf` is deleted, or it keeps resetting the leaf for
one-file-many-conversations, which this decision rejects. **Decide by deleting it** unless step 2
finds a caller; an uncalled primitive is how this row started.

### Step 4 — the docs

`docs/user/sessions.md` states the model (one file per launch, where they live, what `-c` opens),
and the M205 notice's paragraph is re-read against the new behaviour — a sentence that was true of
the old shape and is now misleading is exactly what this repository's review discipline looks for.

## Limits

- **No migration.** Old files stay on disk and stay readable; nothing moves them into the new
  directory, so a workspace will hold both shapes indefinitely. That is deliberate — a migration
  that rewrites a user's history is a worse failure than an untidy directory.
- **The pruner is not designed here.** Nothing deletes old session files, and this plan does not
  add anything that does. The sessions browser is the manual answer for now.
- **Step 1's naming test pins format, not round-trip.** It proves the name matches the reference
  for fixed inputs; it does not prove that two launches cannot collide within the same
  millisecond, which the reference handles with the id component and this plan inherits without
  exercising.
