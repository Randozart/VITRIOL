#!/usr/bin/env bash
# install.sh — put VITRIOL on your PATH (build + symlink, no sudo, idempotent).
#
#   ./scripts/install.sh                        build + install to ~/.local/bin
#   ./scripts/install.sh --prefix ~/bin         install to a specific bin dir
#   ./scripts/install.sh --no-build             symlink only (skip cargo build)
#   ./scripts/install.sh --list                 show what would be installed
#
# What it does:
#   - Builds vitriol-tui (the Ratatui ops dashboard) with a release build.
#   - Symlinks `vitriol` (a script resolved via readlink -f, so it works from any
#     cwd) and `vitriol-tui` into <prefix>/bin, default ~/.local/bin.
#   - Scaffolds ~/.vitriol/{config,profiles} if absent.
#   - Never sudo, never destructive: re-running updates symlinks in place.
#
# Requirements: cargo; a built llama.cpp server (see llama.cpp/build/bin) and the
# model files — this script does NOT build the inference server or fetch models.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

PREFIX="${HOME:-$HOME}/.local"
NO_BUILD=0
UNINSTALL=0

usage() {
    cat <<'EOF'
Usage: scripts/install.sh [--prefix DIR] [--no-build] [--uninstall]
  --prefix DIR   bin directory root (default ~/.local -> ~/.local/bin)
  --no-build     symlink only; skip the cargo build
  --uninstall    remove the symlinks and the tui binary
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --prefix) PREFIX="$2"; shift 2 ;;
        --no-build) NO_BUILD=1; shift ;;
        --uninstall) UNINSTALL=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "install.sh: unknown arg: $1" >&2; usage; exit 2 ;;
    esac
done

BIN_DIR="$PREFIX/bin"
TUI_BIN="$PROJECT_DIR/vitriol-tui/target/release/vitriol-tui"

complete_list() {
    echo "$BIN_DIR/vitriol"
    echo "$BIN_DIR/vitriol-tui"
}

if [[ "$UNINSTALL" == "1" ]]; then
    echo "[install] removing symlinks:"
    while read -r link; do
        if [[ -L "$link" ]]; then
            echo "  rm $link"
            rm -f "$link"
        fi
    done < <(complete_list)
    echo "[install] done. vitriol-tui binary left at $TUI_BIN (remove with rm)."
    exit 0
fi

# 1. ensure bin dir
mkdir -p "$BIN_DIR"

# 2. build TUI unless skipped
if [[ "$NO_BUILD" != "1" ]]; then
    if [[ ! -d "$PROJECT_DIR/vitriol-tui" ]]; then
        echo "install: no vitriol-tui/ in $PROJECT_DIR" >&2
        exit 1
    fi
    echo "install: building vitriol-tui (release)…"
    (cd "$PROJECT_DIR/vitriol-tui" && cargo build --release)
else
    echo "install: --no-build, skipping cargo build"
fi

# 3. install symlinks
echo "install: installing into $BIN_DIR"
ln -sfn "$PROJECT_DIR/scripts/vitriol" "$BIN_DIR/vitriol"
if [[ -x "$TUI_BIN" ]]; then
    ln -sfn "$TUI_BIN" "$BIN_DIR/vitriol-tui"
else
    echo "install: WARN no vitriol-tui binary at $TUI_BIN (skipped; run without --no-build)"
fi

# 4. scaffold ~/.vitriol — config is a single INI FILE (not a dir); profiles is a
#    directory of named profiles. Touch only what is missing; never overwrite the
#    existing config the user may already have.
VITRIOL_HOME="${HOME:-$HOME}/.vitriol"
mkdir -p "$VITRIOL_HOME/profiles"
if [[ ! -e "$VITRIOL_HOME/config" ]]; then
    : > "$VITRIOL_HOME/config"
fi

# 5. PATH hint
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "[install] note: $BIN_DIR is not on your PATH."
       echo "          add one of:  export PATH=\"$BIN_DIR:\$PATH\"   (in ~/.bashrc)" ;;
esac

echo "[install] done. →"
echo "  vitriol        $(command -v vitriol 2>/dev/null || echo "$BIN_DIR/vitriol")"
echo "  vitriol-tui    $BIN_DIR/vitriol-tui"
echo "  next: run 'vitriol' (opens the dashboard)"