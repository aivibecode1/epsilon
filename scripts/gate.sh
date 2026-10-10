#!/usr/bin/env bash
# Full verification battery for Epsilon. Writes a durable log so a run can be
# inspected after the fact. Usage: bash scripts/gate.sh [label]
#
# Every battery attempt needs its OWN label: the log below is truncated on start,
# so re-running with a label that already exists destroys the failed attempt's
# evidence (AGENTS.md, "Verification rounds on this machine").
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
LABEL="${1:-run}"
LOG="target/gate-$LABEL.log"
mkdir -p target
: > "$LOG"

# Resolve cargo BEFORE running anything.
#
# A bash launched from a Windows coding harness does not necessarily inherit the
# PATH PowerShell sees, so `cargo` can be missing here while it works in the shell
# next door. That failure mode is nasty out of proportion to its cause: every gate
# exits 127 in zero seconds and the log ends in GATE FAIL, which reads like five
# broken gates rather than one missing PATH entry. It has cost this project more
# than one round (AGENTS.md, "Windows notes"). The override exists for a cargo
# that lives somewhere unusual; the fallback covers the ordinary install.
CARGO="${CARGO:-$(command -v cargo 2>/dev/null || true)}"
if [ -z "$CARGO" ]; then
  for candidate in "$HOME/.cargo/bin/cargo.exe" "$HOME/.cargo/bin/cargo"; do
    if [ -x "$candidate" ]; then
      CARGO="$candidate"
      break
    fi
  done
fi
if [ -z "$CARGO" ]; then
  echo "gate.sh: cannot find cargo — set CARGO=/path/to/cargo and re-run" >&2
  exit 127
fi
printf 'cargo: %s\n' "$CARGO" >> "$LOG"

run() {
  printf '\n===== %s =====\n' "$1" >> "$LOG"
  shift
  start=$(date +%s)
  "$@" >> "$LOG" 2>&1
  rc=$?
  end=$(date +%s)
  printf '[exit=%s %ss]\n' "$rc" "$((end - start))" >> "$LOG"
  return $rc
}
overall=0
# Documentation links are a gate, not a chore.  The audit renumbering of
# 2026-09-28 left 40 dead relative links across the docs, and nothing noticed,
# because a dead citation looks exactly like a live one until a reader follows
# it.  It costs a second and it is the only check that can see this class.
run "check-links"    python scripts/check-links.py --quiet        || overall=1
# Cargo does not merge a package `[lints]` table with `[workspace.lints]`: a package
# table REPLACES it unless it says `workspace = true`, and cargo rejects
# `workspace = true` when the same table carries overrides.  So a crate that must
# override one lint — `epsilon-safety`, for the FFI — hand-copies the whole table, and
# a lint added at the root afterwards reaches every other crate and stops there.
# `epsilon-lsp` had exactly that bug (D1/D21), found by a human reading twelve
# manifests; this is the check that means nobody has to.
run "check-lint-tables" python scripts/check-lint-tables.py       || overall=1
# Parity-matrix citations name production lines, and moving code re-points
# every citation below it without touching the row. The export move deleted 53
# lines and shifted nineteen rows in round 14, all repaired by hand; this is
# the check that means nobody has to. Reference-side paths are excluded (not
# committed) -- see the script.
run "check-parity-citations" python scripts/check-parity-citations.py --quiet || overall=1
# A citation that resolves can still land inside the crate's test module -- a
# harness line, never what a done row's ours column means (the fifth goal
# audit's finding; the vintage remap cannot see a row wrong when written).
run "check-citation-placement" python scripts/check-citation-placement.py || overall=1
# The reference column resolves against the vendored trees: the workspace-side
# checker skips references/ because a fresh clone lacks it, which left 883
# reference citations unverified by construction. Absent trees: says so, exits 0.
run "check-reference-citations" python scripts/check-reference-citations.py || overall=1
# Verb counts are stated in prose in four places and updated by hand every
# verb round; four rounds running each missed at least one site. Dated
# records are excluded by design -- see the script.
run "check-verb-counts" python scripts/check-verb-counts.py --quiet || overall=1
# Matrix summaries, totals, open-row lists and embedded heading counts are
# hand-maintained; stale ids sat in a missing list two rounds after going
# done. Per-section placement is verified for pi-tui only -- see the script.
run "check-parity-counts" python scripts/check-parity-counts.py --quiet || overall=1
# A CRLF working copy is invisible to `git diff` (`.gitattributes` normalises on
# commit) and breaks every source-parsing test in the tree while reporting
# something else entirely -- M231 lost a battery to a dispatcher reported as
# having no catch-all arm, which was false.  One second, and it names the file.
run "check-line-endings" python scripts/check-line-endings.py --quiet || overall=1
# `AGENTS.md` is maintainer-owned and locked: an agent that may rewrite the rules is
# not governed by them.  The lock is a SHA-256 in `AGENTS.md.sha256`; this gate fails
# when the bytes move.  A proposal belongs in `docs/AGENTS-PROPOSALS.md`.
run "check-agents-lock" python scripts/check-agents-lock.py || overall=1
# A C0 control byte inside a tracked text file is a corrupted *letter*, not content:
# `e` -> 0x1B, `a` -> 0x07, `b` -> 0x08 is the shape this repository has shipped 15
# times, twice inside `epsilon-runtime/src/text_safety.rs` -- the module whose whole
# job is stripping those bytes.  No other gate can see it, because fmt and clippy read
# code and a control byte in a comment is not code (ledger A49).
run "check-control-bytes" python scripts/check-control-bytes.py --quiet || overall=1
run "fmt --check"   "$CARGO" fmt --all -- --check            || overall=1
run "clippy"        "$CARGO" clippy --workspace --all-targets -- -D warnings || overall=1
# `-D warnings`: the workspace denies its configured lints, but clippy's own `-W clippy::all`
# findings exit 0 by default — which is how a warning got past this gate once (M69).
run "test"          "$CARGO" test --workspace                 || overall=1
run "demo"          "$CARGO" run -q -p epsilon-cli -- --demo   || overall=1
run "real-check"    "$CARGO" check -p epsilon-cli --features real || overall=1
printf '\n===== GATE %s =====\n' "$([ $overall -eq 0 ] && echo PASS || echo FAIL)" >> "$LOG"
exit $overall
