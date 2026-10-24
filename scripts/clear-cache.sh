#!/usr/bin/env sh
# Clear the Rust build cache after a build or test round.
#
# A full target directory is disposable build output. On the development machine
# it is also 4-35 GB, which is why clearing it is a written-down step rather
# than a remembered command.
#
# Usage:
#   scripts/clear-cache.sh                 # clear the shared target dir
#   scripts/clear-cache.sh --report        # say what it uses, delete nothing
#   scripts/clear-cache.sh --target DIR    # clear one explicit directory
#
# Only a directory whose name contains "target" is removed. The script never
# touches the repository, references/, .git or a user data directory.
set -u

REPORT=0
EXPLICIT=""

while [ $# -gt 0 ]; do
    case "$1" in
        --report) REPORT=1; shift ;;
        --target)
            [ $# -ge 2 ] || { echo "clear-cache: --target needs a directory" >&2; exit 2; }
            EXPLICIT=$2
            shift 2
            ;;
        -h|--help)
            sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "clear-cache: unknown argument: $1" >&2
            exit 2
            ;;
    esac
done

if [ -n "$EXPLICIT" ]; then
    CANDIDATES=$EXPLICIT
elif [ -n "${CARGO_TARGET_DIR:-}" ]; then
    CANDIDATES=$CARGO_TARGET_DIR
elif [ -n "${LOCALAPPDATA:-}" ]; then
    CANDIDATES="$LOCALAPPDATA/Temp/epsilon-target"
else
    CANDIDATES="${TMPDIR:-/tmp}/epsilon-target"
fi

show_usage() {
    df -h "$1" 2>/dev/null | tail -1 || df -h . 2>/dev/null | tail -1 || true
}

clear_one() {
    dir=$1
    case "$dir" in
        ""|"/"|"."|"..") echo "refusing unsafe cache path: '$dir'" >&2; return ;;
    esac
    case "$(basename "$dir")" in
        *target*) : ;;
        *) echo "refusing '$dir': the directory name does not contain 'target'" >&2; return ;;
    esac
    if [ ! -d "$dir" ]; then
        echo "no build cache at $dir"
        return
    fi
    before=$(du -sh "$dir" 2>/dev/null | cut -f1)
    if [ "$REPORT" -eq 1 ]; then
        echo "build cache at $dir: $before"
        show_usage "$dir"
        return
    fi
    rm -rf -- "$dir"
    echo "cleared $dir: $before -> gone"
    show_usage "$dir"
}

# Newline-separated so a path containing spaces is not split into two.
printf '%s\n' "$CANDIDATES" | while IFS= read -r dir; do
    [ -n "$dir" ] && clear_one "$dir"
done
