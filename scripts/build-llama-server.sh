#!/usr/bin/env bash
set -euo pipefail

# Build llama.cpp with VITRIOL patches
# Usage: ./scripts/build-llama-server.sh [OPTIONS] [llama.cpp directory]
#
# Options:
#   --backend <cuda|sycl|vulkan|auto>  Backend to build (default: auto-detect)
#   --clean                             Clean build directory before building
#
# Examples:
#   ./scripts/build-llama-server.sh                        # Auto-detect hardware
#   ./scripts/build-llama-server.sh --backend vulkan       # Intel Vulkan
#   ./scripts/build-llama-server.sh --backend sycl         # Intel SYCL (needs oneAPI)
#   ./scripts/build-llama-server.sh --backend cuda         # NVIDIA CUDA

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VITRIOL_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Parse arguments
BACKEND=""
CLEAN_BUILD=false
LLAMA_DIR=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --backend)
            BACKEND="$2"
            shift 2
            ;;
        --clean)
            CLEAN_BUILD=true
            shift
            ;;
        -*)
            echo "Unknown option: $1"
            exit 1
            ;;
        *)
            LLAMA_DIR="$1"
            shift
            ;;
    esac
done

LLAMA_DIR="${LLAMA_DIR:-$VITRIOL_ROOT/llama.cpp}"
BUILD_DIR="$LLAMA_DIR/build"

if [ ! -d "$LLAMA_DIR" ]; then
    echo "Error: llama.cpp directory not found at $LLAMA_DIR"
    echo "Usage: $0 [--backend cuda|sycl|vulkan|auto] [--clean] [llama.cpp directory]"
    exit 1
fi

# Auto-detect backend if not specified
if [ -z "$BACKEND" ]; then
    if command -v nvidia-smi &>/dev/null && nvidia-smi &>/dev/null 2>&1; then
        BACKEND="cuda"
        echo "Auto-detected: NVIDIA GPU(s) — building with CUDA"
    elif [ -d "/dev/accel" ] && ls /dev/accel/accel* &>/dev/null 2>&1; then
        # Check for Intel GPU first (SYCL preferred over NPU-only)
        if [ -d "/sys/class/drm" ] && ls /sys/class/drm/card*/device/vendor &>/dev/null 2>&1; then
            if grep -q "0x8086" /sys/class/drm/card*/device/vendor 2>/dev/null; then
                # Intel GPU found — prefer SYCL if oneAPI available
                if [ -n "${ONEAPI_ROOT:-}" ] || [ -d "/opt/intel/oneapi" ]; then
                    BACKEND="sycl"
                    echo "Auto-detected: Intel GPU + oneAPI — building with SYCL"
                else
                    BACKEND="vulkan"
                    echo "Auto-detected: Intel GPU (no oneAPI) — building with Vulkan"
                fi
            else
                BACKEND="vulkan"
                echo "Auto-detected: Non-Intel GPU — building with Vulkan"
            fi
        else
            BACKEND="vulkan"
            echo "Auto-detected: NPU present, no GPU — building with Vulkan"
        fi
    else
        BACKEND="vulkan"
        echo "No GPU detected — building with Vulkan (CPU fallback)"
    fi
fi

echo "Building llama.cpp with backend=$BACKEND at: $LLAMA_DIR"

# Apply patches first if VITRIOL files aren't present (CUDA only)
if [ "$BACKEND" = "cuda" ] && [ ! -f "$LLAMA_DIR/ggml/src/ggml-cuda/vitriol-cuda-integration.cpp" ]; then
    echo "VITRIOL source files not found. Applying patches..."
    "$SCRIPT_DIR/apply-llama-patches.sh" "$LLAMA_DIR"
fi

# Clean build if requested
if [ "$CLEAN_BUILD" = true ] && [ -d "$BUILD_DIR" ]; then
    echo "Cleaning build directory..."
    rm -rf "$BUILD_DIR"
fi

# Create build directory
mkdir -p "$BUILD_DIR"

# Configure and build
cd "$BUILD_DIR"
# CMAKE_POSITION_INDEPENDENT_CODE=ON is required — the vendored
# cpp-httplib static lib is linked into libllama-common.so and a clean build
# fails with "relocation R_X86_64_TPOFF32 ... recompile with -fPIC" without it.

CMAKE_ARGS="-DCMAKE_BUILD_TYPE=Release -DCMAKE_POSITION_INDEPENDENT_CODE=ON"

