#!/usr/bin/env python3
"""No tracked text file may carry a C0 control byte (ledger A49).

## Why this exists

Fourteen bytes across ten files had a **letter replaced by the C0 control character of
the same C escape name**: `e` → `0x1B` (`\\e`), `a` → `0x07` (`\\a`), `b` → `0x08`
(`\\b`). So `epsilon-tui` in a source file was `\\x1b` + `psilon-tui`, and
`audit/03-defects.md` opened its evidence path with a bell character. Every gate was
green, because none of them can see it:

- `cargo fmt` reprints text and does not compare whitespace inside comments;
- `clippy` lints code, and a control byte inside a `///` line is not code;
- `check-line-endings` looks for `\\r\\n`, and these are lone bytes in the middle of a
  line;
- the 1,300 test functions assert on values, not on the bytes of a doc comment.

Two of the ten were in source: `epsilon-runtime/src/text_safety.rs` — the module whose
entire job is stripping exactly these characters — and the crate roots of
`epsilon-cli`, `epsilon-core` and `epsilon-runtime`. That is the shape worth
remembering: **the corruption lands where nobody is looking, and the file that exists
to reject the byte is one of the files carrying it.**

The likely cause is a tool that wrote the C escape names through a layer that
interpreted them. Nothing in this repository needs the escape, so the check is on the
byte, not on the history.

## What it checks

Every tracked non-binary file, for a byte in `0x00..0x08`, `0x0B`, `0x0C`, `0x0E..0x1F`
or `0x7F`. Tab, newline and carriage return are layout and are allowed — the same
allowance `epsilon_runtime::text_safety` makes, deliberately, so this check and the
product's own rule cannot drift.

## What it does not do

- It does not rewrite anything. A file is fixed where it was edited, not here.
- It does not look at `audit/history/`, which is superseded and deliberately
  unrepaired, nor `audit/evidence/`, which `gate.ps1` regenerates on every run.
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def _load(name: str, filename: str):
    """Reuse the sibling check's file-selection policy rather than restating it.

    `check-line-endings.py` already answers "which tracked files are text, and which
    directories are not this repository's to police". A second list answering the same
    question is exactly the duplication ledger A12 records as this repository's own
    D-class failure, so it is imported rather than copied.
    """
    spec = importlib.util.spec_from_file_location(name, REPO / "scripts" / filename)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


_line_endings = _load("check_line_endings", "check-line-endings.py")

# Tab, newline and carriage return are layout. Everything else below 0x20, and DEL,
# is either an escape a terminal might honour or a byte that renders as nothing.
# This is `epsilon_runtime::text_safety`'s own list, for the reason in its module doc.
_ALLOWED = {0x09, 0x0A, 0x0D}


def offenders(files: list[str], binary: set[str]) -> list[tuple[str, int, int, str]]:
    """`(path, line, byte, context)` for each control byte found.

    The context is what makes this check worth having. "line 3 has 0x1B" costs a hunt;
    the line rendered with the control characters replaced by a visible marker reads
    `psi-lon-tui` and is obvious immediately — which is the only reason the corruption
    was findable at all.
    """
    found: list[tuple[str, int, int, str]] = []
    for rel in files:
        if any(rel == d or rel.startswith(d + "/") for d in _line_endings.SKIP_DIRS):
            continue
        if Path(rel).suffix.lower() in binary:
            continue
        try:
            data = (REPO / rel).read_bytes()
        except OSError:
            continue
        if not any(b < 0x20 and b not in _ALLOWED or b == 0x7F for b in data):
            continue
        for index, byte in enumerate(data):
            if (byte < 0x20 and byte not in _ALLOWED) or byte == 0x7F:
                line = data[:index].count(b"\n") + 1
                start = data.rfind(b"\n", 0, index) + 1
                end = data.find(b"\n", index)
                end = len(data) if end < 0 else end
                raw = data[start:end].decode("utf-8", "replace")
                shown = "".join(
                    c if c.isprintable() and c != "\x7f" else f"<0x{ord(c):02X}>" for c in raw
                )
                found.append((rel, line, byte, shown.strip()))
    return found


def main(argv: list[str]) -> int:
    quiet = "--quiet" in argv
    binary = _line_endings.binary_suffixes()
    files = _line_endings.tracked_files()
    bad = offenders(files, binary)

    if bad:
        print(f"check-control-bytes: {len(bad)} control byte(s) in tracked text\n")
        for rel, line, byte, context in bad[:40]:
            print(f"  {rel}:{line}  byte 0x{byte:02X}  |{context}|")
        if len(bad) > 40:
            print(f"  ... and {len(bad) - 40} more")
        print(
            "\nA C0 control byte inside a tracked text file is a corrupted character,"
            "\nnot content: `e` -> 0x1B (\\e), `a` -> 0x07 (\\a) and `b` -> 0x08 (\\b) is"
            "\nthe shape this repository has actually shipped (14 bytes, 10 files,"
            "\nincluding the module whose job is stripping them). No gate can see it --"
            "\nfmt, clippy and the test suite all read code, and a control byte in a"
            "\ncomment is not code."
            "\n\nTo fix one: replace the byte with the letter it stands for. If a file"
            "\nreally must show an escape, write the escape text (`\\x1b`), never the"
            "\nbyte."
        )
        return 1

    if not quiet:
        print(
            f"check-control-bytes: {len(files)} tracked files, 0 control bytes "
            f"({len(binary)} binary suffix(es) skipped)"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))