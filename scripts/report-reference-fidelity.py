"""Report reference rows whose cited lines share no vocabulary with the behaviour claim.

**Why this exists.** The reference-citation leg proves every cited line *exists*; the
sixth goal audit's point was that existence is not fidelity — a row claiming "Enter
grants once" whose reference citation resolves to an unrelated line is still a row
built on nothing. The 1(g)/1(h) audit was a four-row sample; this turns the sample
into a measurement: every row is scored, the weakest are the candidates a reviewer
reads first, and the score is reported rather than hidden.

**Method.** For each row, the behaviour cell's distinctive words (length >= 5, not
stop-words, not the matrix's own vocabulary) are checked against the window of
lines around each reference citation. A row scores 0 when no distinctive word
appears near any cited line. Zero is **not** proof of a wrong citation — a citation
can be right in a vocabulary the claim does not share — so this reports candidates
and says so. It is a reviewer's instrument, not a gate.

    python scripts/report-reference-fidelity.py            # all candidates
    python scripts/report-reference-fidelity.py --verbose # every row's score
"""

from __future__ import annotations

import argparse
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

REFERENCE_ROOTS = {
    "pi-tui.md": os.path.join(ROOT, "references", "pi-1.3.0"),
    "dsh-desktop.md": os.path.join(ROOT, "references", "deepseek-harness-dsh-v0.2.1-rc.2"),
}

CITE = re.compile(r"`([\w./-]+?\.(?:ts|tsx|rs|json|md)):(\d+)(?:-(\d+))?`")
ROW_ID = re.compile(r"^[A-Z]+-\d+$")
SENTINEL = "\x00"
WINDOW = 6

STOP = {
    "the", "and", "with", "that", "this", "from", "into", "when", "while", "which",
    "there", "their", "they", "have", "has", "not", "are", "was", "were", "been",
    "its", "it", "for", "any", "all", "one", "two", "per", "via", "then", "than",
    "only", "also", "such", "same", "each", "every", "before", "after", "during",
    "does", "doing", "done", "shows", "show", "shown", "using", "used", "uses",
    "window", "panel", "button", "state", "states", "reference", "behaviour",
    "behavior", "claim", "row", "rows", "matrix", "parity", "missing", "partial",
    "deliberately", "different", "click", "press", "pressed", "keyboard", "footer",
}


def keywords(text: str) -> list[str]:
    words = re.findall(r"[a-z][a-z-]{4,}", text.lower())
    out = []
    for w in words:
        if w not in STOP and "-" not in w and w not in out:
            out.append(w)
    return out


def resolve(root: str, rel: str) -> str | None:
    direct = os.path.join(root, *rel.split("/"))
    if os.path.exists(direct):
        return direct
    nested = os.path.join(root, "packages", "client", *rel.split("/"))
    if os.path.exists(nested):
        return nested
    return None


def data_rows(path: str):
    with open(path, encoding="utf-8") as f:
        lines = f.read().splitlines()
    for number, line in enumerate(lines, 1):
        if not line.startswith("|"):
            continue
        cells = [
            x.strip().replace(SENTINEL, "|")
            for x in line.replace("\\|", SENTINEL).strip().strip("|").split("|")
        ]
        if len(cells) < 6 or not ROW_ID.match(cells[0]):
            continue
        yield number, cells[0], cells[1], cells[2]


def emit(text: str) -> None:
    """Print, surviving a console whose code page lacks the matrix's glyphs.

    The matrices carry arrows and ellipses; a legacy console's cp1256 encoder
    refuses them and the report dies before printing. Replacing is the honest
    degradation for a report, and the full text is in the matrices anyway.
    """
    try:
        print(text)
    except UnicodeEncodeError:
        print(text.encode("ascii", "replace").decode("ascii"))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()
    candidates = []
    total_rows = 0
    scored = 0
    for name, root in REFERENCE_ROOTS.items():
        matrix = os.path.join(ROOT, "docs", "parity", name)
        if not os.path.isfile(matrix) or not os.path.isdir(root):
            print(f"{name}: matrix or vendored tree absent; skipped")
            continue
        for number, rid, behaviour, reference in data_rows(matrix):
            total_rows += 1
            words = keywords(behaviour)
            if not words:
                continue
            hits: set[str] = set()
            for m in CITE.finditer(reference):
                target = resolve(root, m.group(1))
                if not target:
                    continue
                start = int(m.group(2))
                try:
                    with open(target, encoding="utf-8", errors="replace") as f:
                        flines = f.read().splitlines()
                except OSError:
                    continue
                lo = max(0, start - 1 - WINDOW)
                hi = min(len(flines), start + WINDOW)
                near = "\n".join(flines[lo:hi]).lower()
                for w in words:
                    if w in near:
                        hits.add(w)
            scored += 1
            if args.verbose:
                emit(f"{name}:{number} [{rid}] {len(hits)}/{len(words)} {sorted(hits)[:6]}")
            if not hits:
                candidates.append((name, number, rid, behaviour[:110]))
    print(
        f"rows: {total_rows}; scored: {scored}; candidates (no shared vocabulary): "
        f"{len(candidates)}"
    )
    for name, number, rid, behaviour in candidates:
        emit(f"  {name}:{number} [{rid}] {behaviour}")
    print("A candidate is a reviewer's queue, not a defect list: a citation can be")
    print("right in a vocabulary its claim does not share.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
