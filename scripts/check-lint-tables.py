#!/usr/bin/env python3
"""Every crate's `[lints]` table must either inherit the workspace's or match it exactly.

## Why this exists

Cargo does not merge a package `[lints]` table with `[workspace.lints]`. A package
table **replaces** it unless it carries `workspace = true`, and `workspace = true`
is *rejected* when the same table also carries overrides:

    error: cannot override `workspace.lints` in `lints`, either remove the
           overrides or remove `lints.workspace = true` and manually specify the
           lints

So there are exactly two legal shapes for a package, and both have a failure mode:

| Shape | Failure mode |
|---|---|
| `workspace = true` | none — it tracks the root automatically |
| a hand-written copy | it silently stops matching the root |

The second is not a hypothetical. `epsilon-safety` has to declare `unsafe_code =
"allow"`, because `forbid` is un-overridable and the Job Object FFI in
`sandbox::jobobject` is the only `unsafe` in the workspace. That forces a
hand-written copy — and it is the crate containing every `unsafe` block, so a lint
added to `[workspace.lints]` would reach the other eleven crates and stop here,
with no warning from cargo and no failing test.

That is the same failure `epsilon-lsp` had (D1/D21: it declared `[lints.rust]` with
`missing_docs` and no `workspace = true`, and so compiled with neither
`unsafe_code = "forbid"` nor `clippy::all`). It was found by a human reading twelve
manifests. This is the check that means nobody has to.

## What it asserts

For each member crate, exactly one of:

* `[lints]` contains `workspace = true` — the correct shape, nothing to compare; or
* `[lints.rust]` / `[lints.clippy]` are present and **equal** to the workspace
  tables, except for lints listed in `ALLOWED_OVERRIDES`, each of which must carry
  a comment naming its reason.

A crate with no `[lints]` at all is also a failure: `missing_lints_inheritance` is a
default-warn cargo lint precisely because the inheritance is not implicit.

Run from the repository root. Exits non-zero with one line per problem.
"""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MEMBERS = ROOT / "Cargo.toml"

# Lints a package may set differently from the workspace, and why.
#
# Each entry is (lint name, required substring of the reason). The substring is
# required rather than optional because an unexplained override is the thing this
# file exists to prevent: "kept for parity" and "we need this" are not reasons, and
# a table that grows an override nobody can justify is how the drift started.
ALLOWED_OVERRIDES: dict[str, str] = {
    # `forbid` is un-overridable, so the one crate with FFI cannot inherit the
    # workspace's `forbid` and must restate the table. See its Cargo.toml.
    "unsafe_code": "AGENTS.md exception",
    # The one crate that owns every `unsafe` block carries a lint the root does
    # not: it is the machine-checkable half of "every unsafe block carries a
    # SAFETY comment", which is the containment discipline AGENTS.md states.
    "undocumented_unsafe_blocks": "AGENTS.md exception",
}

TABLE = re.compile(r"^\[(?P<name>[A-Za-z0-9_.-]+)\]\s*$")
KEY = re.compile(r"^(?P<key>[A-Za-z0-9_-]+)\s*=\s*(?P<value>.+?)\s*$")

# Markdown punctuation a comment is likely to wrap a phrase in.
_NOISE = str.maketrans("", "", "`*_'\"")


def normalize(text: str) -> str:
    """Compare prose, not punctuation.

    The required phrase is matched case-insensitively and with quoting and
    emphasis removed, because a comment that writes `` `AGENTS.md` exception ``
    has said the same thing as one that writes `AGENTS.md exception` and a check
    that cannot tell the difference is a check that gets worked around.
    """
    return re.sub(r"\s+", " ", text.translate(_NOISE)).lower()


def parse_tables(text: str) -> dict[str, dict[str, str]]:
    """The tables we care about, as `{table: {key: value}}`.

    Comments are stripped before key/value lines are read, so a `reason = "..."` on
    an attribute is not mistaken for a lint level.
    """
    tables: dict[str, dict[str, str]] = {}
    current: str | None = None
    for raw in text.splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        m = TABLE.match(line)
        if m:
            current = m.group("name")
            tables.setdefault(current, {})
            continue
        if current is None:
            continue
        m = KEY.match(line)
        if m:
            tables[current][m.group("key")] = m.group("value").strip('"')
    return tables


