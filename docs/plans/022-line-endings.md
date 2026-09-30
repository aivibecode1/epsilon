# 022 — a line-ending policy, and the renormalization that has to go with it (A9)

**Ledger row:** A9. **Status:** specified, not started. **Risk:** low, but *tree-wide*, which is why it
is written down rather than done in passing.

## The finding

The repository has **no `.gitattributes`**, and the consequence is visible in every commit made with
git on Windows:

```text
warning: in the working copy of 'docs/history.md', LF will be replaced by CRLF the next time Git touches it
```

That warning is not cosmetic. It says the working tree, the index and the eventual checkout disagree
about line endings — so the *next* `git add` of any touched file rewrites its endings, which means a
one-line edit can arrive as a whole-file diff, and a future contributor on a different platform can
see failures that have nothing to do with their change.

## Why it is not a three-line change

Adding `.gitattributes` is three lines. **Adding it and doing nothing else makes the problem worse**:
`* text=auto` marks every existing file as "should be normalized", so the next `git add` renormalizes
whatever it touches — turning one commit into a tree-wide line-ending rewrite, mixed in with real
work. The file and the renormalization are one change, and the renormalization must be **isolated in
its own commit that contains nothing else**, so that a bisect never lands on it.

## The change, in one commit

```gitattributes
# Every text file is normalized to LF in the repository; the working tree gets the platform's
# ending. This is the policy the repo has been missing — see docs/plans/022.
* text=auto

# Explicit, so a future `*` rule cannot swallow them: these are binary in every sense that matters
# to a diff, a merge or a checkout.
*.png binary
*.ico binary
*.exe binary
*.lock binary
```

Then, **alone in its own commit**:

```sh
git add --renormalize .
git status --short        # the diff must be line endings ONLY — read it, do not trust it
git commit -m "chore: normalize line endings to LF (A9)"
```

## Verification, and what would make it wrong

1. **The diff contains no content change.** `git diff --cached --stat` should show the same files
   with the same insertions/deletions counts as their previous commits; a file whose *count* changed
   is a content change smuggled into a whitespace commit. The check that actually catches this:
   `git diff --cached --ignore-all-space --stat` must come back **empty**.
2. **The warnings stop.** The next ordinary commit must not print "LF will be replaced by CRLF".
3. **The suite still passes.** Run the battery: `pwsh -File scripts/gate.ps1 m215` with the M208 disk
   knob. Line endings inside fixtures matter — a test that compares a multi-line string to a file's
   bytes is exactly what this change can break, and `check-links`/`check-lint-tables` both parse
   files rather than reading them into a string, so a policy that damages them would show as a gate
   failure rather than a silent pass.
4. **A JSONL fixture is the one to watch.** The session files are written by the product with `\n`
   and read back by the product; if anything in the tree is compared byte-for-byte, `text=auto` is
   what would change it. `*.lock binary` above is deliberate (a lock file's bytes are meaningless and
   its mtime is what matters), but `.jsonl` is **left as text on purpose** so diffs of sessions stay
   readable.

## What this does not do

- **It does not change the working tree's endings for anyone who has not re-checked-out.** A
  contributor with CRLF files on disk keeps them until they refresh; the repository's *stored* form is
  what changes.
- **It does not fix `core.autocrlf`.** A machine with `autocrlf=true` in its global config overrides
  the intent for files that are not explicitly marked; this policy marks them, which is the part that
  travels with the repository rather than with a developer's dotfiles.
- **It does not undo the historical mixed endings.** Old commits keep whatever they had; only the
  index going forward is normalised.
