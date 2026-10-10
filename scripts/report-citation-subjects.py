"""Report rows whose cited lines share no vocabulary with the behaviour claim: a reviewer's queue, not a gate.

**Why this exists, and why it is a report.** The ninth goal audit found two rows whose
citations pointed at unrelated production code (P-079 cited the Resume/New arms for the
Import prompt; P-038 cited the F2 chords for the Ctrl+P sentence) — wrong when written, so
invisible to the blame-vintage remap, and pointing at production lines, so invisible to the
placement leg. Both were found by a human reading the row against the code, and both are
fixed. This instrument surfaces that class for the next reviewer: it flags citations whose row
names a subject that exists somewhere in the target file yet not within `WINDOW` lines of the
citation.

**Why it must not be a gate.** It is a vocabulary heuristic and it is noisy by construction:
a citation can be right in words its claim does not share. Measured on the tree after those
fixes: **218 candidates across 181 rows, the overwhelming majority prose words**
("prints", "always", "becomes") — a gate leg would fail the build on a false positive, which is
worse than the blind spot. It is also blind to struct-field symbols (`pending_import`): it
sees only backticked names that appear as definitions in the target file, which is why the
auditor found P-079 by reading where this only ranks it among its noisy queue. The honest
claim is a queue to read, not a proof of anything — the same role
`report-reference-fidelity.py` holds.

    python scripts/report-citation-subjects.py            # the queue, by count
    python scripts/report-citation-subjects.py --verbose  # list them, with the subject
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


def emit(text: str) -> None:
    """Print, surviving a console whose code page lacks the matrix's glyphs."""
    try:
        print(text)
    except UnicodeEncodeError:
        print(text.encode("ascii", "replace").decode("ascii"))


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
    # Exit 0 by design: this is a report, not a control. A queue whose majority is
    # false positives must not read as a failing gate -- that was the ninth audit's
    # objection to its predecessor, which exited 1 under the `check-` name while no
    # gate and no document owned its number.
    emit(
        f"citations: {total}; measurable: {checked}; drifted: {len(drifted)} "
        f"across {len({d[1] for d in drifted})} rows (report only — exit 0)"
    )
    if verbose:
        for matrix, rid, path, start, subject in drifted:
            emit(f"  {matrix}:{rid} {path}:{start} (subject `{subject}` elsewhere)")
    emit("A candidate is a reviewer's queue, not a defect list: a citation can be")
    emit("right in a vocabulary its claim does not share.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
