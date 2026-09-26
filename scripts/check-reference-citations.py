"""Check every `references/pi-0.84.2/...:<lines>` citation in the repo against the
checkout that actually exists (pi-0.85.1).

Two objective checks, no guessing:
  1. does the cited FILE still exist under the current checkout?
  2. is the cited LINE RANGE still inside that file?

A citation that passes both is NOT thereby correct -- line numbers may have
shifted onto different content. This script only proves a citation is
*checkable*, and it finds the ones that provably are not. Anything that passes
here still needs a human to confirm the cited lines say what the doc claims.
"""

import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REFS = os.path.join(REPO, "references")

# references/pi-0.84.2/packages/<pkg>/<path>.ts:12-34,56  or  :68 or :401
# NOTE: the extension alternation must be ordered longest-first, or `package.json`
# matches as `package.js` and the `:2-4` line range is silently left behind. The
# trailing lookahead keeps `foo.json` from being re-matched as `foo.js` + "on".
# Two shapes, and the brace form must come FIRST. A single alternation of
# `path + extension` stops at the first `.ts` *inside* `{a.ts,b.ts}` and yields
# the nonsense path `src/{a.ts`, which then reads as "file not found". The
# subagent who found this gap was right that the earlier count under-reported.
CITE = re.compile(
    r"references/pi-0\.84\.2/"
    r"(?P<path>[\w./\-]*\{[^{}]*\}[\w./\-]*"  # brace group, whole
    r"|[\w./\-]+\.(?:tsx|ts|json|js|md))"  # plain file
    r"(?![A-Za-z])"
    r"(?::(?P<lines>\d+(?:[–\-]\d+)?(?:,\d+(?:[–\-]\d+)?)*))?"
)


def expand(path: str) -> list[str]:
    """Expand brace alternation: `src/{a.ts,b/c.ts}` -> the concrete paths.

    Without this, a brace-form citation is silently skipped -- the regex just
    fails to match and the count comes out low with no error. A subagent caught
    exactly that: 5 citations were never validated by the first version of this
    script, and the script reported "all checkable" without mentioning them.
    A tool that under-reports looks identical to a tool that finds nothing wrong.
    """
    m = re.search(r"\{([^{}]+)\}", path)
    if not m:
        return [path]
    head, tail = path[: m.start()], path[m.end() :]
    out = []
    for alt in m.group(1).split(","):
        out.extend(expand(head + alt.strip() + tail))
    return out

# Only these trees are worth walking; scratch/build output is noise.
SCAN_DIRS = ["audit", "docs", "crates"]


def cited_versions():
    """Every references/<dir> that a citation actually names."""
    named = set()
    for d in SCAN_DIRS:
        for root, _dirs, files in os.walk(os.path.join(REPO, d)):
            if "target" in root.split(os.sep):
                continue
            for f in files:
                if not f.endswith((".md", ".rs")):
                    continue
                try:
                    text = open(os.path.join(root, f), encoding="utf-8", errors="replace").read()
                except OSError:
                    continue
                named.update(re.findall(r"references/(pi-[\w.\-]+)/", text))
    return named


def main():
    print("reference checkouts present:")
    for name in sorted(os.listdir(REFS)):
        print(f"  {name}")
    named = cited_versions()
    print(f"\ndistinct versions named in citations: {sorted(named)}")
    missing = [v for v in sorted(named) if not os.path.isdir(os.path.join(REFS, v))]
    print(f"named but ABSENT from references/: {missing}\n")

    total = ok = bad_path = bad_line = 0
    per_file = {}
    for d in SCAN_DIRS:
        for root, _dirs, files in os.walk(os.path.join(REPO, d)):
            if "target" in root.split(os.sep):
                continue
            for f in files:
                if not f.endswith((".md", ".rs")):
                    continue
                full = os.path.join(root, f)
                rel = os.path.relpath(full, REPO)
                text = open(full, encoding="utf-8", errors="replace").read()
                for m in CITE.finditer(text):
                    total += 1
                    lines = m.group("lines")
                    for path in expand(m.group("path")):
                        # Does it exist under the version we ACTUALLY have?
                        cur = os.path.join(REFS, "pi-0.85.1", path)
                        if not os.path.isfile(cur):
                            bad_path += 1
                            per_file.setdefault(rel, []).append(
                                (path, lines, "no such file in pi-0.85.1")
                            )
                            continue
                        if lines:
                            n = len(
                                open(cur, encoding="utf-8", errors="replace").read().splitlines()
                            )
                            worst = 0
                            for part in re.split(r",", lines):
                                part = part.replace("–", "-")
                                nums = [int(x) for x in re.findall(r"\d+", part)]
                                worst = max(worst, max(nums) if nums else 0)
                            if worst > n:
                                bad_line += 1
                                per_file.setdefault(rel, []).append(
                                    (path, lines, f"cites line {worst}, file has {n}")
                                )
                                continue
                        ok += 1

    print(f"citations to pi-0.84.2: {total}")
    print(f"  file not found in pi-0.85.1 : {bad_path}")
    print(f"  line range past end of file  : {bad_line}")
    print(f"  path+range both checkable    : {ok}")
    if per_file:
        print("\nby file:")
        for rel in sorted(per_file, key=lambda r: -len(per_file[r])):
            print(f"  {len(per_file[rel]):3d}  {rel}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
