#!/bin/bash
# VITRIOL Baseline Test Script
# Tests llama.cpp with different GPU layer configurations

set -e

MODEL_PATH="/mnt/data/ai/koboldcpp/Qwen_Qwen3.5-9B-Q4_K_M.gguf"
LLAMA_SERVER="/mnt/data/ai/llama.cpp/bin/llama-server"
PORT=5003

echo "=== VITRIOL Baseline Test ==="
echo "Model: $MODEL_PATH"

# Kill any existing processes
pkill -f "llama-server.*$PORT" 2>/dev/null || true
sleep 1

# Test 1: Baseline - full GPU offload (25 layers)
echo ""
echo "Test 1: Full GPU offload (25 layers)"
$LLAMA_SERVER \
    -m "$MODEL_PATH" \
    -c 4096 \
    -ngl 25 \
    --port $PORT \
    --no-mmap \
    --threads 4 \
    > /tmp/vitriol_test1.log 2>&1 &
PID1=$!
echo "Started llama-server (PID: $PID1)"

sleep 15
if curl -s http://localhost:$PORT/health > /dev/null 2>&1; then
    echo "Server ready, testing inference..."
    curl -s http://localhost:$PORT/v1/chat/completions \
        -H "Content-Type: application/json" \
        -d '{"messages":[{"role":"user","content":"What is 2+2?"}],"max_tokens":20}' \
        | jq -r '.choices[0].message.content' 2>/dev/null | head -c 200
    echo ""
else
    echo "Server failed to start. Check log:"
    tail -20 /tmp/vitriol_test1.log
fi

# Test 2: Reduced GPU layers (15 layers)
echo ""
echo "Test 2: Partial GPU offload (15 layers)"
pkill -f "llama-server.*$PORT" 2>/dev/null || true
sleep 2

$LLAMA_SERVER \
    -m "$MODEL_PATH" \
    -c 4096 \
    -ngl 15 \
    --port $PORT \
    --no-mmap \
    --threads 4 \
    > /tmp/vitriol_test2.log 2>&1 &
PID2=$!
sleep 15

if curl -s http://localhost:$PORT/health > /dev/null 2>&1; then
    echo "Server ready, testing inference..."
    curl -s http://localhost:$PORT/v1/chat/completions \
        -H "Content-Type: application/json" \
        -d '{"messages":[{"role":"user","content":"What is 2+2?"}],"max_tokens":20}' \
        | jq -r '.choices[0].message.content' 2>/dev/null | head -c 200
    echo ""
else
    echo "Server failed to start."
    tail -10 /tmp/vitriol_test2.log
fi

# Test 3: CPU only (0 layers)
echo ""
echo "Test 3: CPU only (0 GPU layers)"
pkill -f "llama-server.*$PORT" 2>/dev/null || true
sleep 2

$LLAMA_SERVER \
    -m "$MODEL_PATH" \
    -c 2048 \
    -ngl 0 \
    --port $PORT \
    --threads 4 \
    > /tmp/vitriol_test3.log 2>&1 &
PID3=$!
sleep 20

if curl -s http://localhost:$PORT/health > /dev/null 2>&1; then
    echo "Server ready, testing inference (this will be slow)..."
    time curl -s http://localhost:$PORT/v1/chat/completions \
        -H "Content-Type: application/json" \
        -d '{"messages":[{"role":"user","content":"Hi"}],"max_tokens":10}' \
        | jq -r '.choices[0].message.content' 2>/dev/null | head -c 100
    echo ""
else
    echo "Server failed to start."
    tail -10 /tmp/vitriol_test3.log
fi

# Cleanup
pkill -f "llama-server.*$PORT" 2>/dev/null || true

echo ""
echo "=== Baseline Tests Complete ==="
echo "Check logs in /tmp/vitriol_test*.log"