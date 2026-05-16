#!/bin/bash
# test_vitriol.sh - Test harness for VITRIOL
#
# Usage:
#   ./test_vitriol.sh stub          - Test stub module (SAFE)
#   ./test_vitriol.sh socket         - Test socket API (requires daemon)
#   ./test_vitriol.sh client         - Test Python client
#   ./test_vitriol.sh infer          - Test inference (requires llama.cpp)
#   ./test_vitriol.sh help           - Show help

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KO_MODULE="${SCRIPT_DIR}/vitriol_new_ffi.ko"
DAEMON_PID=""
EXPECTED_LOG="VITRIOL:"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_fail() { echo -e "${RED}[FAIL]${NC} $1"; }
log_step() { echo -e "${BLUE}[STEP]${NC} $1"; }

check_root() {
    if [[ $EUID -ne 0 ]]; then
        log_fail "This script must be run as root"
        exit 1
    fi
}

check_module_exists() {
    if [[ ! -f "$KO_MODULE" ]]; then
        log_fail "Module not found: $KO_MODULE"
        log_info "Build with: make"
        exit 1
    fi
}

unload_module() {
    log_info "Unloading module..."
    rmmod vitriol_new_ffi 2>/dev/null || true
}

# ==============================================================================
# Test: Stub Mode (SAFE - no real hardware)
# ==============================================================================

test_stub_mode() {
    log_step "Testing STUB mode (safe - no hardware access)..."

    unload_module

    log_info "Clearing dmesg..."
    dmesg -c > /dev/null

    log_info "Loading module with test_mode=1..."
    insmod "$KO_MODULE" test_mode=1 || {
        log_fail "Failed to load module"
        dmesg | tail -20
        exit 1
    }

    sleep 1

    log_info "Checking dmesg for VITRIOL output..."
    OUTPUT=$(dmesg | grep "$EXPECTED_LOG")

    if [[ -z "$OUTPUT" ]]; then
        log_fail "No VITRIOL output found in dmesg"
        log_info "Full dmesg output:"
        dmesg | tail -30
        unload_module
        exit 1
    fi

    log_info "SUCCESS: Module loaded and produced expected output"
    echo ""
    echo "=== dmesg output ==="
    echo "$OUTPUT"
    echo "===================="
    echo ""

    unload_module
    log_info "Stub mode test PASSED"
}

# ==============================================================================
# Test: Socket API (requires vitriol-daemon)
# ==============================================================================

test_socket_api() {
    log_step "Testing Socket API..."

    check_daemon() {
        if [[ ! -f "${SCRIPT_DIR}/vitriol-daemon/target/debug/vitriol-daemon" ]]; then
            log_fail "Daemon not built. Run: cd vitriol-daemon && cargo build"
            return 1
        fi
    }

    if ! check_daemon; then
        log_warn "Skipping socket test - daemon not built"
        return 1
    fi

    # Start daemon
    log_info "Starting vitriol-daemon..."
    cd "${SCRIPT_DIR}/vitriol-daemon"
    RUST_LOG=info ./target/debug/vitriol-daemon &
    DAEMON_PID=$!
    sleep 2

    # Check if daemon is running
    if ! kill -0 $DAEMON_PID 2>/dev/null; then
        log_fail "Daemon failed to start"
        exit 1
    fi

    log_info "Daemon started with PID $DAEMON_PID"

    # Test with Python client
    cd "${SCRIPT_DIR}"
    log_info "Testing STATUS command..."
    python3 -c "
from libvitriol import VitriolClient
try:
    with VitriolClient() as c:
        status = c.get_status()
        print('STATUS:', status)
        print('GPU present:', status.get('gpu_present', 'N/A'))
except Exception as e:
    print('Error:', e)
    exit(1)
"

    # Test PING
    log_info "Testing PING command..."
    python3 -c "
from libvitriol import VitriolClient
try:
    with VitriolClient() as c:
        resp = c._send_request('PING')
        print('PING response:', resp)
except Exception as e:
    print('Error:', e)
    exit(1)
"

    # Stop daemon
    log_info "Stopping daemon..."
    kill $DAEMON_PID 2>/dev/null || true

    log_info "Socket API test PASSED"
}

# ==============================================================================
# Test: Python Client
# ==============================================================================

test_python_client() {
    log_step "Testing Python Client Library..."

    cd "${SCRIPT_DIR}"

    # Test imports
    log_info "Testing imports..."
    python3 -c "
from libvitriol import VitriolClient, VitriolStatus
print('Import test PASSED')
"

    # Test VitriolError
    log_info "Testing error handling..."
    python3 -c "
from libvitriol.client import VitriolError
e = VitriolError('TEST_CODE', 'Test message')
assert e.code == 'TEST_CODE'
assert e.message == 'Test message'
print('Error handling test PASSED')
"

    log_info "Python Client test PASSED"
}

# ==============================================================================
# Test: Inference (requires llama.cpp integration)
# ==============================================================================

test_inference() {
    log_step "Testing Inference..."

    log_warn "Inference test requires llama.cpp integration"
    log_warn "This is Phase 1 next step - not yet implemented"

    # Check if model exists
    MODEL_PATH="/home/randozart/Downloads/Qwen_Qwen3.5-9B-Q4_K_M.gguf"
    if [[ ! -f "$MODEL_PATH" ]]; then
        log_fail "Model not found: $MODEL_PATH"
        log_info "Please download Qwen3.5 9B Q4_K_M"
        exit 1
    fi

    log_info "Model found: $MODEL_PATH"
    log_info "llama.cpp integration is the next implementation step"
    log_info "See IMPLEMENTATION_PLAN.md for details"
}

# ==============================================================================
# Test: Full Stack (requires all components)
# ==============================================================================

test_full_stack() {
    log_step "Testing Full Stack..."

    log_info "This test requires:"
    log_info "  1. vitriol.ko loaded (test_mode=1)"
    log_info "  2. vitriol-daemon running"
    log_info "  3. llama.cpp integrated"

    log_warn "Full stack test is Phase 1 target - not yet complete"
}

# ==============================================================================
# Help
# =============================================================================

show_help() {
    echo "VITRIOL Test Harness"
    echo ""
    echo "Usage: $0 <test> [options]"
    echo ""
    echo "Tests:"
    echo "  stub       - Test stub module (SAFE - no hardware)"
    echo "  socket     - Test socket API (requires daemon)"
    echo "  client     - Test Python client library"
    echo "  infer      - Test inference (requires llama.cpp)"
    echo "  full       - Test full stack"
    echo "  help       - Show this help"
    echo ""
    echo "Examples:"
    echo "  $0 stub           # Safe stub test"
    echo "  $0 socket        # Socket API test"
    echo "  $0 infer         # Inference test"
    echo ""
    echo "Safety:"
    echo "  ALWAYS test stub mode first before any hardware access"
    echo "  For your primary machine, always use test_mode=1"
}

# ==============================================================================
# Main
# ==============================================================================

main() {
    check_root
    check_module_exists

    MODE="${1:-stub}"

    case "$MODE" in
        stub)
            test_stub_mode
            ;;
        socket)
            test_socket_api
            ;;
        client)
            test_python_client
            ;;
        infer)
            test_inference
            ;;
        full)
            test_full_stack
            ;;
        help|--help|-h)
            show_help
            ;;
        *)
            log_fail "Unknown test mode: $MODE"
            show_help
            exit 1
            ;;
    esac
}

main "$@"
