"""Fail the build on a parity-matrix citation that does not resolve.

Why this is a gate and not a one-off repair
-------------------------------------------
The parity matrices (`docs/parity/pi-tui.md`, `docs/parity/dsh-desktop.md`) are
the evidence trail for the whole port: every row names the production lines
that establish it.  Moving code — even a pure 53-line deletion, as the export
move in round 14 did — silently re-points every citation below it, and a row
that cites the wrong lines is a claim that cannot be followed.  Round 14
re-pointed nineteen by hand; the twentieth such move should fail the build
instead.

Scope and deliberate exclusions
-------------------------------
* Only the two matrices are scanned.  Other documents cite code too, but the
  matrices are the tables whose rows are counted, so they are where a dangling
  citation corrupts a number.
* Only workspace-owned paths (`crates/`, `docs/`, `scripts/`, `audit/`) are
  checked.  Reference-side citations (`packages/`, `ui-*/`, `apps/`) point
  into `references/`, which is not committed — like `check-links.py`, this
  gate does not depend on it.
* Directories count as resolved: a bare `` `crates/epsilon-desktop` `` names a
  crate, not a file.
* This checks that the cited lines *exist*, not that they still say what the
  row claims.  Existence catches moves and deletions; meaning stays a
  reviewer's job, and the script says so wherever it reports.

    python scripts/check-parity-citations.py            # report, exit 1 if broken
    python scripts/check-parity-citations.py --quiet    # summary line only
"""

from __future__ import annotations

import argparse
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

MATRICES = (
    os.path.join(ROOT, "docs", "parity", "pi-tui.md"),
    os.path.join(ROOT, "docs", "parity", "dsh-desktop.md"),
)

# A backticked workspace path with an optional :line or :start-end suffix.
# Bare crate names (no line) are crate-level citations and resolve as dirs.
CITE = re.compile(r"`((?:crates|docs|scripts|audit)/[^`:\s]+?)(?::(\d+)(?:-(\d+))?)?`")

ROW_ID = re.compile(r"^[A-Z]+-\d+$")
SENTINEL = "\x00"


def data_rows(path: str):
    """Yield (line_number, row_id, ours, verified) for each data row."""
    with open(path, encoding="utf-8") as f:
        lines = f.read().splitlines()
    for number, line in enumerate(lines, 1):
        if not line.startswith("|"):
            continue
        cells = [
            x.strip().replace(SENTINEL, "|")
            for x in line.replace("\\|", SENTINEL).strip().strip("|").split("|")
        ]
        if len(cells) != 6 or not ROW_ID.match(cells[0]):
            continue
        yield number, cells[0], cells[4], cells[5]


def check():
    """Return a list of (matrix, line, row, citation, problem)."""
    problems = []
    checked = 0
    for matrix in MATRICES:
        for number, rid, ours, verified in data_rows(matrix):
            for cell in (ours, verified):
                for m in CITE.finditer(cell):
                    checked += 1
                    rel, start, end = m.group(1), m.group(2), m.group(3)
                    target = os.path.join(ROOT, *rel.split("/"))
                    if not os.path.exists(target):
                        problems.append(
                            (matrix, number, rid, m.group(0), "no such file or directory")
                        )
                        continue
                    if start is None:
                        continue
                    try:
                        with open(target, encoding="utf-8", errors="replace") as f:
                            total = sum(1 for _ in f)
                    except OSError as e:
                        problems.append((matrix, number, rid, m.group(0), f"unreadable: {e}"))
                        continue
                    lo, hi = int(start), int(end) if end else int(start)
                    if not 1 <= lo <= hi <= total:
                        problems.append(
                            (
                                matrix,
                                number,
                                rid,
                                m.group(0),
                                f"line {lo}-{hi} outside 1..{total}",
                            )
                        )
    return checked, problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--quiet", action="store_true", help="summary line only")
    args = parser.parse_args()
    checked, problems = check()
    names = [os.path.basename(m) for m in MATRICES]
    if not problems:
        print(f"parity-citations: {checked} citations in {names} resolve")
        return 0
    if args.quiet:
        print(f"parity-citations: {len(problems)} broken of {checked}")
        return 1
    print(f"parity-citations: {len(problems)} broken of {checked} citations:")
    for matrix, number, rid, cite, problem in problems:
        print(f"  {os.path.basename(matrix)}:{number} [{rid}] {cite}: {problem}")
    print("Existence only: a resolving line can still say the wrong thing.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
