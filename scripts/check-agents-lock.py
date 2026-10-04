#!/usr/bin/env python3
"""`AGENTS.md` is maintainer-owned: this gate fails when its bytes stop matching the lock.

## Why this exists

`AGENTS.md` loads into the context of every agent that opens this repository, and it is the
file that tells those agents what the rules are. A rule file that any agent may edit is not a
rule file -- it is a suggestion that accumulates exceptions, because the cheapest way to get
past a rule is to rewrite the rule.

So the file is locked, and the lock is **checked** rather than asserted. An assertion lives in
a document; a check lives in a gate that fails the build.

The workflow for a genuine correction is deliberately not "edit the file":

1. append the proposed change to `docs/AGENTS-PROPOSALS.md`, and
2. say so in the final report.

The maintainer applies proposals and re-issues the lock. That keeps one writer for this file.

## What it checks

The SHA-256 of `AGENTS.md`, against the digest recorded in `AGENTS.md.sha256`.

CRLF is normalised to LF before hashing. The repository declares `* text=auto eol=lf`, so a
correct checkout is already LF and normalisation changes nothing; but `sha256sum` on Windows
tools has a habit of handing back a CRLF file, and a lock that fails on a line ending is a
lock people disable. `scripts/check-line-endings.py` is the gate that actually owns line
endings; this one owns content.

The digest file is read in `sha256sum` format (`<hex>  <name>`), so `sha256sum -c` works on it
directly on any platform that has the tool.

## What it does not do

It does not prevent an edit. Nothing can, from inside the repository -- the OS read-only bit is
advisory and this repository may be checked out on filesystems that ignore it. What it does is
make the edit *visible and blocking* on the next gate run or CI, which is the property that
actually holds a rule file together.
"""

from __future__ import annotations

import hashlib
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TARGET = "AGENTS.md"
LOCK = "AGENTS.md.sha256"


def digest(text: str) -> str:
    """SHA-256 of the file's bytes with endings normalised to LF."""
    return hashlib.sha256(text.replace("\r\n", "\n").encode("utf-8")).hexdigest()


def read_expected() -> str | None:
    """The recorded digest, or None when the lock file is missing or unreadable."""
    lock_path = REPO / LOCK
    if not lock_path.is_file():
        return None
    for raw in lock_path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        # `<hex>  <name>`; tolerating a bare hex keeps a hand-edited lock file usable.
        return line.split()[0].lower()
    return None


def main(argv: list[str]) -> int:
    quiet = "--quiet" in argv
    target_path = REPO / TARGET
    if not target_path.is_file():
        print(f"check-agents-lock: {TARGET} is missing")
        return 1

    expected = read_expected()
    if expected is None:
        print(
            f"check-agents-lock: {LOCK} is missing or empty.\n\n"
            f"{TARGET} is maintainer-owned and locked. To issue a lock:\n"
            f'    python -c "import hashlib,pathlib;'
            f'print(hashlib.sha256(pathlib.Path(\'{TARGET}\').read_bytes())'
            f'.hexdigest())" > {LOCK}\n'
            f"then prepend nothing -- the file holds one line: '<digest>  {TARGET}'."
        )
        return 1

    actual = digest(target_path.read_text(encoding="utf-8"))
    if actual == expected:
        if not quiet:
            # ASCII only: this console's code page mangles a real ellipsis.
            print(f"check-agents-lock: {TARGET} matches its lock ({actual[:12]}...)")
        return 0

    print(
        f"check-agents-lock: {TARGET} does NOT match {LOCK}\n"
        f"  locked   {expected}\n"
        f"  on disk  {actual}\n\n"
        f"{TARGET} is maintainer-owned; agents never edit it. If you changed it by accident,\n"
        f"restore it (`git checkout -- {TARGET}`) and re-run this check. If you believe the\n"
        f"change is right, it is not yours to make: append the proposal to\n"
        f"docs/AGENTS-PROPOSALS.md, leave {TARGET} alone, and mention it in your report."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
