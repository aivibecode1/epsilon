#!/usr/bin/env sh
# Install Epsilon on macOS or Linux from this checkout.
#
#   bash install.sh              # install into ~/.cargo/bin (or $CARGO_HOME/bin)
#   bash install.sh --prefix DIR # install under DIR/bin
#   bash install.sh --no-verify  # skip the post-install smoke check
#
# Requires Rust 1.88 or newer (https://rustup.rs). Build output is placed in a
# temporary target directory rather than inside the repository: in-tree output
# has produced compiler failures on this project's machine, and an installer
# should not reproduce them.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PREFIX=""
VERIFY=1

usage() {
    sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)
            [ $# -ge 2 ] || { echo "install.sh: --prefix needs a directory" >&2; exit 2; }
            PREFIX=$2
            shift 2
            ;;
        --prefix=*)
            PREFIX=${1#--prefix=}
            shift
            ;;
        --no-verify)
            VERIFY=0
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "install.sh: unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

CARGO=${CARGO:-}
if [ -z "$CARGO" ]; then
    CARGO=$(command -v cargo 2>/dev/null || true)
fi
if [ -z "$CARGO" ] && [ -x "${HOME:-/nonexistent}/.cargo/bin/cargo" ]; then
    CARGO="${HOME}/.cargo/bin/cargo"
fi
if [ -z "$CARGO" ]; then
    echo "install.sh: cargo was not found." >&2
    echo "Install Rust 1.88+ from https://rustup.rs, then run this script again." >&2
    exit 127
fi

RUSTC=${RUSTC:-$(dirname "$CARGO")/rustc}
if [ -x "$RUSTC" ]; then
    RUST_VERSION=$("$RUSTC" --version 2>/dev/null | awk '{print $2}')
    MAJOR=${RUST_VERSION%%.*}
    REST=${RUST_VERSION#*.}
    MINOR=${REST%%.*}
    case "$MAJOR" in ''|*[!0-9]*) MAJOR=0 ;; esac
    case "$MINOR" in ''|*[!0-9]*) MINOR=0 ;; esac
    if [ "$MAJOR" -lt 1 ] || { [ "$MAJOR" -eq 1 ] && [ "$MINOR" -lt 88 ]; }; then
        echo "install.sh: Rust $RUST_VERSION is too old; Epsilon needs 1.88 or newer." >&2
        echo "Update with: rustup update stable" >&2
        exit 1
    fi
fi

if [ -z "${CARGO_TARGET_DIR:-}" ]; then
    CARGO_TARGET_DIR="${TMPDIR:-/tmp}/epsilon-install-target"
    export CARGO_TARGET_DIR
fi

echo "Epsilon installer"
echo "  checkout:     $ROOT"
echo "  cargo:        $CARGO"
echo "  target dir:   $CARGO_TARGET_DIR"
[ -n "$PREFIX" ] && echo "  install root: $PREFIX"

set -- install --path "$ROOT/crates/epsilon-cli" --locked --force
if [ -n "$PREFIX" ]; then
    set -- "$@" --root "$PREFIX"
fi
"$CARGO" "$@"

if [ -n "$PREFIX" ]; then
    BIN="$PREFIX/bin"
else
    BIN="${CARGO_HOME:-${HOME:-/nonexistent}/.cargo}/bin"
fi
EPSILON="$BIN/epsilon"

if [ "$VERIFY" -eq 1 ]; then
    if [ ! -x "$EPSILON" ]; then
        echo "install.sh: installation finished but $EPSILON is not executable." >&2
        exit 1
    fi
    "$EPSILON" --version
fi

echo
echo "Installed: $EPSILON"
case ":${PATH:-}:" in
    *":$BIN:"*) : ;;
    *)
        echo
        echo "$BIN is not on PATH. Add it to your shell profile, for example:"
        echo "  export PATH=\"$BIN:\$PATH\""
        ;;
esac
echo
echo "Next:"
echo "  epsilon --demo     # offline smoke test, no key and no network"
echo "  epsilon --help     # modes, flags, environment and files"
echo "  epsilon --rea      # optional: attach the REA reverse-engineering MCP server"
echo
case "$(uname -s 2>/dev/null || echo unknown)" in
    Darwin)
        echo "macOS note: shell commands refuse by default because no native confinement"
        echo "backend is available. Read docs/user/permissions.md before using --sandbox off."
        ;;
    Linux)
        echo "Linux note: install bubblewrap (bwrap) for shell confinement. Without it,"
        echo "side-effecting shell calls refuse rather than running unconfined."
        ;;
esac
