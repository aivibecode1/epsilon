"""Fail the build on a broken relative link in the documentation.

Why this is a gate and not a one-off repair
-------------------------------------------
The audit documents were moved into `audit/history/` on 2026-09-28 and the
155 links that pointed at them were never relinked.  That was left as a debt and
paid forward: the same class of damage was still present in `docs/user/`,
`docs/design/`, `README.md` and thirteen milestone logs, because nothing checked.

A dead link in a repository like this one is not cosmetic.  These documents are
the *evidence* trail: a milestone log cites a gate log, an audit row cites a
measurement, and a reader follows the citation to decide whether a claim was
checked.  A link that does not resolve is a claim that cannot be followed, and a
claim that cannot be followed is indistinguishable from a claim that was never
made.

Scope and deliberate exclusions
-------------------------------
* `audit/history/` is **excluded**.  It is superseded and deliberately
  unrepaired — rewriting it would point readers of a historical record at
  documents written years later, which is a worse lie than a dead link.  The
  directory says so in `AGENTS.md`; this script honours that.
* External URLs (`http`, `https`, `mailto`) are not fetched.  A link checker
  that requires the network is a gate that fails when someone else's server is
  down, and a gate that fails for reasons outside the change teaches people to
  ignore it.
* `references/` is excluded: it is not committed, and its layout is not ours.

    python scripts/check-links.py            # report, exit 1 if broken
    python scripts/check-links.py --quiet    # summary line only
"""

from __future__ import annotations

import argparse
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

LINK = re.compile(r"\[[^\]]*\]\(([^)]+)\)")

# Directories never walked: build output, vendored references, VCS, and the
# deliberately-unrepaired history tree.
SKIP_DIRS = {".git", "target", "references", "node_modules", "__pycache__", "history"}


def markdown_files() -> list[str]:
    """Every tracked markdown file that is not inside a skipped directory."""
    out: list[str] = []
    for dirpath, dirnames, filenames in os.walk(ROOT):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for name in filenames:
            if name.endswith(".md"):
                out.append(os.path.join(dirpath, name))
    return out


def is_external(target: str) -> bool:
    return target.startswith(("http://", "https://", "mailto:", "tel:", "#"))


def strip_angle(target: str) -> str:
    """`(./a.md)` and `<./a.md>` are both legal markdown; normalise both."""
    return target.strip().strip("<>").strip()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--quiet", action="store_true", help="summary line only")
    args = ap.parse_args()

    broken: list[tuple[str, int, str]] = []
    scanned = 0

    for path in sorted(markdown_files()):
        rel = os.path.relpath(path, ROOT).replace("\\", "/")
        with open(path, encoding="utf-8", errors="replace") as fh:
            for lineno, line in enumerate(fh, 1):
                for match in LINK.finditer(line):
                    target = strip_angle(match.group(1))
                    if is_external(target):
                        continue
                    scanned += 1
                    # The fragment is a same-file anchor, not a path.
                    path_part = target.split("#", 1)[0]
                    if not path_part:
                        continue
                    resolved = os.path.normpath(
                        os.path.join(os.path.dirname(path), path_part)
                    )
                    if not os.path.exists(resolved):
                        broken.append((rel, lineno, target))

    if args.quiet:
        print(f"check-links: {scanned} relative links, {len(broken)} broken")
    else:
        for rel, lineno, target in broken:
            print(f"  {rel}:{lineno}  ->  {target}")
        print(f"\ncheck-links: {scanned} relative links checked, {len(broken)} broken")

    return 1 if broken else 0


if __name__ == "__main__":
    sys.exit(main())
