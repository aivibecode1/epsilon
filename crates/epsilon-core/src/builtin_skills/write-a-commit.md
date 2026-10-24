# Write a commit

Use this skill when a change is finished and should become history. A commit is
the one artifact that outlives the session, so it is worth two minutes.

## Inspect before staging

```sh
git status --short
git diff
git log -5 --oneline
```

Read the diff, not just the file list. The worktree may contain changes you did
not make; they are the user's work and must not be swept into your commit.

Stage **explicit paths**:

```sh
git add path/one.rs path/two.md
```

`git add -A` and `git add .` are how unrelated edits and generated files get
committed. Do not use them unless the whole tree is known to be yours.

## One logical change

- The commit answers one question. A fix and a refactor are two commits, even if
  they touch the same file.
- Tests belong with the behaviour they prove; documentation belongs with the
  behaviour it describes.
- Generated files, caches, secrets, and local configuration do not belong in a
  commit. Check `.gitignore` before adding a new artifact.
- Never commit a credential, token, private key, or `.env` file. If one is
  already staged, unstage it and tell the user; do not "clean" history silently.

## Message

Subject: imperative, at most 72 characters, no trailing period.

```text
fix: refuse a settings write when the file is unreadable
```

Body, when the subject is not enough: why the change exists, what it replaces,
and any trade-off a reader would otherwise rediscover. Wrap at 72 columns. Cite
an issue or a reference by URL when the reasoning comes from one.

Do not write "various fixes" or "update code". A message that fits every commit
fits none.

## Verify last

Run the project's test or verification command **before** committing, not after.
If the repository's convention is a full battery, run it; a green narrow test is
not the same claim. Then check `git diff --cached` once more: this is exactly
what will be recorded.

Push only when asked. Rewriting published history needs an explicit instruction
and a reason that survives being said out loud.