case "$BACKEND" in
    cuda)
        CMAKE_ARGS="$CMAKE_ARGS -DGGML_CUDA=ON"
        # Detect CUDA architectures from available GPUs
        if command -v nvidia-smi &>/dev/null; then
            # Default to common architectures; override with env if needed
            CUDA_ARCHS="${CUDA_ARCHITECTURES:-61;86}"
            CMAKE_ARGS="$CMAKE_ARGS -DCMAKE_CUDA_ARCHITECTURES=$CUDA_ARCHS"
            echo "CUDA architectures: $CUDA_ARCHS"
        fi
        ;;
    sycl)
        # Source oneAPI environment if available
        if [ -f "/opt/intel/oneapi/setvars.sh" ]; then
            echo "Sourcing oneAPI environment..."
            source /opt/intel/oneapi/setvars.sh --force
        elif [ -z "${ONEAPI_ROOT:-}" ]; then
            echo "Error: oneAPI not found. Install the Intel oneAPI packages."
            echo "  sudo pacman -S intel-oneapi-dpcpp-cpp intel-oneapi-mkl intel-oneapi-mkl-sycl"
            exit 1
        fi
        CMAKE_ARGS="$CMAKE_ARGS -DGGML_SYCL=ON -DGGML_SYCL_TARGET=INTEL"
        # F16 build: better prompt processing, XMX tensor-core paths (2026-09-04 bench)
        CMAKE_ARGS="$CMAKE_ARGS -DGGML_SYCL_F16=ON"
        # Explicit DPC++ compilers — cmake defaults to gcc which cannot build SYCL
        CMAKE_ARGS="$CMAKE_ARGS -DCMAKE_C_COMPILER=icx -DCMAKE_CXX_COMPILER=icpx"
        # Optional SYCL optimizations
        CMAKE_ARGS="$CMAKE_ARGS -DGGML_SYCL_GRAPH=ON -DGGML_SYCL_HOST_MEM_FALLBACK=ON"
        # Native ISA for CPU fallback paths (AVX2 + AVX-VNNI on Panther Lake)
        if [ -z "${SYCL_NO_NATIVE:-}" ]; then
            CMAKE_ARGS="$CMAKE_ARGS -DCMAKE_C_FLAGS=-march=native -DCMAKE_CXX_FLAGS=-march=native"
        fi
        ;;
    vulkan)
        CMAKE_ARGS="$CMAKE_ARGS -DGGML_VULKAN=ON"
        # cmake 4.4+ FindVulkan may fail to auto-detect paths; help it
        if [ -z "${Vulkan_INCLUDE_DIR:-}" ] && [ -d "/usr/include/vulkan" ]; then
            CMAKE_ARGS="$CMAKE_ARGS -DVulkan_INCLUDE_DIR=/usr/include"
        fi
        if [ -z "${Vulkan_LIBRARY:-}" ] && [ -f "/usr/lib/libvulkan.so" ]; then
            CMAKE_ARGS="$CMAKE_ARGS -DVulkan_LIBRARY=/usr/lib/libvulkan.so"
        fi
        ;;
    *)
        echo "Error: Unknown backend '$BACKEND'. Use: cuda, sycl, or vulkan"
        exit 1
        ;;
esac

echo "CMake args: $CMAKE_ARGS"
cmake .. $CMAKE_ARGS
make -j"$(nproc)" llama-server

echo ""
echo "Build complete."
echo "Server: $BUILD_DIR/bin/llama-server"
case "$BACKEND" in
    cuda)
        echo "CUDA lib: $BUILD_DIR/bin/libggml-cuda.so"
        ;;
    sycl)
        echo "SYCL lib: $BUILD_DIR/bin/libggml-sycl.so"
        ;;
    vulkan)
        echo "Vulkan lib: $BUILD_DIR/bin/libggml-vulkan.so"
        ;;
esac

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
echo "  backend=$BACKEND"
case "$BACKEND" in
    cuda)
        echo "  source $VITRIOL_ROOT/vitriol.env"
        echo "  CUDA_VISIBLE_DEVICES=\"\${VITRIOL_GPU:-0}\" \"$BUILD_DIR/bin/llama-server\" \\"
        echo "      -m \"\$VITRIOL_MODEL_DIR/Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf\" \\"
        echo "      -ngl 20 -ot \".*exps.*=CPU\" --port \"\${VITRIOL_PORT:-8279}\" --no-mmap"
        ;;
    sycl)
        echo "  source /opt/intel/oneapi/setvars.sh"
        echo "  \"$BUILD_DIR/bin/llama-server\" \\"
        echo "      -m \"\$VITRIOL_MODEL_DIR/Qwen3.8-27B-Q4_K_M.gguf\" \\"
        echo "      -ngl 99 --port 8080"
        ;;
    vulkan)
        echo "  \"$BUILD_DIR/bin/llama-server\" \\"
        echo "      -m \"\$VITRIOL_MODEL_DIR/Qwen3.8-27B-Q4_K_M.gguf\" \\"
        echo "      -ngl 99 --port 8080"
        ;;
esac
