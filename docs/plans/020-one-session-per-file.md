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

`build_runtime` stops passing the fixed `tui.jsonl` and asks for a new session path. `--resume`
and `-c` keep their meanings (`-c` = newest for this cwd, which starts working correctly once step 1
lands). **Evidence:** two launches in a temp workspace produce two files whose headers carry two
different ids; `-c` after the second restores the second conversation only, asserted on
`history()` (one user message, not three).

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
