"""Repair the broken relative links left by the 2026-09-28 audit renumbering.

Run order: `python scripts/check-links.py` first.  This script fixes the
document renames; the checker is what proves there is nothing else wrong.

Why this is a table and not a search
------------------------------------
`audit/NN-*.md` was renumbered, and the old files moved to `audit/history/`.
The naive repair matches on the numeric prefix -- `audit/04-roadmap.md` becomes
whatever file is now called `04-*`.  That is **wrong here**, and wrong in the
worst way, because the numbers were reassigned to different subjects:

    old 02-defect-ledger      ->  new 02 is *port fidelity*
    old 03-port-plan          ->  new 03 is *the defect ledger*
    old 06-reference-comparison -> new 06 is *what to take from the references*

A prefix rewrite sends every reader of the defect ledger to the port-fidelity
document and calls it a fix.  So the mapping below is keyed on the **old
filename**, and each row was checked against the *text of the citation* at every
site -- a link whose label says "defect ledger" must not land on a document about
port fidelity.  `docs/design/epsilon-architecture.md:14` says "the [defect
ledger]'s"; pointing that at port fidelity would be a working link that lies.

    python scripts/fix-links.py --dry-run
    python scripts/fix-links.py --apply
"""

from __future__ import annotations

import argparse
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

LINK = re.compile(r"\[[^\]]*\]\(([^)]+)\)")

SKIP_DIRS = {".git", "references", "target", "node_modules", "__pycache__", "history"}

# old basename -> current path, repo-relative.
#
# 02-07 were renumbered onto DIFFERENT subjects; 08-12 were retired into
# audit/history/ with no successor.  A retired document still gets a link, but
# one that says it is history: a live document that does not contain the claim
# is worse than an honest pointer to the record that does.
RENAMES = {
    "02-defect-ledger.md": "audit/03-defects.md",
    "03-port-plan.md": "audit/02-port-fidelity.md",
    "04-roadmap.md": "audit/07-plan.md",
    "06-reference-comparison.md": "audit/06-references.md",
    "01-usability-review.md": "audit/01-current-state.md",
    "05-verification-log.md": "audit/history/05-verification-log.md",
    "07-inconsistencies.md": "audit/history/07-inconsistencies.md",
    "08-restructure-record.md": "audit/history/08-restructure-record.md",
    "09-revalidation.md": "audit/history/09-revalidation.md",
    "10-tui-enhancement.md": "audit/history/10-tui-enhancement.md",
    "11-current-assessment.md": "audit/history/11-current-assessment.md",
    "12-fresh-reference-audit.md": "audit/history/12-fresh-reference-audit.md",
}


def markdown_files() -> list[str]:
    out: list[str] = []
    for dirpath, dirnames, filenames in os.walk(ROOT):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for name in filenames:
            if name.endswith(".md"):
                out.append(os.path.join(dirpath, name))
    return out


def relativise(new_abs: str, citing_abs: str) -> str:
    """Repo-relative link from the citing file's directory to the target."""
    rel = os.path.relpath(new_abs, os.path.dirname(citing_abs)).replace("\\", "/")
    return rel if rel.startswith(".") else "./" + rel


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--apply",
        action="store_true",
        help="write the changes (default: report only)",
    )
    ap.add_argument(
        "--dry-run",
        action="store_true",
        help="report only; the default, accepted so the flag in the docstring works",
    )
    args = ap.parse_args()

    changes: list[tuple[str, str, str]] = []
    unresolved: list[tuple[str, str]] = []

    for path in sorted(markdown_files()):
        rel = os.path.relpath(path, ROOT).replace("\\", "/")
        with open(path, encoding="utf-8", errors="replace") as fh:
            text = original = fh.read()

        def repl(m: re.Match) -> str:
            whole, target = m.group(0), m.group(1).strip().strip("<>").strip()
            if target.startswith(("http://", "https://", "mailto:", "tel:", "#")):
                return whole
            path_part = target.split("#", 1)[0]
            if not path_part:
                return whole
            if os.path.exists(
                os.path.normpath(os.path.join(os.path.dirname(path), path_part))
            ):
                return whole

            base = os.path.basename(path_part)
            if base not in RENAMES:
                unresolved.append((rel, target))
                return whole
            new_rel = relativise(os.path.join(ROOT, RENAMES[base]), path)
            if "#" in target:
                new_rel += "#" + target.split("#", 1)[1]
            changes.append((rel, target, new_rel))
            return whole.replace(f"({target})", f"({new_rel})")

        text = LINK.sub(repl, text)
        if text != original and args.apply:
            with open(path, "w", encoding="utf-8", newline="\n") as fh:
                fh.write(text)

    verb = "repaired" if args.apply else "would repair"
    print(f"{verb} {len(changes)} links in {len({c[0] for c in changes})} files")
    for rel, old, new in changes:
        print(f"  {rel}: {old}  ->  {new}")
    if unresolved:
        print(f"\n{len(unresolved)} left alone (no verified mapping):")
        for rel, target in unresolved:
            print(f"  {rel}: {target}")
    if not args.apply:
        print("\nDry run. Re-run with --apply to write.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
