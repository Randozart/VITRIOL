#!/bin/bash
# VITRIOL Stack Launch Script
# Uses /mnt/data/ai for all large files (79GB Linux partition)

set -e

# Set TMPDIR to /mnt/data/ai/tmp for PyInstaller
export TMPDIR=/mnt/data/ai/tmp
mkdir -p $TMPDIR

echo "=== VITRIOL Stack Launch ==="
echo "Using /mnt/data/ai for models, swap, and temp files"
echo ""

# Kill any existing instances
pkill -9 -f koboldcpp 2>/dev/null || true
pkill -9 -f vitriol_shim.py 2>/dev/null || true
sleep 2

# Start KoboldCPP with context offloading optimizations
echo "1. Starting KoboldCPP on port 5001..."
cd /mnt/data/ai/koboldcpp
nohup ./koboldcpp \
    --model Qwen_Qwen3.5-9B-Q4_K_M.gguf \
    --usecuda \
    --gpulayers 25 \
    --contextsize 8192 \
    --lowvram \
    --usemmap \
    --smartcache 4096 \
    --smartcontext \
    --port 5001 > /tmp/kobold.log 2>&1 &
KOBOLD_PID=$!
echo "   KoboldCPP started (PID: $KOBOLD_PID)"
echo "   Context offloading: --smartcache 4096 + --smartcontext enabled"

# Wait for Kobold to load
echo "   Waiting for model to load (30-60 seconds)..."
for i in {1..30}; do
    if curl -s http://localhost:5001/v1/chat/completions -H "Content-Type: application/json" -d '{"messages":[{"role":"user","content":"hi"}],"max_tokens":5}' > /dev/null 2>&1; then
        echo "   ✓ KoboldCPP is ready!"
        break
    fi
    sleep 2
done

# Check if Kobold is running
if ! curl -s http://localhost:5001/v1/chat/completions -H "Content-Type: application/json" -d '{"messages":[{"role":"user","content":"hi"}],"max_tokens":5}' > /dev/null 2>&1; then
    echo "   ✗ KoboldCPP failed to start. Check /tmp/kobold.log"
    tail -20 /tmp/kobold.log
    exit 1
fi

# Start VITRIOL Shim
echo ""
echo "2. Starting VITRIOL Context Rectifier on port 5010..."
cd ~/Desktop/Projects/linux-pipe-module
nohup python3 libvitriol/vitriol_shim.py > /tmp/vitriol_shim.log 2>&1 &
SHIM_PID=$!
echo "   VITRIOL shim started (PID: $SHIM_PID)"
sleep 3

# Test VITRIOL
if curl -s http://localhost:5010/health > /dev/null 2>&1; then
    echo "   ✓ VITRIOL shim is ready!"
else
    echo "   ✗ VITRIOL shim failed to start. Check /tmp/vitriol_shim.log"
    exit 1
fi

echo ""
echo "=== VITRIOL Stack Ready ==="
echo ""
echo "Services:"
echo "  KoboldCPP: http://localhost:5001 (PID: $KOBOLD_PID)"
echo "  VITRIOL:   http://localhost:5010 (PID: $SHIM_PID)"
echo ""
echo "Storage:"
echo "  Models:    /mnt/data/ai/koboldcpp/"
echo "  Swap:      /mnt/data/ai/swap/ (16GB)"
echo "  Temp:      $TMPDIR"
echo "  Archives:  /mnt/data/ai/vitriol/"
echo ""
echo "Context Offloading Strategy:"
echo "  - Model: Loaded in VRAM (5.5GB)"
echo "  - KV Cache: Smart cached (4096 tokens in VRAM)"
echo "  - Old Context: Auto-archived to SSD"
echo "  - Active Context: Last 4 messages + system prompt"
echo ""
echo "API Endpoints:"
echo "  POST /v1/chat/completions - Inference with rectification"
echo "  POST /context/archive     - Manually archive context to SSD"
echo "  GET  /context/retrieve    - Retrieve archived context"
echo "  GET  /health              - Status check"
echo ""
echo "Test commands:"
echo "  curl http://localhost:5010/health"
echo "  python3 test_shim.py"
echo ""
echo "Point OpenCode to: http://localhost:5010/v1/chat/completions"
echo ""
echo "To stop: pkill -f koboldcpp; pkill -f vitriol_shim.py"
