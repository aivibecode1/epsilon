"""Fail the build on a verb-count claim that drifted from SUPPORTED.

Why this is a gate and not care
-------------------------------
The window's verb count is stated in prose in four places
(`docs/parity/dsh-desktop.md`, the `SUPPORTED` doc comment,
`docs/plans/025-completion-plan.md`), and every verb round updates them by
hand. Four rounds running each missed at least one site (a paragraph still
said 20/17 three rounds later), and no other gate reads prose numbers. This
is the check that means nobody has to.

Scope and deliberate exclusions
-------------------------------
* Only CURRENT claims are checked. Dated records — the ADR amendments
  ("N of 37" per round) and the STATUS phase log — describe the tree on the
  day they were written and must never change, so this script does not read
  those files at all.
* Only the exact claim shapes below. A new sentence with a new shape is the
  author's job to add here, and the script says so when it finds a number it
  does not recognise near the word "heads".
* The guard test (`the_window_runs_exactly_the_heads_it_declares_supported`)
  already holds SUPPORTED against the arms; this holds the prose against
  SUPPORTED. Neither covers the other.

    python scripts/check-verb-counts.py            # report, exit 1 on drift
    python scripts/check-verb-counts.py --quiet   # summary line only
"""

from __future__ import annotations

import argparse
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

WORDS = {
    "twenty": 20,
    "twenty-one": 21,
    "twenty-two": 22,
    "twenty-three": 23,
    "twenty-four": 24,
    "twenty-five": 25,
    "twenty-six": 26,
    "twenty-seven": 27,
    "twenty-eight": 28,
    "twenty-nine": 29,
    "thirty": 30,
    "thirty-one": 31,
    "thirty-two": 32,
    "thirty-three": 33,
    "thirty-four": 34,
    "thirty-five": 35,
    "thirty-six": 36,
    "thirty-seven": 37,
    "thirty-eight": 38,
}


def read(rel: str) -> str:
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        return f.read()


def parse_array(source: str, name: str) -> list[str]:
    """One quoted string per line between `pub const NAME` and the closing."""
    start = source.index(f"pub const {name}")
    end = source.index("];", start)
    return re.findall(r'"([a-z0-9_-]+)"', source[start:end])


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args()

    problems: list[str] = []

    contracts = read("crates/epsilon-contracts/src/command.rs")
    heads = parse_array(contracts, "COMMAND_HEADS")
    total = len(heads)

    desktop = read("crates/epsilon-desktop/src/commands.rs")
    supported = parse_array(desktop, "SUPPORTED")
    have = len(supported)

    def check(cond: bool, message: str) -> None:
        if not cond:
            problems.append(message)

    parity = read("docs/parity/dsh-desktop.md")
    for m in re.finditer(r"answers (\d+) of the (\d+) heads", parity):
        check(
            int(m.group(1)) == have and int(m.group(2)) == total,
            f"dsh-desktop.md says {m.group(1)} of {m.group(2)} heads; "
            f"SUPPORTED holds {have} of {total}",
        )
    for m in re.finditer(r"refuses the other (\d+) by name", parity):
        check(
            int(m.group(1)) == total - have,
            f"dsh-desktop.md says {m.group(1)} refused; {total} - {have} = "
            f"{total - have}",
        )
    for m in re.finditer(r"over ((?:twenty|thirty)[a-z-]*) verbs", parity):
        check(
            WORDS.get(m.group(1)) == have,
            f"dsh-desktop.md says '{m.group(1)} verbs'; SUPPORTED holds {have}",
        )
    for m in re.finditer(r"is one of the ((?:twenty|thirty)[a-z-]*) heads", parity):
        check(
            WORDS.get(m.group(1)) == have,
            f"dsh-desktop.md says 'one of the {m.group(1)}'; SUPPORTED holds {have}",
        )
    for m in re.finditer(r"SUPPORTED.* is ((?:twenty|thirty)[a-z-]*) names", desktop):
        check(
            WORDS.get(m.group(1)) == have,
            f"commands.rs doc says '{m.group(1)} names'; SUPPORTED holds {have}",
        )

    plan = read("docs/plans/025-completion-plan.md")
    for m in re.finditer(r"answers (\d+) of (\d+) commands", plan):
        check(
            int(m.group(1)) == have and int(m.group(2)) == total,
            f"025-completion-plan.md says {m.group(1)} of {m.group(2)}; "
            f"SUPPORTED holds {have} of {total}",
        )

    if problems and not args.quiet:
        print("verb-count drift:")
        for p in problems:
            print(f"  - {p}")
    if args.quiet:
        print(
            f"verb-counts: SUPPORTED holds {have} of {total}; "
            f"{len(problems)} problem(s)"
        )
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
