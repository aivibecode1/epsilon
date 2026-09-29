# M199 — a link is not a name: workspace containment follows symlinks

**Status:** landed. GATE PASS `m199d` (shipping battery; `m199c` covers the same code
pre-doc) — 1,568 tests across 63 binaries, 0 failed, 7 of 7 gates.
Reinstall: **not required** — nothing here is CLI-visible.

## What

Workspace containment was decided **lexically**. A path was normalized for `.` and
`..`, compared against the root with `starts_with`, and that was the whole boundary.

So this, committed to a repository the user opens:

```
creds -> /home/user/.aws
```

spells its way *inside* the workspace. No `external_dir` ask is ever raised, and
`write` is `Allow` by default — so a model writing to `creds/credentials` wrote to
`~/.aws/credentials` with no prompt, every layer that would have asked agreeing the
path was fine. A repository is untrusted input the moment a user opens one, and this
is the whole attack in one line.

The same rule was implemented **twice** — once in `epsilon-core/src/paths.rs` and
again, privately, inside `gate_subject` in `exec.rs` — and two lexical copies of one
rule cannot disagree visibly, because neither consults the filesystem. That is the
"faithful port, no production caller" shape pointed the other way: a rule that is
everywhere, implemented nowhere it matters.

## The fix, and why it is two functions

`paths::resolve_in_workspace` split in two, because the two callers want different
answers and conflating them is what made the original rule wrong:

| | `lexically_normalize` | `resolve_for_access` |
|---|---|---|
| answers | where does this spelling point | may we touch this |
| follows symlinks | no | yes |
| for | a **planner key** — `path_key`/`search_key` scheduling identity | every **enforcement** site |

A planner key must stay stable when a symlink is retargeted, or two calls onto one
real file get scheduled as independent. An enforcement site must ask the filesystem.
Keeping the planner keys lexical and the tool sites resolved is the whole design.

`resolve_for_access` canonicalizes the **deepest existing prefix** and re-appends
what is not there yet, which is what makes `write` judgeable: the leaf of a new file
cannot be canonicalized, but its directory can. A path that cannot be resolved at
all **fails closed**. A root that cannot be resolved falls back to the lexical
answer — there is no directory behind it, so there is nothing to escape *from*, and
reporting everything as outside would turn "the directory is gone" into a
confinement error that teaches users the message means nothing.

Six call sites moved to the one function: `read`/`write`/`edit` (via
`resolve_or_refuse`), `find`, `grep`, `glob`, `ls`, and the `@`-reference expander.
`escaped_workspace` now names the real target — `notes/creds` resolving to
`~/.aws/credentials` is a fact the user needs, and a bare "outside the workspace" on
a path visibly in their own tree is a fact they will not believe.

## The bypass this slice would otherwise have shipped

Containment began answering on the real path; the **sensitive-path check did not**,
because `is_sensitive` is name-based and `references.rs` still passed it the lexical
spelling. Two shapes followed, both *inside* the workspace by the check above:

- `@vcs/` where `vcs -> .git` — the entries are innocent names; the directory they
  came from is not.
- `@notes/env` where `env` is a file link to `.env` — the name betrays nothing.

Both now judge the resolved path as well as the spelled one. This is why the field is
`real: Option<PathBuf>` rather than `escaped_to: Option<PathBuf>`: the escape case was
the one the *error message* needed, and the real path is the one every **check**
needed. Keeping only the former would have fixed the loud case and left the quiet one.

Negative control, reverted in place and restored with a `sha256sum -c` check —
`audit/evidence/m199-negative-controls.md`:

```
@vcs/ reached .git through a link: "config"
```

The body is `config`: not refused, so `.git/config` was on its way into the prompt.

## Two things the work found that were not in it

**The M197 stdin guard caught the new code.** `test_support::dir_link` shells out to
`mklink /J` and I did not set stdin on it — the exact defect class
`no_production_spawn_leaves_stdin_unset` was written for, in a module I was adding
while that test existed. The guard found it on the first battery. It is the argument
for that guard's shape: the M197 fix was applied to five sites and three were missed,
and *this* time a new site could not ship without being caught.

