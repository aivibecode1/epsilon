# Installed binary — M192/M193 wire verification

**Binary:** `C:\Users\Hp\.cargo\bin\epsilon.exe`, 21,486,080 bytes, installed after `3baff78`.
**Method:** real frames into the installed release's stdin, JSON out of its stdout.
Nothing here is an in-process call, because **an in-process call is exactly what hid D29**
for two milestones.

Probe: `probe_m193.py` (scratch; not committed — it is a tool for one investigation, and
`scripts/` is for tools the project keeps).

## Why this document exists

M192B passed 63 targets and 1,458 results. Both `get_entries` and `get_tree` were still
wrong in the same run — `get_entries` was serving epsilon's internal entry shape while
`get_tree` served pi's. Every test had asserted the handler against itself, so raw and
converted agreed by construction. D27, one function over, and the same root cause.

**A wire probe is not a stricter unit test. It is the only kind of check that can see what
a client receives.**

## Pass 1 — two runs, then read the file

```
prompt           ok=True  OBJECT{started}
prompt           ok=False another run is already active
get_entries      ok=True  OBJECT{entries,leafId}
entries: 2
```

The second prompt raced the first. `prompt` returns `{started}` and the run continues in
the background, so a client must wait for the run's end event before sending the next
prompt. Worth knowing before writing a client against this.

## Pass 2 — `fork` at the first entry

```
fork             ok=True  OBJECT{cancelled,text}
     text.id=ent_1a0e78f04c52  cancelled=False
get_entries      ok=True  AFTER FORK: 1 entries (file had 2)
get_tree         ok=True  roots=1 depth=1
FORK FILE: 2026-09-28T10-28-30-472Z_ses_1a0e78f1a880.jsonl  500 bytes
```

All four properties that matter, over a real wire:

| Property | Evidence |
|---|---|
| The reply names what it forked from | `text.id` equals the requested `entryId` |
| The fork is a **shorter** conversation | 2 entries → 1, forking at entry 0 |
| The agent **rebound** | `get_entries` in the *same process* reports the fork's count |
| The tree re-derived | 1 root, depth 1 |

The rebind is only observable in one process, because the rebind is process-local — a
second process would start on the original file. That is why the probe runs all frames
through a single `Popen`.

## Pass 3 — `new_session`, then `switch_session` back

```
new_session      ok=True  OBJECT{cancelled,sessionFile}
     sessionFile=ses_1a0e78f1aa70.jsonl
get_entries      ok=True  0 entries
get_messages     ok=True  ARRAY[0]  <- BARE array, as pi's client unwraps it
switch_session   ok=True  OBJECT{cancelled,sessionFile}
get_entries      ok=True  2 entries
```

The round trip closes: a new session is **empty in both senses** — zero entries in the file
*and* zero messages in the conversation — and switching back restores the original's two
entries. This is what control H was written to protect, now measured from outside.

`get_messages` returning `ARRAY[0]` rather than `{messages: []}` is D27's fix confirmed on
the installed binary, three milestones after it landed.

## Two defects the probe found that 1,458 tests did not

### D29 — `get_entries` served the wrong entry shape

```
get_entries: {"id":"ent_1a0e7861a211",
              "kind":{"custom_type":"session_name","data":{...},"type":"custom"},
              "parent_id":null,"seq":0,"timestamp_ms":1790590720545}

get_tree:    {"id":"ent_1a0e7861a211",
              "parentId":null,"timestamp":"2026-09-28T10:18:40.545Z","type":"custom"}
```

Two ops behind one reference type, two shapes. A client reading `entry.parentId` from
`get_entries` got `undefined`, and `entry.timestamp` was a number where pi sends a string.
Fixed in `6a19af1`; both ops now share `entry_as_pi`, and control I (raw structs restored)
fails both new tests while printing the defect.

### A new session was named differently from a fork

```
2026-09-28T10-28-30-472Z_ses_1a0e78f1a880.jsonl   <- fork, named by the store
ses_1a0e78f1aa70.jsonl                            <- new_session, named by the handler
```

Nothing failed. Two spellings in one directory sort into two groups with no visible reason.
`SessionStore::new_session_path()` now owns the convention (`1b79f8a`), so a second caller
cannot invent a third.

## Three lessons about the probe itself

**A probe can report a false negative about the thing it exists to check.** The first
version compared the post-fork directory against the *pre-run* list — necessarily empty,
because the first prompt creates the session — and printed `NO NEW FILE` directly beneath
the fork file it had just listed. The precondition was checked in the wrong order.

**An undrained stderr pipe blocks the child.** The first inline version read stderr only
after `terminate()`, and a chatty child fills the 64 KB pipe buffer and stops. Written as a
script with a draining thread.

**`--rpc` ignores the directory you launched it from.** `main.rs` hardcodes
`session_file: ".epsilon/sessions/rpc.jsonl"`, resolved against the **process's** cwd. An
early probe therefore wrote into the repository's own session file while appearing to work
in a temp directory. A probe that pollutes the working tree while reporting success is the
same failure as a test that asserts itself — just further from the code.

## What this does not prove

- **No real provider was called.** stderr reports `no credentials for provider
  "openrouter"`, and the runs completed on the offline faux provider. Every reply here is
  protocol shape, not model behaviour.
- **A fork of a fork is untested** over the wire, and the in-process tests do not cover it
  either.
- **Two sessions open at once is untested.** The second prompt failing with `another run is
  already active` is a *run* guard, not a session guard; two concurrent sessions in one
  process were not exercised.
- **`export_html` and `bash` were not probed** — both unimplemented.

---

## Re-probe after M193 — both defects fixed on the shipped artifact

**Binary:** 21,466,112 bytes (M192's was 21,486,080 — the size moved, which is the first
sign anything changed).

`type` went from `None` to `message`, and both ops now return **byte-identical** entries:

```json
{"id":"ent_1a0e7aa7b682","parentId":null,
 "timestamp":"2026-09-28T10:58:24.744Z","type":"message"}
```

The internal names are gone: no `kind`, no `parent_id`, no `seq`, no `timestamp_ms`.

All three session files in one directory now use the store's convention:

```
2026-09-28T10-58-18-021Z_ses_1a0e7aa61250.jsonl   <- fork       500 bytes
2026-09-28T10-58-18-051Z_ses_1a0e7aa61430.jsonl   <- new_session 160 bytes
rpc.jsonl                                          <- original
```

And the M192 op set still holds end to end: fork 2 → 1 entry with the tree at one root;
`new_session` at 0 entries and `get_messages` a bare `ARRAY[0]`; `switch_session` back to
2 entries.

Still no real provider call — stderr reports no credentials and the runs used the offline
faux provider. Protocol shape only.
