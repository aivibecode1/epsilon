# M216 — a line-ending policy, and the tree that was drifting from it

**Ledger row A9, plan [`022`](../plans/022-line-endings.md).** The repository has always stored LF.
What it never had was a file saying so — so a Windows Git install decided for it, and the result was
a working tree that disagreed with every commit while Git reported it clean.

## What

```text
.gitattributes     * text=auto eol=lf  (+ *.png/*.ico/*.exe binary)
```

- **`eol=lf`, not bare `text=auto`.** `text=auto` alone fixes what is *stored* and leaves the
  checkout to the platform — on this machine that is CRLF, because `core.autocrlf=true` ships in
  `C:/Program Files/Git/etc/gitconfig` as the Git for Windows installer default. Declaring the eol
  is what makes the working tree match the repository instead of merely being reconcilable with it.
- **Binaries are named explicitly** so a later `*` rule cannot start treating `assets/Logo.png` as
  text. It is the one file in the tree that still contains CR bytes on disk, which is what binary
  means.
- **`*.lock` is deliberately not in the binary list.** The plan proposed it for session lease files;
  those live under `.epsilon/` at runtime and are never tracked, while the one `*.lock` this
  repository does track is `Cargo.lock` — which wants a readable diff, not an opaque blob. The plan
  was written from the class of file rather than from the file, and this is the correction.
