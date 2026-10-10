"""Measure citation drift: does each matrix citation's target line carry the row's subject?

**Why this exists.** `check-parity-citations.py` proves a citation resolves to a file and an
in-range line. It does not — and its own docstring says it cannot — prove the line says what
the row claims. A citation that points at the wrong line is worse than one that points
nowhere, because it looks verified. The fourth goal audit found ~100 of them: rounds 100-103
and 105-109 inserted code above anchors written earlier, and the checker stayed green.

**Method.** For every row, the `ours` cell is parsed for citations
(`crates/...:line` / `:start-end`) and for *subjects* — backticked identifiers and
snake_case/CamelCase tokens that look like code symbols. A citation is **drifted** when some
subject of its row *exists somewhere in the target file* yet appears nowhere within
`WINDOW` lines of the citation. A citation whose row names no symbol the file contains is
left alone: it points at a prose region and is not measurable this way — those are the
checker's honest blind spot, stated rather than hidden.

    python scripts/check-citation-subjects.py            # exit 1 if any drifted
    python scripts/check-citation-subjects.py --verbose  # list them, with the subject
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

MATRICES = ["docs/parity/pi-tui.md", "docs/parity/dsh-desktop.md"]
CITATION = re.compile(r"(?P<path>[\w./-]+\.rs):(?P<start>\d+)(?:-(?P<end>\d+))?")
SYMBOL = re.compile(r"`([a-z_][a-z0-9_]{3,}|[A-Z][A-Za-z0-9]{3,})`")
WORD = re.compile(r"\b([a-z_][a-z0-9_]{5,}|[A-Z][A-Za-z0-9]{5,})\b")
WINDOW = 8
NOISE = {
    "crates",
    "epsilon",
    "app.rs",
    "keys.rs",
    "state.rs",
    "session.rs",
    "card.rs",
    "view.rs",
    "theme.rs",
    "tests",
    "window",
    "partial",
    "missing",
    "deliberately",
    "different",
    "reference",
}


def subjects_of(cell: str) -> list[str]:
    """Identifiers the row's ours-cell names, backticked first (strongest signal)."""
    found = [m.group(1) for m in SYMBOL.finditer(cell)]
    found += [m.group(1) for m in WORD.finditer(cell)]
    seen: list[str] = []
    for s in found:
        if s not in seen and s not in NOISE and not s.startswith(("crate", "docs", "audit")):
            seen.append(s)
    return seen


def main() -> int:
    verbose = "--verbose" in sys.argv
    drifted: list[tuple[str, str, str, int, str]] = []
    total = 0
    checked = 0
    for matrix in MATRICES:
        text = Path(matrix).read_text(encoding="utf-8")
        for line in text.splitlines():
            if not line.startswith("|"):
                continue
            cells = [c.strip() for c in line.replace("\\|", "\x00").split("|")]
            if len(cells) < 7 or not re.fullmatch(r"[A-Z]+-\d+", cells[1]):
                continue
            rid, ours = cells[1], cells[5]
            citations = list(CITATION.finditer(ours))
            if not citations:
                continue
            subjects = subjects_of(ours)
            if not subjects:
                continue
            total += len(citations)
            for m in citations:
                path = m.group("path")
                start = int(m.group("start"))
                try:
                    lines = Path(path).read_text(encoding="utf-8").splitlines()
                except OSError:
                    continue
                whole = "\n".join(lines)
                in_file = [s for s in subjects if s in whole]
                if not in_file:
                    # The row names nothing this file contains: a prose citation, not
                    # measurable drift. The checker's stated blind spot.
                    continue
                checked += 1
                lo = max(0, start - 1 - WINDOW)
                hi = min(len(lines), start + WINDOW)
                near = "\n".join(lines[lo:hi])
                if not any(s in near for s in in_file):
                    drifted.append((matrix, rid, path, start, in_file[0]))
    print(
        f"citations: {total}; measurable: {checked}; drifted: {len(drifted)} "
        f"across {len({d[1] for d in drifted})} rows"
    )
    if verbose:
        for matrix, rid, path, start, subject in drifted:
            print(f"  {matrix}:{rid} {path}:{start} (subject `{subject}` elsewhere)")
    return 1 if drifted else 0


if __name__ == "__main__":
    raise SystemExit(main())
