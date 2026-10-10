"""Re-map every matrix row's citations from the row's own last-modified commit to HEAD.

**Why this exists.** `remap-citations.py` re-points a whole document from one pre-edit ref,
and its docstring is explicit: run it once per edit, only on documents holding the OLD
numbers, because mapping already-mapped numbers corrupts them. The parity matrices violate
that precondition by design — rows are written and re-pointed at different rounds, so one
document holds citations of many vintages. Rounds 100-103 and 105-109 inserted code above
anchels written earlier and the resolution-only checker stayed green; the fourth goal audit
measured ~100 of them, 168-248 lines off.

**Method.** `git blame --line-porcelain` gives each matrix line its last-modified commit —
which is exactly the commit whose numbering the row's citations are in, because a row is one
line and the round that wrote it verified its citations then. For each row, for each cited
file, difflib-aligns `file@row-commit` against `file@HEAD` (the same `line_map` the existing
tool uses, whole-line, autojunk off) and re-points the citations through that mapping. A row
whose commit predates the file, or whose citation the alignment cannot map, is left alone and
reported — never guessed at.

    python scripts/remap-by-vintage.py            # dry run: report, write nothing
    python scripts/remap-by-vintage.py --apply    # write the documents
"""

from __future__ import annotations

import re
import subprocess
import sys
import importlib.util
from pathlib import Path

_spec = importlib.util.spec_from_file_location(
    "remap_citations", Path(__file__).parent / "remap-citations.py"
)
_rc = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_rc)
CITATION, git_show, line_map = _rc.CITATION, _rc.git_show, _rc.line_map  # noqa: E402

MATRICES = ["docs/parity/pi-tui.md", "docs/parity/dsh-desktop.md"]


def basename_map(matrix: str) -> dict[str, str]:
    """`app.rs` -> the full `crates/...` path(s) with that basename in this matrix.

    The matrices abbreviate: a row can carry only `app.rs:5218`, continuing a
    full path from an earlier row of the same document. Resolve by basename --
    exactly one match, or leave it alone rather than guess.
    """
    full = re.findall(r"(crates/[\w./-]+\.rs)", Path(matrix).read_text(encoding="utf-8"))
    by_base: dict[str, set[str]] = {}
    for p in full:
        by_base.setdefault(Path(p).name, set()).add(p)
    return {k: next(iter(v)) for k, v in by_base.items() if len(v) == 1}


def row_vintages(matrix: str) -> dict[int, str]:
    """Matrix line number (1-based) -> the commit that last touched it."""
    out = subprocess.run(
        ["git", "blame", "--line-porcelain", matrix],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    vintages: dict[int, str] = {}
    lineno = 0
    for line in out.splitlines():
        m = re.match(r"^([0-9a-f]{40}) \d+ \d+", line)
        if m:
            lineno += 1
            vintages[lineno] = m.group(1)
    return vintages


def main() -> int:
    apply = "--apply" in sys.argv
    grand = 0
    for matrix in MATRICES:
        path = Path(matrix)
        text = path.read_text(encoding="utf-8")
        lines = text.split("\n")
        vintages = row_vintages(matrix)
        bases = basename_map(matrix)
        # One mapping per (file, vintage) pair, built once and reused.
        mappings: dict[tuple[str, str], dict[int, int]] = {}
        changed_rows = 0
        problems: list[str] = []
        for i, line in enumerate(lines, start=1):
            if not line.startswith("|") or not CITATION.search(line):
                continue
            commit = vintages.get(i)
            if not commit:
                continue

            def one(m: re.Match[str]) -> str:
                nonlocal grand
                path_s, start = m.group("path"), int(m.group("start"))
                # The matrices' own convention: a bare filename (`app.rs:2705`)
                # continues the last full path in the same line. Resolve it before
                # anything else, or the git lookup fails and the citation is left.
                if "/" not in path_s and path_s in bases:
                    path_s = bases[path_s]
                key = (path_s, commit)
                if key not in mappings:
                    try:
                        old = git_show(commit, path_s)
                    except subprocess.CalledProcessError:
                        mappings[key] = {}
                        return m.group(0)
                    new = Path(path_s).read_text(encoding="utf-8").splitlines()
                    mapping, _unaligned = line_map(old, new)
                    mappings[key] = mapping
                mapping = mappings[key]
                target = mapping.get(start)
                if target is None:
                    problems.append(f"{matrix}:{i} {path_s}:{start} (no anchor)")
                    return m.group(0)
                end = m.group("end")
                if end:
                    new_end = mapping.get(int(end))
                    if new_end is None:
                        problems.append(f"{matrix}:{i} {path_s}:{end} (no anchor)")
                        return m.group(0)
                    repl = f"{path_s}:{target}-{new_end}"
                else:
                    repl = f"{path_s}:{target}"
                if repl != m.group(0):
                    grand += 1
                return repl

            new_line = CITATION.sub(one, line)
            if new_line != line:
                changed_rows += 1
                lines[i - 1] = new_line
        print(f"{matrix}: {changed_rows} row(s) with citations to re-point")
        for p in problems[:12]:
            print(f"  UNMAPPED: {p}")
        if problems:
            print(f"  {len(problems)} citation(s) could not be mapped; leaving them")
        if apply:
            path.write_text("\n".join(lines), encoding="utf-8", newline="\n")
    print(f"total: {grand} citation(s) {'re-pointed' if apply else 'would be re-pointed'}")
    if not apply:
        print("dry run — nothing written; re-run with --apply")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
