#!/bin/bash
# Test VITRIOL Expert Cache with Qwen3.6-35B-A3B

set -e

# --- Configuration (overridable via environment) ---
: "${VITRIOL_MODEL_DIR:=/mnt/data/ai/koboldcpp}"
: "${VITRIOL_LLAMA_DIR:=/mnt/data/ai/llama.cpp}"

MODEL_PATH="${VITRIOL_MODEL_DIR}/Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf"
LLAMA_SERVER="${VITRIOL_LLAMA_DIR}/bin/llama-server"
PORT="${VITRIOL_PORT:-8279}"

export CUDA_VISIBLE_DEVICES="${VITRIOL_GPU:-0}"

echo "=== VITRIOL Expert Cache Test ==="

# Kill any existing server
pkill -f "llama-server" 2>/dev/null || true
sleep 1

# Test 1: Try running Qwen3.6-35B-A3B with expert override to CPU
# This forces experts to stay on CPU, reducing VRAM usage
echo ""
echo "Test 1: Running with experts on CPU (lower VRAM)..."
echo ""

CUDA_VISIBLE_DEVICES=0 /mnt/data/ai/llama.cpp/bin/llama-server \
    -m /mnt/data/ai/koboldcpp/Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf \
    -ngl 20 \
    -ot ".*exps.*=CPU" \
    --port 8279 \
    --no-mmap \
    2>&1 | head -50 &

# Wait for load
sleep 60

# Check if server started
if curl -s http://localhost:8279/health > /dev/null 2>&1; then

    RESULT=$(curl -s http://localhost:8279/v1/chat/completions \
        -H "Content-Type: application/json" \
        -d '{"messages":[{"role":"user","content":"Hello"}],"max_tokens":30}')
    
    echo "Result: $RESULT"
else
    echo "Server failed to start"
fi

echo ""
echo "Test complete"