- **The renormalization is its own commit and contains nothing else** (plan 022's requirement), so a
  bisect never lands on it.

## The measurements that define the job

Taken at M215, in the working tree, before anything was changed:

| Where | Files containing CR |
|---|---|
| working tree, tracked | **155** of 768 (154 text + `Logo.png`) |
| the committed blobs | **5** text files, plus `Logo.png` |

So the repository was *mostly* LF already, and the drift was almost entirely in the working tree —
which is why nothing looked broken. Git hid it: for a file whose stat cache entry matched, `status`
never read the content, so 154 CRLF files reported as clean. The damage arrived the day someone
edited one of them, because the edit rewrote the whole file with CRLF and the diff then showed every
line. M215 paid that once, on a one-line change in `epsilon-tools/src/lib.rs`: **a 185-line diff of a
173-line file**.

The five committed ones are themselves the argument for the policy: two of them are the gate logs
committed *one commit before* this milestone. PowerShell writes logs with CRLF, so the day the
policy was missing, the evidence for the fix became another instance of the problem.

## Key decisions

- **Fix the working tree too, and disclose the cost.** Leaving 154 files CRLF would have been
  defensible on paper — under the new filter they no longer produce diff noise — but it leaves the
  thing a reader opens different from the thing the repository stores, and it keeps printing
  `CRLF will be replaced by LF` at every checkout. They were converted. The cost is real and was
  paid once: their mtimes moved, so the next battery recompiles the crates that contain them.
- **Convert by bytes, not by line-oriented tools.** The conversion removes a CR only when it is
  followed by LF, and only in files `git check-attr` calls text, so `Logo.png` was skipped rather
  than corrupted.
- **No new gate script.** See `Limits`.

## Two traps, both measured rather than reasoned about

- **`git checkout-index -a -f` does not refresh the working tree's endings.** It is the standard
  recipe and it is the wrong tool here: under `text=auto eol=lf` the CRLF working copy already
  counts as clean, so the checkout is content-identical and writes the same bytes back. Measured —
  **155 CRLF files before, 155 after**, exit 0.
- **A stale stat cache looks exactly like a dirty tree.** After the conversion, `git status
  --short` listed all 154 files as modified, and `git update-index --refresh` answered
  `<path>: needs update` for each — while `git diff` was **empty**, and

  ```text
  git hash-object --path=<f> <f>   ==   git rev-parse :<f>   ==   git rev-parse HEAD:<f>
  ```

  held for every one of them. The index's cached stat data described files that no longer existed in
  that shape. A second `git add --renormalize .` cleared it and **staged nothing**. The lesson
  generalizes past line endings: when `status` and a content hash disagree, the hash is the answer.

## Verification

**The change is line-endings-only, proved two ways** — both come back **empty**:

```text
git diff --cached --ignore-cr-at-eol --stat      (empty)
git diff --cached --ignore-all-space  --stat     (empty)
```

**The class of defect is gone, by probe rather than by argument.** Appending one line to a file that
was CRLF before the change (`docs/milestones/M70-message-sanitization.md`) now yields

```text
$ git diff --numstat -- docs/milestones/M70-message-sanitization.md
1       0       docs/milestones/M70-message-sanitization.md
```

one insertion, no deletions — where the same shape of edit produced a 185-line diff at M215.

**The tree is consistent:** `git grep -l $'\r'` over the working tree returns one file,
`assets/Logo.png`; `git status --short` returns nothing.

**Battery `m216` — GATE PASS**, `audit/evidence/gate-m216.log`: all seven gates, **1,634 tests across
65 binaries, 0 failing** — *identical* to M215's totals, which is the result a line-endings-only
change should produce and the one that would have been impossible to fake by coincidence: the
working tree's mtimes moved across every crate, so the `test` gate genuinely recompiled them
(462 s against M215-final's 179 s) and the numbers came out the same. Gate shape: check-links 0 s,
check-lint-tables 0 s, fmt 3 s, clippy 47 s, test 462 s, demo 20 s, real-check 9 s.

This battery is the one that matters for this slice, because a line-ending change is exactly the
kind that can break a fixture which compares a file's bytes to a multi-line string. **Nothing did**:
no fixture in the tree was reading endings. `check-links` and `check-lint-tables` parse rather than
compare, so damage to them would have shown as a failure rather than a silent pass, and both passed
first (0 s each) — the sessions under `.epsilon/` and every JSONL fixture in the tree are already
LF-authored and were unaffected.

Run with `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0` (M208's disk knob, disclosed
in every milestone since): a panic backtrace from this round would be close to useless, so a failure
would be re-run with debug info back on before being believed. The full battery cost 4.19 GB with
the knob (measured at M208) rather than the ~33 GB the same run costs with debug info on.

## Limits

- **No gate enforces the policy.** If someone deletes `.gitattributes`, nothing fails — the next
  Windows checkout simply starts handing out CRLF again, silently, exactly as before. A CI check is
  the obvious follow-up (`.github/workflows/ci.yml` already runs the battery on Windows) and it is
  not in this slice.
- **The worktree refresh was done by hand, once.** A contributor with an existing CRLF clone gets
  the new filter immediately (their diffs stop being whole-file) but their files stay CRLF until
  something rewrites them; the warnings they see until then are correct, not a bug.
- **The 154 converted files were not individually reviewed** — the conversion is a pure byte
  transform, and the `--ignore-cr-at-eol` diff being empty is what says so.

## Nothing exercises

No test in the suite reads `.gitattributes`, asserts the tree is LF, or would fail if the policy
were reverted. The property this milestone establishes is about the *repository*, not about the
program, and the program's tests cannot see it. That is the honest reason the check above is a
follow-up rather than a claim.

The verification that *was* run is therefore the only verification there can be, and it is worth
naming what it is: probes against git itself (the two empty diffs, the one-line append, the hash
equality) rather than a check inside the suite. **No `epsilon.exe` reinstall is needed** — nothing
in this milestone is reachable from the binary; it is a repository property, and the next CLI-visible
change will carry the reinstall.

## The follow-up this leaves, written down

A CI step that fails when `.gitattributes` is missing or when a tracked text file in the index
carries a CR byte. It belongs in `.github/workflows/ci.yml` (which already runs the battery on
Windows) and it is a *separate* slice, because the check has to be written to be right about
binaries and about contributors with an un-refreshed clone — neither of which is a thing to settle
in the same change that creates the policy.
