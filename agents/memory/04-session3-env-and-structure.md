# Session 3 — environment, structure, then the port (2026-09-28)

**Status: open.** Baseline established; the build was broken for a reason no
document recorded, and it is now fixed.

## The blocker nobody wrote down: a non-interactive shell cannot build this repo

**The battery failed 4/4 on my first run**, with `exit=101` in clippy, test,
demo and real-check — the shape of a catastrophically broken tree. It was not.
The real error, buried 70 lines into the first gate:

```
error: failed to run custom build command for `aws-lc-sys v0.44.0`
  process didn't exit successfully: build-script-main (exit code: 1)
  warning: Compilation of 'stdalign_check.c' failed ... "cl.exe" ... (exit code: 2)
```

`reqwest`'s `rustls` feature compiles **`aws-lc-sys`**, a C library. `cc-rs`
finds `cl.exe` by itself, so the error is *not* "no compiler" — it is `cl.exe`
running and **failing**, because in a non-interactive shell:

```
INCLUDE=[]  LIB=[]  VCINSTALLDIR=[]
```

`cl.exe` with no `INCLUDE` cannot find the Windows SDK headers. Exit code 2, per
file, for the whole library.

**The cost of the misreading is the point.** Four red gates read as "the tree is
destroyed" and would have sent the next agent hunting a code regression that did
not exist. A gate that fails in *every* stage is usually the environment, not
the change — and this repository's own rule already says so in a different
place: *every gate failure at parallel jobs has been `0xc0000142`*, an
environment failure dressed as a build failure.

**Fix:** `scripts/gate.ps1` runs `vcvars64.bat` and imports its variables when
`INCLUDE` is empty. Verified: `INCLUDE set: True / LIB set: True` when loaded.
`scripts/gate.sh` is POSIX/CI and does not need it — the runners have a toolchain.

**Generalise it:** *when every gate fails at once, suspect the environment
before the code.* One `env | grep` answers it in seconds and the alternative is
an hour of reading a diff that cannot be the cause.

## The documentation debt I measured myself

**40 broken relative links**, all fallout from the 2026-09-28 audit renumbering
(`audit/NN-*.md` renumbered **onto different subjects**). Now **0**, with
`scripts/check-links.py` as a permanent gate wired into both battery scripts.

The trap worth recording: the old `scripts/fix-links.py` resolved broken links
by **numeric prefix**, which after a renumbering sends *the defect ledger* to
*port fidelity* and calls it a fix. I replaced it with an explicit, verified
mapping table. **A rename that reassigns numbers needs a table keyed on the old
name, checked against the citation's own text — never a prefix match.**

Also fixed: 8 links whose file existed at the right place but was cited from the
wrong depth, and 22 link *labels* that still named the pre-rename file (a
working link whose text disagrees with its target is a link that lies).

## G7 markdown, confirmed by reading the code (not the audit)

`crates/epsilon-tui/src/markdown.rs:36` parses with
`Options::ENABLE_STRIKETHROUGH` and **no other extension**. So:

- **tables** — not parsed; a table renders as run-together paragraph text
- **task lists** — not parsed; `- [ ]` renders literally
- **strikethrough** — *parsed but unhandled*: there is no
  `Event::Start(Tag::Strikethrough)` arm, so the option is a setting that
  claims to work and does not

That last one is the repo's own `app.clear` defect in a different costume, and
it is the most-used markdown feature after bold and code. **Small, and a user
meets it daily.**

## What I did NOT do, deliberately

- **No builds or writes to `D:`** — owner instruction this session. Everything
  is on `C:` in the repo's `target/`, which is also what `AGENTS.md` requires.
- **Did not touch product source while a battery ran**, per the AGENTS.md rule
  that has already cost this project three logs. The link work was docs-only
  (zero build impact) and is separate from the code work.
- **Did not repair `audit/history/`'s 146 links.** Deliberate: rewriting a
  historical record to point at documents written later is a worse lie than a
  dead link. `check-links.py` excludes that directory on purpose.

## Next safe actions

1. Land the M196 battery with the MSVC fix and record the real test count.
2. **G7 markdown** (tables, task lists, strikethrough) — small, high visibility.
3. **`branch_with_summary` production caller** — pi's `navigateTree`; the one
   remaining instance of the "faithful port, no production caller" class.
4. Session tree surface, then repo layout + docs rewrite.

## Machine

`CARGO_BUILD_JOBS=1`, no `CARGO_TARGET_DIR`, build in the repo's `target/`,
`pwsh -File scripts/gate.ps1 <label>` (not gate.sh — that is WSL here).