def comment_near(text: str, table: str, key: str) -> str:
    """The justification available for one lint line.

    Preference order, and the order matters:

    1. A comment block immediately above the line, or on the line below it.
    2. Failing that, **any** comment in the file.

    The second rule is deliberate and is what keeps the check from being a
    formatting ritual. A manifest documents its lint exception in a header
    explaining *why the table has to be hand-copied at all*, and that is where the
    reason genuinely lives — a per-line comment repeating it would say the same
    words twice and drift from the first copy. The requirement is that the file
    says why, not that it says it in one particular place; demanding a
    particular position is how a check starts being worked around.
    """
    lines = text.splitlines()
    marker = f"{key} ="
    for index, line in enumerate(lines):
        if marker not in line or line.split("#", 1)[0].strip() == "":
            continue
        block: list[str] = []
        cursor = index - 1
        while cursor >= 0 and lines[cursor].strip().startswith("#"):
            block.append(lines[cursor].strip())
            cursor -= 1
        if not block and index + 1 < len(lines):
            block.append(lines[index + 1].strip())
        if block:
            return " ".join(reversed(block))
    # Fall back to the file's own comments as a whole.
    return " ".join(
        line.strip().lstrip("#").strip() for line in lines if line.strip().startswith("#")
    )


def main() -> int:
    root_manifest = tomllib.loads(MEMBERS.read_text(encoding="utf-8"))
    # `[workspace.lints]` is nested *under* `[workspace]`, so it is
    # `workspace.lints` in the parsed document — not a top-level `lints` key.
    # Reading the wrong one yields `None`, every comparison is against an empty
    # table, and the check passes on a workspace whose lint law is a fiction.
    # Which is exactly what happened the first time this ran.
    workspace = root_manifest.get("workspace", {}).get("lints", {})
    if not workspace:
        print(
            "Cargo.toml has no [workspace.lints] table; there is no law to check "
            "against.",
            file=sys.stderr,
        )
        return 2

    problems: list[str] = []
    members = root_manifest.get("workspace", {}).get("members", [])
    if not members:
        print("no workspace members found in Cargo.toml", file=sys.stderr)
        return 2

    inherited = 0
    matched = 0
    for member in members:
        manifest_path = ROOT / member / "Cargo.toml"
        if not manifest_path.is_file():
            problems.append(f"{member}: manifest not found")
            continue
        text = manifest_path.read_text(encoding="utf-8")
        data = tomllib.loads(text)
        lints = data.get("lints", {})
        name = data["package"]["name"]

        if lints.get("workspace") is True:
            inherited += 1
            continue

        tables = parse_tables(text)
        if "lints.rust" not in tables and "lints.clippy" not in tables:
            problems.append(
                f"{name}: no [lints] table. Cargo does not inherit "
                f"[workspace.lints] implicitly, so this crate compiles with no "
                f"lint law at all. Add `workspace = true`."
            )
            continue

        for section in ("rust", "clippy"):
            want = dict(workspace.get(section, {}))
            have = tables.get(f"lints.{section}", {})
            reasons = normalize(comment_near(text, f"lints.{section}", section))

            # An allowed override is removed from the comparison entirely: it is
            # allowed to be *absent*, to be *new*, and to carry a *different
            # level*, because that is what an override is. `unsafe_code` is the
            # case in point — the workspace says `forbid` and this crate says
            # `allow`, which is the whole exception, and a check that flagged it
            # would be flagging the one difference that is allowed to exist.
            #
            # What it must still do is *say why*, and that is checked here: an
            # override with no recorded reason is the drift this file exists to
            # stop, because the next person to widen it finds no argument
            # against.
            for lint in list(have):
                if lint in ALLOWED_OVERRIDES:
                    if normalize(ALLOWED_OVERRIDES[lint]) not in reasons:
                        problems.append(
                            f"{name}: [lints.{section}] overrides {lint}, which is "
                            f"allowed, but the file's comments do not say why. "
                            f"Required phrase: {ALLOWED_OVERRIDES[lint]!r}"
                        )
                    have.pop(lint)
                    want.pop(lint, None)

            # Anything left is a lint the crate declares and the workspace does
            # not, with no recorded reason for existing at all.
            for lint, level in have.items():
                if lint not in want:
                    problems.append(
                        f"{name}: [lints.{section}] sets {lint} = {level}, which "
                        f"the workspace does not define and this script has no "
                        f"recorded reason for. Add it to ALLOWED_OVERRIDES with a "
                        f"reason, or drop it."
                    )
            have = {k: v for k, v in have.items() if k in want}

            missing = {k: v for k, v in want.items() if k not in have}
            for lint, level in missing.items():
                problems.append(
                    f"{name}: [lints.{section}] is missing {lint} = {level} from "
                    f"the workspace table. This crate hand-copies the table, so a "
                    f"lint added at the root reaches the other crates and stops "
                    f"here — silently, and with cargo reporting nothing."
                )
            differing = {k: v for k, v in have.items() if want[k] != v}
            for lint, level in differing.items():
                problems.append(
                    f"{name}: [lints.{section}] sets {lint} = {level}, but the "
                    f"workspace says {want[lint]}. A hand-copied table that "
                    f"quietly changes a level is the same defect as one that "
                    f"omits a lint."
                )
        matched += 1

    if problems:
        print("lint tables have drifted from [workspace.lints]:", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1

    print(
        f"lint tables OK — {inherited} crate(s) inherit the workspace, "
        f"{matched} hand-copied and matching it"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
