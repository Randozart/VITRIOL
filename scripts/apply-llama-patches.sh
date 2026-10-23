#!/usr/bin/env bash
set -euo pipefail

# DEPRECATED (2026-10-23 monorepo import): the engine lives in-tree at
# engine/ - patches no longer apply; VITRIOL sources are already present.
# Kept only for reference/rollback to the submodule era.
# Apply VITRIOL patches to llama.cpp submodule
# Usage: ./scripts/apply-llama-patches.sh [llama.cpp directory]

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VITRIOL_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LLAMA_DIR="${1:-$VITRIOL_ROOT/llama.cpp}"
PATCH_DIR="$VITRIOL_ROOT/llama.cpp-patches"

if [ ! -d "$LLAMA_DIR" ]; then
    echo "Error: llama.cpp directory not found at $LLAMA_DIR"
    echo "Usage: $0 [llama.cpp directory]"
    exit 1
fi

if [ ! -d "$PATCH_DIR" ]; then
    echo "Error: Patch directory not found at $PATCH_DIR"
    exit 1
fi

echo "Applying VITRIOL patches to llama.cpp at: $LLAMA_DIR"

# Apply unified diffs
for patch in "$PATCH_DIR"/*.patch; do
    if [ -f "$patch" ]; then
        echo "  Applying: $(basename "$patch")"
        git -C "$LLAMA_DIR" apply --verbose "$(basename "$patch")" 2>/dev/null || \
            (cd "$LLAMA_DIR" && patch -p1 < "$patch")
    fi
done

# Copy source files that aren't covered by patches
if [ -d "$PATCH_DIR/source" ]; then
    echo "  Copying source files..."
    for src in "$PATCH_DIR/source"/*; do
        if [ -f "$src" ]; then
            filename="$(basename "$src")"
            # Determine destination based on filename
            case "$filename" in
                vitriol-cuda-integration.*)
                    dest="$LLAMA_DIR/ggml/src/ggml-cuda/$filename"
                    ;;
                vitriol-config.*)
                    dest="$LLAMA_DIR/src/$filename"
                    ;;
                vitriol-config.h)
                    dest="$LLAMA_DIR/include/$filename"
                    ;;
                vitriol-dma.h)
                    dest="$LLAMA_DIR/include/$filename"
                    ;;
                *)
                    dest="$LLAMA_DIR/$filename"
                    ;;
            esac
            echo "    $filename -> $dest"
            cp "$src" "$dest"
        fi
    done
fi

echo "Done. Patches applied to $LLAMA_DIR"
echo "Next: Run ./scripts/build-llama-server.sh"
