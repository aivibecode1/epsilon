# M265 — the workspace walk followed a link out of the workspace

Row **A31** of the defect ledger, which is itself the row a parallel review wrote down two
rounds ago and the project's own review had written down in 2026-09.

## What it was

Both workspace walkers classified every entry with `Path::is_dir()`:

```rust
let is_dir = path.is_dir();      // fs::metadata — FOLLOWS links
...
if is_dir { walk_inner(root, &path, depth + 1, …); }
```

`Path::is_dir()` calls `fs::metadata`, which follows symlinks. So a repository containing

```
docs  ->  C:\Users\<you>\Documents
link  ->  ~/.aws
```

had a **plain `grep` walk it**, print every matching line into the tool result, the transcript
and the durable session log.

**No permission was asked for, and none could have been.** `gate_subject` (`core/src/exec.rs`)
sees only a workspace-relative name, so `grep` resolves to `("grep", "*")` and the default
ruleset **allows** it (`safety/src/engine.rs:110`) — an `external_dir` capability is never
raised, because from the harness's point of view nothing outside was ever named. Every path it
handled began with the workspace.

**Proven, not argued.** With the fix removed, the walk returns:

```
[".env", ".gitignore", "escape/secret.txt", "main.rs", "src/lib.rs"]
```

and the tools' own walker:

```
["escape/secret.txt", "main.rs"]
```

`escape/secret.txt` is a file that does not exist inside the workspace.

## Why a link and not `resolve_for_access`

The containment primitive already existed: `resolve_for_access` canonicalises and compares
against the root, and it is the right answer for **one named file** at an enforcement site.
It was never the right answer here, and that is the part worth recording —

> A walk that descends from the root through **real** directories cannot leave the root. A link
> is the only thing on the way that points somewhere else.

So enumeration does not need a syscall per entry; it needs to refuse to descend into a link.
That is the whole fix, in one predicate, in the crate both walkers already depend on.

## The Windows half is not `is_symlink()`

A **junction** is a reparse point that `FileType::is_symlink()` reports as an ordinary
directory — and creating one needs **no privilege**, so it is both the case that matters most
on this platform and the one the obvious check misses. `paths::is_link` tests the
`FILE_ATTRIBUTE_REPARSE_POINT` attribute from `std::os::windows::fs::MetadataExt`, which covers
symlinks and junctions together. **No new dependency**: the standard library already has it.

The other half was the **file** link. `is_dir()` answers `false` for one, so it was not
descended into — it was **listed**, and the model could then read an out-of-workspace file by
name. Both cases are covered.

## Saying so rather than narrowing silently

A skipped link **spends the walk's budget**, so the caller's existing "this list is partial"
signal can fire. A file tree that says `400 files` when the disk holds 405 through a link is
lying in exactly the way this repository cares most about, and the walk already had a
documented concept for this: *"truncated, not failed."*

## Verification

Battery `m265` — **GATE PASS: 1,879 tests across 76 binaries, 0 failed**, all seven gates,
**on attempt 1** (`audit/evidence/gate-m265.log`). Up from 1,876 at M264.

That first attempt is worth a line of its own: the two batteries before it were stopped by
Avira `LNK1104` on `epsilon_core-4a8081d0361ce74c.exe`, and the only remedy is fresh
test-profile metadata — which this run had, so it **did not recur**, and the test leg ran in
505 s rather than the 1,489 s the cold rebuild cost last time. The pin is on the *output path*,
so a warm tree under different metadata is immune.

The tests **assert their own precondition**, which is the part worth copying: each one creates
the link and then checks `is_link` agrees before it walks anything, so a test that silently
failed to make the link cannot report `ok` on a walk that had nothing to escape through. The
first version of this test *did* exactly that — it skipped on the unavailable half and passed
on the rest, which is the false control `AGENTS.md` warns about.

- `a_link_out_of_the_workspace_is_neither_listed_nor_descended` (core walk, feeds `@`
  completion and the desktop file tree) — **runs on this machine**: a directory junction
  needs no elevation, and the test panics rather than skips if it cannot be made.
- `the_tool_walk_stops_at_a_link_to_the_outside` (the tools' walker, feeds `grep`, `find`,
  `ls`) — same shape, same precondition.
- `a_link_to_a_file_outside_is_not_listed_as_content` — **SKIPPED here, and recorded rather
  than hidden**: creating a file symlink on Windows needs Developer Mode
  (`os error 1314`). The junction half above did run.

**Negative controls**, both run with the fix disabled:

```
nothing behind a link may appear in the listing: […, "escape/secret.txt", …]
nothing behind a link may reach a tool walk: ["escape/secret.txt", "main.rs"]
```

## What nothing exercises

- **The file-symlink half does not run on this machine** (Developer Mode). It is written and
  will run on a machine that allows it; here it says SKIPPED and returns.
- **A link the user wants followed is still refused.** `node_modules` installed as a junction,
  or a monorepo that links its packages, now disappears from `@`, `grep` and the file tree.
  That is the correct default for a security boundary and the wrong one for those
  repositories, and the escape hatch is not built. Recorded rather than designed: the rule
  that makes it safe is "a walk cannot leave the root", and any exception reopens it.
- **This closes the *walk*.** `A33` (`.env.` and `.env::$DATA` bypassing the ask) and `A34`
  (extension/MCP children inheriting the credential environment) are untouched and still open.
- **The 2026-09 review named this and it was never implemented** — `f09-path-policy-map.md:26`
  ("validate implicit root and each traversed resource") and `:29` ("do not follow discovered
  links/reparse directories implicitly"). A finding in the ledger is not a fix, which this row
  spent three rounds demonstrating.