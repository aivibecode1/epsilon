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
run "fmt --check"   "$CARGO" fmt --all -- --check            || overall=1
run "clippy"        "$CARGO" clippy --workspace --all-targets -- -D warnings || overall=1
# `-D warnings`: the workspace denies its configured lints, but clippy's own `-W clippy::all`
# findings exit 0 by default — which is how a warning got past this gate once (M69).
run "test"          "$CARGO" test --workspace                 || overall=1
run "demo"          "$CARGO" run -q -p epsilon-cli -- --demo   || overall=1
run "real-check"    "$CARGO" check -p epsilon-cli --features real || overall=1
printf '\n===== GATE %s =====\n' "$([ $overall -eq 0 ] && echo PASS || echo FAIL)" >> "$LOG"
exit $overall
