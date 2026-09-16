#!/usr/bin/env bash
# Full verification battery for Epsilon. Writes a durable log so a run can be
# inspected after the fact. Usage: bash scripts/gate.sh [label]
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
LABEL="${1:-run}"
LOG="target/gate-$LABEL.log"
mkdir -p target
: > "$LOG"
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
run "fmt --check"   cargo fmt --all -- --check            || overall=1
run "clippy"        cargo clippy --workspace --all-targets -- -D warnings || overall=1
# `-D warnings`: the workspace denies its configured lints, but clippy's own `-W clippy::all`
# findings exit 0 by default — which is how a warning got past this gate once (M69).
run "test"          cargo test --workspace                 || overall=1
run "demo"          cargo run -q -p epsilon-cli -- --demo   || overall=1
run "real-check"    cargo check -p epsilon-cli --features real || overall=1
printf '\n===== GATE %s =====\n' "$([ $overall -eq 0 ] && echo PASS || echo FAIL)" >> "$LOG"
exit $overall
