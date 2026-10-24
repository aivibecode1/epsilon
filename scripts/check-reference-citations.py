"""Resolve every reference-side citation in the parity matrices against the vendored trees.

**Why this exists.** `check-parity-citations.py` checks only workspace-owned paths and
deliberately skips the reference column — `references/` is not committed, so a gate that
depends on it cannot run on a fresh clone. The consequence, named by the sixth goal audit:
the reference-side citations are **unverified by construction**. Both matrices say "if
there is no citation there is no row"; a reference citation that points at a missing file
or an out-of-range line is a row built on nothing.

**What it checks.** Every line-cited reference path in the reference column of both
matrices, resolved against that matrix's vendored root (`references/pi-1.3.0/`,
`references/deepseek-harness-dsh-v0.2.1-rc.2/`), for existence and line range. Existence
and range only: this is the same instrument the workspace side already has, on the other
half of the row. It is exit 0 when the vendored tree is absent (a fresh clone) and says so,
rather than failing a build that cannot satisfy it — the same contract `check-links.py`
keeps for `references/`.

    python scripts/check-reference-citations.py            # report, exit 1 if broken
    python scripts/check-reference-citations.py --quiet    # summary line only
"""

from __future__ import annotations

import argparse
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# matrix -> the vendored root its reference paths are relative to.
REFERENCE_ROOTS = {
    "pi-tui.md": os.path.join(ROOT, "references", "pi-1.3.0"),
    "dsh-desktop.md": os.path.join(ROOT, "references", "deepseek-harness-dsh-v0.2.1-rc.2"),
}

# A backticked source path with a :line or :start-end suffix, in the reference column.
# Every extension: the DSH matrix cites CSS for layout and motion rows
# (TurnNavigator.module.css:207, focus.css:3) and a checker that matched only
# .ts/.tsx/.rs/.json/.md would silently skip them.
CITE = re.compile(r"`([\w./-]+?\.[a-z]+):(\d+)(?:-(\d+))?")

# Workspace-owned prefixes: the workspace checker owns these, and a reference
# row that cites our own code beside the reference's is naming the port, not the
# reference (the E-### rows in the Pi matrix do exactly that).
WORKSPACE_PREFIXES = ("crates/", "docs/", "scripts/", "audit/")

ROW_ID = re.compile(r"^[A-Z]+-\d+$")
SENTINEL = "\x00"


def data_rows(path: str):
    """Yield (line_number, row_id, reference_cell) for each data row."""
    with open(path, encoding="utf-8") as f:
        lines = f.read().splitlines()
    for number, line in enumerate(lines, 1):
        if not line.startswith("|"):
            continue
        cells = [
            x.strip().replace(SENTINEL, "|")
            for x in line.replace("\\|", SENTINEL).strip().strip("|").split("|")
        ]
        if len(cells) < 4 or not ROW_ID.match(cells[0]):
            continue
        # columns: id, behaviour, reference, status, ours, verified
        yield number, cells[0], cells[2]


def resolve(root: str, rel: str) -> str | None:
    """The file a reference citation names, under the matrix's vendored root.

    The DSH matrix's `ui-*`, `shortcuts/`, `locale/` and `connection/` paths live
    under `packages/client/` in the vendored tree (`packages/client/AGENTS.md`
    records the layout); `apps/` and `packages/` are used as written. Direct
    first, then the client-package fallback.
    """
    direct = os.path.join(root, *rel.split("/"))
    if os.path.exists(direct):
        return direct
    nested = os.path.join(root, "packages", "client", *rel.split("/"))
    if os.path.exists(nested):
        return nested
    return None


def check():
    """Return (checked, problems, skipped_because_absent)."""
    checked = 0
    problems = []
    absent = []
    for name, root in REFERENCE_ROOTS.items():
        matrix = os.path.join(ROOT, "docs", "parity", name)
        if not os.path.isfile(matrix):
            absent.append(name)
            continue
        if not os.path.isdir(root):
            absent.append(name)
            continue
        for number, rid, reference in data_rows(matrix):
            for m in CITE.finditer(reference):
                rel, start, end = m.group(1), int(m.group(2)), m.group(3)
                if rel.startswith(WORKSPACE_PREFIXES):
                    continue
                checked += 1
                target = resolve(root, rel)
                if target is None:
                    problems.append((name, number, rid, m.group(0), "no such file in the vendored tree"))
                    continue
                try:
                    with open(target, encoding="utf-8", errors="replace") as f:
                        total = sum(1 for _ in f)
                except OSError as e:
                    problems.append((name, number, rid, m.group(0), f"unreadable: {e}"))
                    continue
                hi = int(end) if end else start
                if not 1 <= start <= hi <= total:
                    problems.append((name, number, rid, m.group(0), f"line {start}-{hi} outside 1..{total}"))
    return checked, problems, absent


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--quiet", action="store_true", help="summary line only")
    args = parser.parse_args()
    checked, problems, absent = check()
    for name in absent:
        print(f"reference-citations: {name}'s vendored tree is absent (a fresh clone); skipped")
    if not problems:
        print(f"reference-citations: {checked} citations resolve against the vendored trees")
        return 0
    if args.quiet:
        print(f"reference-citations: {len(problems)} broken of {checked}")
        return 1
    print(f"reference-citations: {len(problems)} broken of {checked} citations:")
    for name, number, rid, cite, problem in problems:
        print(f"  {name}:{number} [{rid}] {cite}: {problem}")
    print("Existence and range only: a resolving line can still say the wrong thing.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
