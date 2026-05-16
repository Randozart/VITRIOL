#!/bin/bash
# test_p2p_run.sh — Guide for cooperative P2P DMA test
#
# Usage: ./test_p2p_run.sh [path/to/model.gguf]

set -e

GGUF_PATH="${1:-}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DAEMON_DIR="$SCRIPT_DIR/vitriol-daemon"
EXECUTOR="$SCRIPT_DIR/alka-executor/alka-executor"
TEST_CUDA="$SCRIPT_DIR/test_p2p_dma"
TEST_STREAM="$SCRIPT_DIR/test_p2p.alkas"
VIAL="$SCRIPT_DIR/alka-handoff/gtx960_2gb.alkavl"
DEVICE="/dev/vitriol"

echo "=== VITRIOL Cooperative P2P DMA Test ==="
echo ""

# ── Check prerequisites ──
for f in "$DAEMON_DIR/vitriol.ko" "$TEST_CUDA" "$TEST_STREAM" "$EXECUTOR" "$VIAL"; do
    if [ ! -f "$f" ]; then
        echo "ERROR: Missing $f"
        exit 1
    fi
done

if [ -n "$GGUF_PATH" ] && [ ! -f "$GGUF_PATH" ]; then
    echo "ERROR: GGUF file not found: $GGUF_PATH"
    exit 1
fi

echo "Step 1: Load kernel module"
echo "  sudo insmod $DAEMON_DIR/vitriol.ko"
echo "  sudo chmod 666 $DEVICE"
echo "  dmesg | tail -5 | grep vitriol"
echo "  # Should see: 'nvidia P2P cooperative DMA available'"
echo ""

echo "Step 2: Run CUDA test (Terminal 1)"
if [ -n "$GGUF_PATH" ]; then
    echo "  $TEST_CUDA $GGUF_PATH"
else
    echo "  $TEST_CUDA"
    echo "  # (no GGUF path - will only check buffer is non-zero)"
fi
echo "  # Note the GPU VA it prints"
echo ""

echo "Step 3: Run executor (Terminal 2)"
if [ -n "$GGUF_PATH" ]; then
    echo "  $EXECUTOR $TEST_STREAM $VIAL --cooperative --gpu-va <GPU_VA> --source $GGUF_PATH"
else
    echo "  $EXECUTOR $TEST_STREAM $VIAL --cooperative --gpu-va <GPU_VA>"
    echo "  # (no --source - kernel will zero-fill the buffer)"
fi
echo ""

echo "Step 4: Press Enter in Terminal 1 to verify"
echo ""

echo "Step 5: Cleanup"
echo "  sudo rmmod vitriol"
echo ""

echo "Or run it all at once (you'll need two terminals):"
echo "  sudo insmod $DAEMON_DIR/vitriol.ko && sudo chmod 666 $DEVICE"
if [ -n "$GGUF_PATH" ]; then
    echo "  $TEST_CUDA $GGUF_PATH"
else
    echo "  $TEST_CUDA"
fi
