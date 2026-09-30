# 021 — a resumed launch opens its session once (A19)

**Ledger row:** A19 (open, high). **Status:** specified, not started.
**Where:** `crates/epsilon-cli/src/main.rs`, `build_runtime`. **Estimate:** one slice, ~1 hour with a
warm target, plus a console check.

## The defect, in one sentence

A resumed launch **opens two stores** — the primary one it will write to (`:1813`, attached at
`:1877`) and the one it resumes (`:1891`) — and since M210 the primary is a **fresh per-launch file
that is the newest in the directory**, so `latest_session_file` returns the file the process just
created, the second open waits on **its own lease** for the full `STALE_AFTER` window, and the
conversation it finally restores is **empty**.

Measured: a 21/22/24-second countdown with **no other process alive** and the lock files deleted.
The mechanism is in [A19](../../audit/03-defects.md); this is the execution.

## The two edits

### 1. Resolve the resume path before opening anything, and open *it*

Move the `resume_path` match (currently `:1882-1888`) to immediately before the primary open
(`:1812`), then:

```rust
let (session_path, resuming) = match resume_path.clone() {
    Some(rp) => (rp, true),                       // A19: write where we read
    None => (workspace.join(session_file), false),
};
```

Everything between the open and the attach (`:1813-1877`) then operates on **the file being
continued** — which is what it should always have done: migration, repair and the unknown-kind
notice all belong to the file that holds the conversation.

### 2. Make the resume block *use* that store instead of opening a second one

The block at `:1889-1956` currently owns `restored`. It should borrow the attached store instead:

```rust
// A19: the store is already open and attached (step 1 opened the resumed path as the primary).
// Opening it again here deadlocked on our own lease for a full STALE_AFTER window, and the
// conversation it then restored was the empty file this launch had just created.
if resuming {
    let handle = agent.session_store().expect("attached above");
    let mut guard = handle.lock().map_err(|_| "session store lock poisoned")?;
    // … repair_on_open(&mut guard), preserved_unknown_notice(), model_view(), resumed_model_to_use(&guard, …)
    let msgs: Vec<Message> = guard.model_view()?…;
    drop(guard);                       // before restore_history, which is not about the store
    agent.restore_history(msgs);
}
```

Three details that will otherwise cost an hour:

- **Migration is now done once, by the primary path** (`:1824` already has the `store.migrated()`
  branch). Delete the resume block's copy rather than keeping both, or the same file is migrated and
  persisted twice.
- **Drop the guard before `restore_history`.** It is not a borrow-checker formality: holding it
  across an agent call makes a future refactor that touches the store inside `restore_history`
  deadlock, which is the defect being fixed.
- **`repair_on_open(&mut guard)`** needs `&mut SessionStore`; `guard` derefs, so this compiles — but
  it means the repair runs *after* attach, where before it ran before. Nothing reads the model view
  in between, so the order is safe; say so in a comment, because a reader will ask.

## Evidence and verification

1. **The prediction first**, one command, before touching code: in a scratch workspace with two
   sessions, run `-c`, then check whether a **third** file appears, whether it is **empty**, and
   whether the countdown exceeds the four-second settle. This is A19's own "not yet established"
   item and it distinguishes "reads one file, writes another" from "reads and writes the same".
2. **After the fix:** `-c` in the same workspace must (a) create **no** new file, (b) show the
   resumed conversation on screen past the wait — the capture the conhost probe could not reach, so
   run it by hand or give the probe a post-submit wait — and (c) start without a thirty-second
   countdown.
3. **Tests:** `crates/epsilon-cli` has no test that drives `build_runtime`, so the cheapest
   regression net is an assertion that a resumed launch's `session_path` equals the resolved resume
   path (a small extracted helper is the natural place — the same trick `resumed_model_to_use`
   already uses to be testable without a workspace).
4. **Battery:** `pwsh -File scripts/gate.ps1 m212` with the M208 disk knob.

## What NOT to do

- **Do not** release the primary lease just before the second open. It fixes this case and permits a
  real second writer whenever the paths differ — the corruption the lease exists to prevent.
- **Do not** keep both opens and make the second one "wait less". The wait is not the defect; the
  second open is.
- **Do not** fix this by reverting M210's per-launch naming. One session per file is right (A16,
  plan 020); the bug is that resume was already writing to a different file *before* M210 and the
  fixed name hid it.

## Docs the fix must carry

- `docs/milestones/M210-a-session-file-per-launch.md` — its withdrawn "`-c` improves without being
  touched" sentence becomes a corrected statement of what actually changed.
- `CHANGELOG.md` — the same sentence is in the user-visible entry.
- `audit/03-defects.md` — A19 closes, with the measurement that closed it.
