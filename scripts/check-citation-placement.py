"""Flag citations that land inside a crate's test module — the class the vintage remap cannot see.

**Why this exists.** `check-parity-citations.py` proves a citation resolves to a file and an
in-range line; `remap-by-vintage.py` faithfully re-points a citation from the row's own
last-modified commit. Neither can see a citation that was **wrong when written**: a `done`
row citing `app.rs:9362-9371` for the permission gate, where 9362 is
`full.textures_delta.clear();` inside the crate's `#[cfg(test)]` harness — the vintage remap
maps the old wrong anchor to the new wrong anchor, and every checker stays green. The fifth
goal audit found four such rows in the desktop approval cluster (D-071, D-072, S-120, I-103),
wrong at their own m313 vintage.

**Method.** For each citation into a file under `crates/`, find the last `#[cfg(test)]` at or
before the cited line. If the citation sits inside a test module, it is a *harness line* — a
`done` row's `ours` column describes production behaviour, so the citation is wrong by
construction, whatever it now points at. The test boundary is computed per file, not
hard-coded.

    python scripts/check-citation-placement.py            # exit 1 if any harness citation
    python scripts/check-citation-placement.py --verbose  # list them
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

MATRICES = ["docs/parity/pi-tui.md", "docs/parity/dsh-desktop.md"]
CITATION = re.compile(r"((?:crates/)?[\w./-]+\.rs):(\d+)(?:-(\d+))?")
# The inline test module: `#[cfg(test)]` immediately followed by `mod tests {`.
# A `#[cfg(test)] mod name;` declaration is a submodule whose content lives in
# another file — it opens no inline region, so it is not a boundary.
TEST_MOD = re.compile(r"^\s*#\[cfg\(test\)\]\s*$")
MOD_TESTS = re.compile(r"^\s*mod tests \{")


def basename_map(matrix: str) -> dict[str, str]:
    """`app.rs` -> the full `crates/...` path with that basename **in this matrix**.

    Per matrix, because each matrix's ours column names one front end's code:
    the DSH matrix's paths are all `epsilon-desktop`, the Pi matrix's all
    `epsilon-tui`/`epsilon-cli`. A map built across both would hold two
    `app.rs` entries and refuse both; built per document, each basename is
    unique and a bare continuation resolves. One match, or leave it alone.
    """
    by_base: dict[str, set[str]] = {}
    full = re.findall(r"(crates/[\w./-]+\.rs)", Path(matrix).read_text(encoding="utf-8"))
    for p in full:
        by_base.setdefault(Path(p).name, set()).add(p)
    return {k: next(iter(v)) for k, v in by_base.items() if len(v) == 1}


def test_module_start(lines: list[str], cited: int) -> int | None:
    """The line range of the inline `mod tests` containing `cited`, if any.

    The last `#[cfg(test)]` + `mod tests {` pair at or before the cited line
    opens the module; brace depth tracks where it closes — a module that ends
    before the citation is not the one containing it.
    """
    boundary = None
    end = None
    pending_attr = False
    depth = 0
    for i, line in enumerate(lines, start=1):
        if TEST_MOD.match(line):
            pending_attr = True
            continue
        if pending_attr:
            pending_attr = False
            if MOD_TESTS.match(line):
                boundary = i
                depth = line.count("{") - line.count("}")
                end = None
                continue
        if boundary is not None and end is None:
            depth += line.count("{") - line.count("}")
            if depth <= 0:
                end = i
        if i > cited:
            break
    if boundary is None or (end is not None and cited > end):
        return None
    return boundary


def resolve(path: str, line_text: str, bases: dict[str, str]) -> str:
    """Resolve a citation's path to a repo-relative `crates/...` path.

    Three shapes appear in the matrices: the full `crates/...` form; the
    committed-but-not-crates form (`epsilon-contracts/src/keybind.rs`); and
    the bare filename (`app.rs:2705`) that continues an earlier full path.
    The basename map (built from the full paths in the same document)
    disambiguates the last; one match or leave it alone.
    """
    if path.startswith("crates/"):
        return path
    if "/" in path:
        return "crates/" + path
    base = Path(path).name
    resolved = bases.get(base)
    return resolved if resolved else path


def main() -> int:
    verbose = "--verbose" in sys.argv
    harness: list[tuple[str, str, str, int]] = []
    total = 0
    for matrix in MATRICES:
        bases = basename_map(matrix)
        text = Path(matrix).read_text(encoding="utf-8")
        for line in text.splitlines():
            if not line.startswith("|"):
                continue
            cells = [c.strip() for c in line.replace("\\|", "\x00").split("|")]
            if len(cells) < 7 or not re.fullmatch(r"[A-Z]+-\d+", cells[1]):
                continue
            rid = cells[1]
            for m in CITATION.finditer(cells[5]):
                path = resolve(m.group(1), cells[5], bases)
                if not path.startswith("crates/"):
                    continue
                total += 1
                start = int(m.group(2))
                try:
                    lines = Path(path).read_text(encoding="utf-8").splitlines()
                except OSError:
                    continue
                boundary = test_module_start(lines, start)
                if boundary is not None:
                    harness.append((matrix, rid, path, start))
    print(f"crates citations checked: {total}; inside a test module: {len(harness)}")
    if verbose:
        for matrix, rid, path, start in harness:
            print(f"  {matrix}:{rid} {path}:{start}")
    return 1 if harness else 0


if __name__ == "__main__":
    raise SystemExit(main())
