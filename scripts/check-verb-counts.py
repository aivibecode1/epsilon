"""Guard the desktop removal (ADR-039): the verb-count gate is retired, not deleted.

Why this script still exists
----------------------------
`check-verb-counts.py` used to hold the window's verb prose
(`docs/parity/dsh-desktop.md`, the desktop `SUPPORTED` table,
`docs/plans/025-completion-plan.md`) against the shared `COMMAND_HEADS`.
The desktop crate is removed, so there is no `SUPPORTED` table left to hold
prose against — and a gate leg that always passes would be the silent no-op
`AGENTS.md` §2.7 forbids. Deleting the leg instead would silently shrink the
battery, which is worse: a missing leg does not lie, but nobody would notice
it left.

So the leg stays and checks the removal itself. It fails when:

* `crates/epsilon-desktop/` exists again (the crate was re-added without
  reopening ADR-039 and restoring the gates that read it);
* the workspace still members `epsilon-desktop` or still pins the
  desktop-only GUI stack (`eframe`, `egui`);
* the retired matrix no longer carries its retirement banner (history must
  say it is history, or a reader will cite frozen rows as current).

This is the removal control for the deletion: it was written first, failed
against the tree that still had the desktop (RED), and passes only when the
removal it guards is real.

    python scripts/check-verb-counts.py            # report, exit 1 on drift
    python scripts/check-verb-counts.py --quiet    # summary line only
"""

from __future__ import annotations

import argparse
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

RETIRED_MATRIX = os.path.join(ROOT, "docs", "parity", "dsh-desktop.md")
BANNER = "RETIRED"
ADR = "ADR-039"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args()

    problems: list[str] = []

    if os.path.exists(os.path.join(ROOT, "crates", "epsilon-desktop")):
        problems.append(
            "crates/epsilon-desktop exists; the desktop is removed (ADR-039)"
        )

    with open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8") as f:
        workspace = f.read()
    workspace_lines = [
        line for line in workspace.splitlines() if not line.strip().startswith("#")
    ]
    workspace_code = "\n".join(workspace_lines)
    if "epsilon-desktop" in workspace_code:
        problems.append("Cargo.toml still names epsilon-desktop")
    for pin in ("eframe", "egui"):
        # A crate name in a comment is fine; a dependency pin is the removal.
        for line in workspace_lines:
            text = line.strip()
            if text.startswith("#"):
                continue
            if pin in text and "=" in text:
                problems.append(f"Cargo.toml still pins desktop-only '{pin}'")
                break

    try:
        with open(RETIRED_MATRIX, encoding="utf-8") as f:
            matrix = f.read()
    except OSError:
        problems.append("docs/parity/dsh-desktop.md is missing entirely")
        matrix = ""
    if matrix and (BANNER not in matrix or ADR not in matrix):
        problems.append(
            "docs/parity/dsh-desktop.md carries no retirement banner "
            f"('{BANNER}' + '{ADR}')"
        )

    if problems and not args.quiet:
        print("desktop-removal drift:")
        for p in problems:
            print(f"  - {p}")
    if args.quiet:
        print(f"verb-counts: desktop-removal guard, {len(problems)} problem(s)")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
