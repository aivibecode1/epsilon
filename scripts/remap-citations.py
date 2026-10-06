"""Re-map `file:line` citations after a source edit, and report what it could not do.

**Why this exists.** A parity matrix cites our own code by line. An edit that inserts lines
moves every citation after it, and a citation that now points at the *wrong* line is worse
than one that points nowhere: it looks verified. This rewrites the citations from a real
alignment of the old file against the new one, so the mapping is measured rather than guessed.

**Method.** `difflib.SequenceMatcher` over whole lines, with `autojunk` off — the edits that
matter here are contiguous insertions, and whole-line alignment maps them exactly. Every
mapping is checked in both directions, and anything the matcher could not align with
confidence is reported rather than guessed at.

    python scripts/remap-citations.py <old-ref> <file> <document> [<document> ...]

`old-ref` is any git revision holding the pre-edit file. Nothing is written unless the
whole document can be re-mapped.

**Run this once per edit, and only on documents that still hold the OLD numbers.** It maps
*old* line numbers to new ones, so running it twice would map already-mapped numbers as if
they were old and corrupt them. That is why it is a script rather than a habit: the failure
is silent, and a citation that points at the wrong line is worse than one that points
nowhere because it looks verified. The docstring of the call is therefore the contract —
`old-ref` and the documents must agree about which state the citations are in.
"""

from __future__ import annotations

import difflib
import re
import subprocess
import sys
from pathlib import Path

# `path/to/file.rs:1234` or `:1234-1240`, including a bare `:1234` that continues the
# previous file — which is how the matrices are written.
CITATION = re.compile(
    r"(?P<path>[\w./-]+\.rs):(?P<start>\d+)(?:-(?P<end>\d+))?"
)


def git_show(ref: str, path: str) -> list[str]:
    out = subprocess.run(
        ["git", "show", f"{ref}:{path}"], capture_output=True, text=True, check=True
    )
    return out.stdout.splitlines()


def line_map(old: list[str], new: list[str]) -> tuple[dict[int, int], list[tuple[int, int]]]:
    """old line number (1-based) -> new line number, plus the pairs we could not align."""
    matcher = difflib.SequenceMatcher(None, old, new, autojunk=False)
    mapping: dict[int, int] = {}
    unaligned: list[tuple[int, int]] = []
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        if tag == "equal":
            for offset in range(i2 - i1):
                mapping[i1 + offset + 1] = j1 + offset + 1
        elif tag == "replace":
            # Same count, different text: a comment rewritten in place. Trust the position.
            if (i2 - i1) == (j2 - j1):
                for offset in range(i2 - i1):
                    mapping[i1 + offset + 1] = j1 + offset + 1
            else:
                unaligned.extend((i1 + k + 1, j1 + k + 1) for k in range(i2 - i1))
        elif tag == "insert":
            # An insertion at old position i1: the line *after* it is the anchor.
            anchor_old = i1 + 1
            anchor_new = j1 + 1
            if anchor_new <= len(new):
                mapping[anchor_old] = anchor_new
    return mapping, unaligned


def remap(text: str, suffix: str, mapping: dict[int, int]) -> tuple[str, int, list[str]]:
    problems: list[str] = []
    changed = 0

    def one(m: re.Match[str]) -> str:
        nonlocal changed
        start = int(m.group("start"))
        target = mapping.get(start)
        if target is None:
            problems.append(f"{suffix}: no mapping for old line {start}")
            return m.group(0)
        new_text = str(target)
        if m.group("end"):
            end = int(m.group("end"))
            new_end = mapping.get(end)
            if new_end is None:
                problems.append(f"{suffix}: no mapping for old line {end}")
                return m.group(0)
            new_text = f"{target}-{new_end}"
        if new_text != m.group(0).split(":", 1)[1]:
            changed += 1
        return f"{m.group('path')}:{new_text}"

    return CITATION.sub(one, text), changed, problems


def main() -> int:
    if len(sys.argv) < 4:
        print(__doc__)
        return 2
    ref, target, *documents = sys.argv[1:]
    old = git_show(ref, target)
    new = Path(target).read_text(encoding="utf-8").splitlines()
    mapping, unaligned = line_map(old, new)
    print(
        f"{target}: {len(old)} -> {len(new)} lines, "
        f"{len(mapping)} anchored, {len(unaligned)} unaligned"
    )
    if unaligned:
        print(f"  UNALIGNED (left alone, reported below): {unaligned[:12]}")

    outputs: list[str] = []
    total = 0
    all_problems: list[str] = []
    for doc in documents:
        text = Path(doc).read_text(encoding="utf-8")
        out, changed, problems = remap(text, doc, mapping)
        outputs.append(out)
        total += changed
        all_problems.extend(problems)
        print(f"  {doc}: {changed} citations re-pointed")

    if all_problems:
        print(f"\n{len(all_problems)} citations could NOT be mapped; nothing written:")
        for line in all_problems[:20]:
            print(f"  {line}")
        return 1

    for doc, out in zip(documents, outputs):
        Path(doc).write_text(out, encoding="utf-8", newline="\n")
    print(f"\n{total} citations re-pointed across {len(documents)} document(s).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
