# Criterion 4 — code quality sweep

Audited 2026-10-14. **The check that found the defect class was `cargo doc`, not clippy.**

## LANDED — 49 broken intra-doc links, now 0

`cargo doc --workspace --no-deps` reported **49 unresolved links across 12 files**, and no
gate in the battery looks at it — which is why they accumulated. Each one is a comment
that asserts a symbol exists where a reader can go look, and cannot:

> "Keybinding this action runs, resolved through [`KeybindingsManager::key_text`]"

…pointing at a symbol not in scope in that module. Others named `Self::method` from the
wrong module, symbols in sibling crates, and one where rustdoc parsed `argv[1]` in prose
as a link to an item named `1`.

Fixed: **46** converted to plain code spans (the honest form when the target is not
resolvable from here), **3** given their real paths, which is better than downgrading a
genuine cross-module reference:

- `IgnoreSet` → `crate::ignore::IgnoreSet`
- `IgnoreSet::is_ignored` → `crate::ignore::IgnoreSet::is_ignored`
- `CURRENT_SESSION_VERSION` → `epsilon_contracts::session_entry::CURRENT_SESSION_VERSION`

`cargo doc --workspace --no-deps` now reports **0 warnings**.

## The trap in doing this mechanically

A script that rewrites these has to find the opening `[` of each link. Rustdoc's caret
does **not** sit on the bracket — it sits on the link's *path text*, a variable number of
characters in. My first script assumed the bracket and reported "0 changed" while
believing it had worked. **The fix is to `rfind('[', 0, caret)`** — scan back from the
caret.

The same shape as the earlier "clippy clean" claim: a command's exit code read as success
while the work had not happened. The count going 28 → 0 is the only proof it ran.

## Still open in criterion 4

- Duplicated logic audit — bounded per crate; `cargo clippy --workspace` is already at
  0 warnings so the lint-visible half is clean.
- Comment *quality* sweep (outdated, redundant, "what not why") — judgement-heavy,
  scope to one crate per window.
- `cargo doc` is not in the battery. Worth adding as a leg, or this class comes back.

## Status

Broken-link class closed and measured. The remaining two are sweeps, not defects.
