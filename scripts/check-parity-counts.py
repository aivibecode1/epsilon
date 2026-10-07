"""Fail the build on a parity-matrix count that drifted from its rows.

Why this is a gate and not care
-------------------------------
Both matrices (`docs/parity/pi-tui.md`, `docs/parity/dsh-desktop.md`) carry
summary tables, grand totals, and open-row lists that are updated by hand
every parity round -- and section headings that embed counts
(`### The missing rows -- 161`, still saying 161 three rounds after it
stopped being true). No other gate reads these numbers. This is the check
that means nobody has to.

What it verifies, per file
---------------------------
* Every section summary row adds up, the sections add up to the grand
  total, and the grand total matches the rows (`done` / `partial` /
  `missing` / `deliberately different`).
* The open-row lists name exactly the rows with that status -- no stale
  ids left behind (S-103/S-107 sat in a missing list two rounds after
  going `done`), no missing ones omitted, and every parenthesised count
  matches the ids beside it.
* The embedded heading counts (`### The missing rows -- N`) match.
* No row id appears twice (a duplicate would double-count silently).

Scope and deliberate exclusions
-------------------------------
* Per-section placement is verified for pi-tui (its `## N. Name` headers
  match its summary rows verbatim) but NOT for dsh-desktop, whose `##`
  headers, summary areas, and list areas use three different namings for
  the same sections. A hand-map would be a fourth list to keep in step;
  the script verifies that file's arithmetic, totals, and list unions
  instead, and says so.
* Dated records (ADR amendments, the STATUS log) are not read -- see
  `check-verb-counts.py`.
* Meaning is a reviewer's job: a row with the wrong status but consistent
  counts still passes, and stale prose beside a correct number does too.

    python scripts/check-parity-counts.py            # report, exit 1 on drift
    python scripts/check-parity-counts.py --quiet   # summary line only
"""

from __future__ import annotations

import argparse
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
STATUSES = ("done", "partial", "missing", "deliberately different")
ID_RE = re.compile(r"^[A-Z]+-\d+$")


def read(rel: str) -> list[str]:
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        return f.read().splitlines()


def cells_of(line: str) -> list[str]:
    return [c.strip() for c in line.strip().strip("|").split("|")]


def is_data_row(line: str) -> re.Match[str] | None:
    if not line.startswith("|"):
        return None
    cells = cells_of(line.replace("\\|", "\x00"))
    if len(cells) != 6 or not ID_RE.match(cells[0]):
        return None
    if cells[3].lower() not in STATUSES:
        return None
    return cells


def check_file(rel: str, problems: list[str], map_sections: bool) -> dict[str, int]:
    lines = read(rel)
    by_id: dict[str, str] = {}
    per_section: dict[str, dict[str, int]] = {}
    current: str | None = None
    for line in lines:
        if line.startswith("## "):
            current = line[3:].strip()
            continue
        row = is_data_row(line)
        if row is None:
            continue
        rid, status = row[0], row[3].lower()
        if rid in by_id:
            problems.append(f"{rel}: {rid} appears twice")
        by_id[rid] = status
        key = current or "<no section>"
        slot = per_section.setdefault(key, {s: 0 for s in STATUSES})
        slot[status] += 1

    totals = {s: 0 for s in STATUSES}
    for status in by_id.values():
        totals[status] += 1

    # Section summaries: `| Name | total | done | partial | missing | dd |`.
    # They appear twice per file; each copy must hold.
    seen_summaries = 0
    for line in lines:
        if not line.startswith("|"):
            continue
        cells = cells_of(line)
        if len(cells) != 6 or cells[0].startswith("**"):
            continue
        try:
            nums = [int(c) for c in cells[1:]]
        except ValueError:
            continue
        if len(nums) != 5:
            continue
        name = cells[0]
        seen_summaries += 1
        t, d, p, m, dd = nums
        if t != d + p + m + dd:
            problems.append(f"{rel}: section '{name}' adds to {d + p + m + dd}, not {t}")
        if map_sections:
            if name not in per_section:
                problems.append(f"{rel}: summary '{name}' matches no ## section")
                continue
            actual = per_section[name]
            got = (t, d, p, m, dd)
            want = (
                sum(actual.values()),
                actual["done"],
                actual["partial"],
                actual["missing"],
                actual["deliberately different"],
            )
            if got != want:
                problems.append(
                    f"{rel}: section '{name}' says {got}, rows say {want}"
                )

    # Grand total row.
    for line in lines:
        if not line.startswith("|"):
            continue
        cells = cells_of(line)
        if len(cells) != 6 or cells[0] != "**Total**":
            continue
        try:
            nums = [int(c.strip("* ") or "0") for c in cells[1:]]
        except ValueError:
            continue
        grand = sum(totals.values())
        want_total = (grand, totals["done"], totals["partial"], totals["missing"],
                      totals["deliberately different"])
        if tuple(nums) != want_total:
            problems.append(f"{rel}: Total row says {tuple(nums)}, rows say {want_total}")

    # Open-row lists and embedded heading counts.
    def lists_under(heading: str) -> list[tuple[str, list[str]]]:
        out: list[tuple[str, list[str]]] = []
        inside = False
        for line in lines:
            if line.startswith("### "):
                inside = heading in line[4:].strip().lower()
                continue
            if not inside or not line.startswith("|"):
                continue
            cells = cells_of(line)
            if len(cells) < 2 or cells[0].lower() in ("area",):
                continue
            ids = [t.strip().strip(",") for t in cells[1].split() if t.strip().strip(",")]
            ids = [i for i in ids if ID_RE.match(i)]
            if ids:
                out.append((cells[0], ids))
        return out

    for status, heading in (("missing", "missing"), ("partial", "partial")):
        listed: list[str] = []
        for area, ids in lists_under(heading):
            m = re.search(r"\((\d+)\)\s*$", area)
            if m and int(m.group(1)) != len(ids):
                problems.append(
                    f"{rel}: list '{area}' counts {m.group(1)} but names {len(ids)}"
                )
            listed.extend(ids)
        want = sorted(i for i, s in by_id.items() if s == status)
        if sorted(listed) != want:
            missing = sorted(set(want) - set(listed))
            extra = sorted(set(listed) - set(want))
            detail = []
            if missing:
                detail.append(f"unlisted: {missing}")
            if extra:
                detail.append(f"stale: {extra}")
            problems.append(f"{rel}: {status} lists differ: {'; '.join(detail)}")

    for line in lines:
        m = re.match(r"### The `(missing|partial)` rows -- (\d+)\s*$", line)
        if m:
            want = sum(1 for s in by_id.values() if s == m.group(1))
            if int(m.group(2)) != want:
                problems.append(
                    f"{rel}: heading says {m.group(1)} -- {m.group(2)}, rows say {want}"
                )

    return totals


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args()

    problems: list[str] = []
    pi = check_file("docs/parity/pi-tui.md", problems, map_sections=True)
    dsh = check_file("docs/parity/dsh-desktop.md", problems, map_sections=False)

    if problems and not args.quiet:
        print("parity-count drift:")
        for p in problems:
            print(f"  - {p}")
    if args.quiet:
        pi_n, dsh_n = sum(pi.values()), sum(dsh.values())
        print(
            f"parity-counts: pi-tui {pi_n} rows, dsh-desktop {dsh_n} rows; "
            f"{len(problems)} problem(s)"
        )
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
