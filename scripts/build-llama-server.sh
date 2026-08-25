#!/usr/bin/env bash
set -euo pipefail

# Build llama.cpp with CUDA support and VITRIOL patches
# Usage: ./scripts/build-llama-server.sh [llama.cpp directory]

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VITRIOL_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LLAMA_DIR="${1:-$VITRIOL_ROOT/llama.cpp}"
BUILD_DIR="$LLAMA_DIR/build"

if [ ! -d "$LLAMA_DIR" ]; then
    echo "Error: llama.cpp directory not found at $LLAMA_DIR"
    echo "Usage: $0 [llama.cpp directory]"
    exit 1
fi

echo "Building llama.cpp with CUDA at: $LLAMA_DIR"

# Apply patches first if VITRIOL files aren't present
if [ ! -f "$LLAMA_DIR/ggml/src/ggml-cuda/vitriol-cuda-integration.cpp" ]; then
    echo "VITRIOL source files not found. Applying patches..."
    "$SCRIPT_DIR/apply-llama-patches.sh" "$LLAMA_DIR"
fi

# Create build directory
mkdir -p "$BUILD_DIR"

# Configure and build
cd "$BUILD_DIR"
# 2026-08-06: CMAKE_POSITION_INDEPENDENT_CODE=ON is required — the vendored
# cpp-httplib static lib is linked into libllama-common.so and a clean build
# fails with "relocation R_X86_64_TPOFF32 ... recompile with -fPIC" without it.
cmake .. -DGGML_CUDA=ON -DCMAKE_BUILD_TYPE=Release -DCMAKE_POSITION_INDEPENDENT_CODE=ON
make -j"$(nproc)" llama-server

echo ""
echo "Build complete."
echo "Server: $BUILD_DIR/bin/llama-server"
echo "CUDA lib: $BUILD_DIR/bin/libggml-cuda.so"

# ── Best-effort CAP_IPC_LOCK reapply ────────────────────────────────────────
# setcap does not survive recompiles (new inode). 2026-08-24: the cap is
# OPTIONAL on this host — CUDA pinned allocations do not count against
# RLIMIT_MEMLOCK, and every deep-context certification ran uncapped. So this
# is best-effort, never fatal, and 'sudo' is only used non-interactively.
apply_caps() {
    local bin="$1"
    if [[ ! -f "$bin" ]]; then return 0; fi
    if [[ "$(id -u)" == "0" ]]; then
        setcap cap_ipc_lock=+ep "$bin" && echo "CAP_IPC_LOCK set on $bin"
    elif sudo -n setcap cap_ipc_lock=+ep "$bin" 2>/dev/null; then
        echo "CAP_IPC_LOCK set on $bin (via passwordless sudo)"
    else
        echo "NOTE: $bin has no CAP_IPC_LOCK (fine on this host)."
        echo "      To set it: sudo vitriol setup"
    fi
}
apply_caps "$BUILD_DIR/bin/llama-server"
apply_caps "$BUILD_DIR/bin/llama-cli"
echo ""
echo "To run:"
echo "  source $VITRIOL_ROOT/vitriol.env"
echo "  CUDA_VISIBLE_DEVICES=\"\${VITRIOL_GPU:-0}\" \"$BUILD_DIR/bin/llama-server\" \\"
echo "      -m \"\$VITRIOL_MODEL_DIR/Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf\" \\"
echo "      -ngl 20 -ot \".*exps.*=CPU\" --port \"\${VITRIOL_PORT:-8279}\" --no-mmap"
