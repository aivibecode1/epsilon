"""Repair the broken relative links left by the `audit/NN-*.md` -> `audit/history/` move.

Why this exists
---------------
On 2026-09-28 the audit documents that had been moved into `audit/history/` were
never relinked. 155 links across 41 files point at paths that no longer exist, and
the damage is not uniform: it ranges from the README's own honesty disclaimer
(`audit/11-current-assessment.md`) to a `../docs/history.md` that is only wrong
because of the depth of the file citing it.

Two failure modes make a blind search-and-replace the wrong tool, so this script
does not do one:

1. **Depth.** The same document is cited as `audit/04-roadmap.md` from the repo
   root and as `../../audit/04-roadmap.md` from `docs/milestones/`. A rename has
   to fix the *tail* and leave the *prefix* alone, because the prefix encodes
   where the citing file lives.
2. **Ambiguity.** A basename can match more than one file. `audit/01-*.md` and
   `audit/14-*.md` are both "current state" in spirit; guessing which one a
   citation meant is how a link gets silently pointed at the wrong document.

So each candidate link is resolved by *searching* for a real file with a matching
numeric prefix, and anything ambiguous or unresolvable is reported rather than
rewritten. Run with `--dry-run` first; it prints every change it would make.

    python scripts/fix-links.py --dry-run
    python scripts/fix-links.py --apply
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from collections import defaultdict

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

LINK = re.compile(r"\[([^\]]*)\]\(([^)]+)\)")

# Files that are deliberately absent: milestone numbers the history skipped, and
# documents this repository has never had. Left alone on purpose.
SKIP_DIRS = {".git", "references", "target", "node_modules", "__pycache__"}


def markdown_files() -> list[str]:
    out = []
    for dirpath, dirnames, filenames in os.walk(ROOT):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for name in filenames:
            if name.endswith(".md"):
                out.append(os.path.join(dirpath, name))
    return out


def numeric_prefix(stem: str) -> str | None:
    """`04-roadmap` -> `04`; `README` -> None."""
    m = re.match(r"^(\d+)", stem)
    return m.group(1) if m else None


def build_index() -> dict[str, list[str]]:
    """Map a numeric prefix to every tracked path carrying it.

    Two files can share a prefix after a move (`audit/01-...` and
    `audit/history/14-...` do not, but renames make it possible), so the value is
    a list and ambiguity is detected rather than resolved by guessing.
    """
    index: dict[str, list[str]] = defaultdict(list)
    for path in markdown_files():
        rel = os.path.relpath(path, ROOT).replace("\\", "/")
        prefix = numeric_prefix(os.path.basename(path)[: -len(".md")])
        if prefix:
            index[prefix].append(rel)
    return index


def resolve(target: str, citing: str, index: dict[str, list[str]]) -> tuple[str | None, str]:
    """Return (fixed_target, status) for one link.

    status is one of: ok, ambiguous, missing, external.
    """
    if target.startswith(("http://", "https://", "#", "mailto:")):
        return None, "external"

    path_part, _, anchor = target.partition("#")
    if not path_part:
        return None, "external"

    resolved = os.path.normpath(os.path.join(os.path.dirname(citing), path_part))
    if os.path.exists(resolved):
        return None, "ok"

    # What was it called, and does a number survive in the basename?
    stem = os.path.basename(path_part)[: -len(".md")] if path_part.endswith(".md") else os.path.basename(path_part)
    prefix = numeric_prefix(stem)
    if not prefix:
        return None, "missing"

    candidates = index.get(prefix, [])
    if not candidates:
        return None, "missing"
    if len(candidates) > 1:
        return None, f"ambiguous({len(candidates)})"

    # Keep the citing file's depth, change only the tail. This is the whole
    # reason a global find-and-replace is wrong: `../docs/history.md` and
    # `../../docs/history.md` are different correct answers from different
    # depths.
    new_abs = os.path.join(ROOT, candidates[0])
    new_rel = os.path.relpath(new_abs, os.path.dirname(citing)).replace("\\", "/")
    if not new_rel.startswith("."):
        new_rel = "./" + new_rel
    return (new_rel + ("#" + anchor if anchor else "")), "repaired"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--apply", action="store_true", help="write the changes (default is a dry run)")
    args = ap.parse_args()

    index = build_index()
    changes: list[tuple[str, str, str]] = []
    unresolved: list[tuple[str, str, str]] = []

    for path in markdown_files():
        rel = os.path.relpath(path, ROOT).replace("\\", "/")
        with open(path, encoding="utf-8", errors="replace") as fh:
            text = fh.read()
        original = text

        def repl(m: re.Match) -> str:
            label, target = m.group(1), m.group(2)
            fixed, status = resolve(target, path, index)
            if status == "repaired" and fixed:
                changes.append((rel, target, fixed))
                return f"[{label}]({fixed})"
            if status.startswith(("ambiguous", "missing")):
                unresolved.append((rel, target, status))
            return m.group(0)

        text = LINK.sub(repl, text)
        if text != original and args.apply:
            with open(path, "w", encoding="utf-8", newline="\n") as fh:
                fh.write(text)

    verb = "repaired" if args.apply else "would repair"
    print(f"{verb} {len(changes)} links in {len({c[0] for c in changes})} files")
    for rel, old, new in changes[:15]:
        print(f"  {rel}: {old}  ->  {new}")
    if len(changes) > 15:
        print(f"  ... and {len(changes) - 15} more")

    if unresolved:
        print(f"\n{len(unresolved)} left alone (no unique target found):")
        for rel, target, status in unresolved:
            print(f"  [{status}] {rel}: {target}")

    if not args.apply:
        print("\nDry run. Re-run with --apply to write.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
