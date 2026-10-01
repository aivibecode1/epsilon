# M232 — a gate for the failure that lied about itself

**The third line-ending incident of this stretch, and the one that decided this be a gate.** M231's
battery failed with a message about a dispatcher that did not exist, because one file had CRLF endings.
Nothing in the toolchain pointed at the cause: `.gitattributes` normalises on commit, so `git diff`
called the file unchanged.

## What the check does

`scripts/check-line-endings.py` — every tracked, non-binary file, for a `\r\n`, reported as
**file:line** so the hunt is a look rather than a search. It reads `.gitattributes` for which suffixes
are binary rather than keeping its own list, because two lists answering the same question can
disagree, which is the mistake A12 records in the config layer.

Wired into `scripts/gate.ps1` and `scripts/gate.sh` beside the other two `check-*` gates. It costs
under a second, which is the argument: a check that runs in the first three legs of a battery can
prevent a failure that costs the last one.

**It found 27 files on its first run**, twenty-four of them battery logs (excluded, with the reason
recorded in the script: they are regenerated per run, never parsed, and normalising them would
rewrite recorded evidence byte-for-byte) and **three source and doc files I had broken with
PowerShell writes earlier in the same session** — `crates/epsilon-session/tests/session_store.rs`,
`audit/03-defects.md` and `docs/plans/019-one-config-reader.md`.

**And it caught me again, one command after I wrote it.** Normalising `docs/history.md` used
`WriteAllLines`, which writes CRLF on Windows; the next command's `check-line-endings` reported
`docs/history.md:1`. The check is not theoretical for this repository — it is the failure mode of
the tool this session has been using to edit files.

## The control

A tracked source file converted to CRLF fails with the file and the line:

```text
check-line-endings: 1 tracked file(s) carry CRLF endings

  crates/epsilon-cli/src/runtime.rs:8
```

and the tree is clean again once the file is restored.

## Verification

**Battery `m232`** — the gate runs in it, and the two failures it caught while being wired in are
part of the record: a stray `'` that a here-string terminator left at the end of the M231 history row
(`the_history_timeline_is_structurally_sound` named it), and a CRLF normalisation under `crates/`
that `no_source_file_lost_a_test_since_head` correctly refused to ignore — that guard sees a
status-dirty file whose content diff is empty, which is exactly what `.gitattributes` normalisation
looks like from git's side. Both were fixed in this commit; the second one is *committed* rather than
reverted, because leaving a CRLF working copy in the tree is the defect.

Test totals are in `audit/evidence/gate-m232.log` and the milestone row in `docs/history.md`.

## Limits

- **It checks the working copy, not the committed content.** That is deliberate — the committed content
  is LF by policy, and the working copy is the thing that bites — but it means a fresh clone passes
  trivially and only a machine that has been edited in place can fail it.
- **It does not repair anything.** A file is fixed where it was edited. A check that rewrote files
  would fight the next editor, and the message says what to do instead.
- **A repository could grow a legitimate CRLF file** (a fixture asserting CRLF handling, a `.bat`
  script). The check has no exception list for text fixtures, so one would fail here and need a
  decision rather than a workaround — which is the right place for that decision to be made.