**Windows canonicalization returns the `\\?\` verbatim form**, so a refusal would
have said "it is a link to `\\?\C:\Users\…\id_rsa`" — true, and useless to the person
who has to act on it. `paths::display_canonical` strips the prefix and rewrites
verbatim UNC to `\\server\share`. It is one implementation in the layer that owns the
artifact, used by both the tool error and the approval subject.

The same artifact made **my own test wrong before the code was**: it spelled the
expectation with `join` and disagreed with a canonicalized `real` path.
`inside` was never affected — it compares two canonicalized paths — so the
containment logic was right and the assertion was not. Third instance of that shape
in two milestones; the expectation is now canonicalized too, and the comment says why.

## Testing it on the platform the bug was reported on

`std::os::windows::fs::symlink_dir` needs `SeCreateSymbolicLinkPrivilege`, which a
stock Windows machine does not hold, so the honest test would have skipped itself
exactly where the defect lives. A **junction** needs no privilege and is followed by
`canonicalize` identically to a symlink, so `test_support::dir_link` makes one via
`mklink /J`. Running the test with `--nocapture` shows exactly which shapes executed:

```
skipped: this platform would not create a file link
```

The junction shape **ran** — that is the one carrying the portable assertions. The
file-symlink shape **skipped out loud**, which is why `file_link` exists and why it
prints: a green test that asserted nothing is the failure mode this repository has
paid for repeatedly.

## The frame harness is now a tool, not scratch

`epsilon-tui/src/app/shot.rs` arrived in the working tree labelled "SCRATCH — revert
after use". It is not reverted, because the next milestones need it and M198 proved
what it finds: reading a rendered frame found the doubled marker glyph that ~450
lines of documentation about the gutter system had missed. It is
`app/frame_dump.rs` — `#[ignore]`d so the battery stays readable, one command to run,
`EPSILON_FRAME_DIR` (absolute, and it **refuses** a relative one) to write evidence.

It also immediately produced three findings, recorded in
`agents/memory/frame-reading-pass.md` and heading the TUI work: **a word is mutilated
at 60 columns** ("and then" → "and the"), **the header drops its `↑` at 60 columns**
and leaves a bare `0`, and the `?`/`CH n/a` legend is still missing.

## Limits — what this does not prove

- **Nothing here reproduces the reported TUI freeze.** This slice is a real
  confinement defect with a silent-write consequence; it is not, on any evidence
  gathered here, the freeze. Four theories remain disproved and the fifth has not
  been attempted.
- **The file-symlink shape is unproven on this box.** It is covered on POSIX and
  skips on Windows. The junction shape covers the same resolution code path, and the
  file-name half of the bypass is a one-line `is_some_and(is_sensitive)`, but the
  end-to-end file-link case is asserted nowhere on this platform.
- **A directory literally named `.env`** is not caught by `is_sensitive`: the
  component allowlist is `.git`/`.ssh`/`.aws`/`.gnupg`, not `.env`. Recorded, not
  chased — it needs a real incident or a decision about what the list is for.
- **The `ReadTool` applies no sensitive-name check at all**, and deliberately so far
  as I can tell: `@` attaches content *silently* into a prompt, while `read` returns
  what the model asked for, visibly, in the transcript. That asymmetry is a judgement
  call, not an oversight, and it is **not recorded anywhere** — which is worse. It
  is a row for the audit, not a change made here.
- **`test_support` is public production surface.** `#[doc(hidden)] pub mod` in
  `epsilon-core` so `epsilon-tools` can build the same fixture rather than keep a
  second answer to "how do I make a link here". It costs a module in the release
  binary and no behaviour; the alternative — two copies of the link helper — is the
  kind of drift this repository has been bitten by.
- **`real` is `None` when the root is unresolvable**, so a caller that wants the
  resolved spelling has to handle the fallback. That is deliberate and documented, but
  it is a second shape for a future caller to get wrong.

## Verification

GATE PASS, 2026-09-29. The shipping battery is **`m199d`**, because it was run after
the documentation edits and therefore covers the bytes in the commit; **`m199c`**
covers the code with the pre-doc tree and is kept beside it. Seven gates, exit 0 each
on `m199d`: `check-links` 0s, `check-lint-tables` 0s, `fmt --check` 2s, `clippy
--workspace --all-targets` 3s, `test --workspace` 220s, `demo` 6s, `real-check` 1s.
**1,568 tests across 63 binaries, 0 failed** — summed from the 63 `test result: ok.`
lines in the log, not read off a previous run's figure. `m199c` measured the same
1,568 with a colder cache (`test` 651 s).

Two earlier attempts are kept because their failures are part of the record:
`m199` (exit 101 — the M197 stdin guard on my own new line) and `m199b` (exit 101 — the
`\\?\` assertion above). Both were read out of the log before anything was changed.

**Reinstall:** not required. Nothing in this slice is reachable from the installed
binary — the TUI surface is unchanged, and the containment change is a refusal path
rather than a new capability. The owner will see it the next time a link is refused.

**One gate is a doc gate and one is a lint-table gate**, so the `m199d` run is not
incidental: `protocol_doc_tests` reads `docs/history.md` and asserts a row exists for
every milestone, so a documentation change in this repository is capable of failing a
test. That is why the doc edits and the verification happened in that order, and why
the cited battery is the later one.
