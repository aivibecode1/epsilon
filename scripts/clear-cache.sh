#!/usr/bin/env bash
# Clear the Rust build cache after a build or test round.
#
# The host drive is chronically near-full, so a full `target/` (35-40 GB) is
# not a cache, it is an outage. This is the "clear the cache after every build
# or test" step, written down once so it is not a remembered command.
#
# Usage:
#   scripts/clear-cache.sh            # clear the shared target dir
#   scripts/clear-cache.sh --report   # say what it freed, delete nothing
#
# The directory is disposable build output. Nothing in it is source, nothing
# is committed, and rebuilding it costs minutes rather than correctness — which
# is the whole trade a small drive asks for.

set -u

# Outside the repository, per the build-env rules: writing build output inside
# the tree makes cl.exe fail with D8050 and cargo report a broken *dependency*.
TARGET_DIR="${CARGO_TARGET_DIR:-$LOCALAPPDATA/Temp/epsilon-target}"

if [ ! -d "$TARGET_DIR" ]; then
  echo "no build cache at $TARGET_DIR"
  exit 0
fi

before="$(du -sh "$TARGET_DIR" 2>/dev/null | cut -f1)"

if [ "${1:-}" = "--report" ]; then
  echo "build cache at $TARGET_DIR: $before"
  df -h /c | tail -1
  exit 0
fi

# Not a blanket clean of unknown paths: this is one directory, it is ours, and
# it is regenerable. Nothing under references/, .git, or the source tree is in
# scope of this script.
rm -rf "$TARGET_DIR"

after="$(du -sh "$TARGET_DIR" 2>/dev/null | cut -f1)"
echo "cleared $TARGET_DIR: $before -> ${after:-gone}"
df -h /c | tail -1
