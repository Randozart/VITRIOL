#!/bin/bash
# Simple baseline test - no VITRIOL environment variables

set -e

export CUDA_VISIBLE_DEVICES=0

MODEL_PATH="/mnt/data/ai/koboldcpp/Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf"
LLAMA_SERVER="/mnt/data/ai/llama.cpp/bin/llama-server"
PORT=8279

echo "=== Simple Baseline Test ==="

# Kill any existing server
pkill -f "llama-server.*$PORT" 2>/dev/null || true
sleep 2

# Start server WITHOUT any VITRIOL variables
echo "Starting llama-server..."
$LLAMA_SERVER \
    -m "$MODEL_PATH" \
    -ngl 25 \
    --port $PORT \
    --no-mmap \
    &

# Wait for server to be ready
echo "Waiting for server to load..."
sleep 30

# Check if server is ready
if ! curl -s http://localhost:$PORT/health > /dev/null 2>&1; then
    echo "ERROR: Server did not start"
    exit 1
fi

echo "Server ready. Running inference test..."

# Run inference
START=$(date +%s.%N)
RESULT=$(curl -s http://localhost:$PORT/v1/chat/completions \
    -H "Content-Type: application/json" \
    -d '{"messages":[{"role":"user","content":"Write a short story about a robot in 2 sentences."}],"max_tokens":50}')
END=$(date +%s.%N)

ELAPSED=$(echo "$END - $START" | bc)

# Extract token count
TOKENS=$(echo "$RESULT" | grep -o '"total_tokens":[0-9]*' | cut -d: -f2)

if [ -z "$TOKENS" ] || [ "$TOKENS" = "0" ]; then
    echo "ERROR: No tokens generated"
    echo "Response: $RESULT"
    exit 1
fi

TPS=$(echo "scale=2; $TOKENS / $ELAPSED" | bc)

echo ""
echo "=== RESULTS ==="
echo "Time: ${ELAPSED}s"
echo "Tokens: $TOKENS"
echo "Speed: ${TPS} tok/s"
echo "=============="

# Cleanup
pkill -f "llama-server.*$PORT" 2>/dev/null || true