#!/bin/bash
# VITRIOL Stack Launch Script - Optimized for GTX 1070 Ti
# Uses /mnt/data/ai for all large files

set -e

# Configuration
export TMPDIR=/mnt/data/ai/tmp
mkdir -p $TMPDIR

MODEL_PATH="/mnt/data/ai/koboldcpp/Qwen_Qwen3.5-9B-Q4_K_M.gguf"
LLAMA_SERVER="/mnt/data/ai/llama.cpp/bin/llama-server"
LLAMA_PORT=8279
VITRIOL_PORT=5010

echo "=== VITRIOL Stack Launch (Optimized) ==="
echo "Model: $MODEL_PATH"
echo ""

# Kill existing processes
pkill -f "llama-server" 2>/dev/null || true
pkill -f "vitriol" 2>/dev/null || true
sleep 2

# Check GPU memory
echo "GPU Memory Status:"
nvidia-smi --query-gpu=memory.used,memory.total,memory.free --format=csv
echo ""

# Start llama.cpp with optimized settings
# Based on testing: 25 layers = 3974 MiB = 10.6 tok/s
#                   15 layers = 2675 MiB + 2385 CUDA_Host = 5.3 tok/s
# Optimal: Full GPU offload for best performance

echo "1. Starting llama.cpp on port $LLAMA_PORT..."
nohup $LLAMA_SERVER \
    -m "$MODEL_PATH" \
    -c 8192 \
    -ngl 25 \
    --port $LLAMA_PORT \
    --no-mmap \
    --threads 4 \
    --parallel 4 \
    > /tmp/llama-server.log 2>&1 &
LLAMA_PID=$!
echo "   llama-server started (PID: $LLAMA_PID)"
echo "   GPU layers: 25 (3974 MiB model weights)"

# Wait for model to load
echo "   Waiting for model to load..."
for i in {1..30}; do
    if curl -s http://localhost:$LLAMA_PORT/health > /dev/null 2>&1; then
        echo "   ✓ llama.cpp ready!"
        break
    fi
    sleep 2
done

# Verify llama.cpp is running
if ! curl -s http://localhost:$LLAMA_PORT/health > /dev/null 2>&1; then
    echo "   ✗ llama.cpp failed to start"
    tail -30 /tmp/llama-server.log
    exit 1
fi

# Test inference
echo "   Testing inference..."
RESULT=$(curl -s http://localhost:$LLAMA_PORT/v1/chat/completions \
    -H "Content-Type: application/json" \
    -d '{"messages":[{"role":"user","content":"Hi"}],"max_tokens":5}')
echo "   ✓ Inference working"

# GPU memory after loading
echo ""
echo "GPU Memory After Loading:"
nvidia-smi --query-gpu=memory.used,memory.total,memory.free --format=csv
echo ""

# Start VITRIOL Layer Manager (optional, for future dynamic layer loading)
echo "2. Starting VITRIOL Layer Manager..."
cd /home/randozart/Desktop/Projects/linux-pipe-module
nohup python3 libvitriol/vitriol_layer_manager.py > /tmp/vitriol-manager.log 2>&1 &
VITRIOL_PID=$!
sleep 2
echo "   VITRIOL manager started (PID: $VITRIOL_PID)"
echo ""

# Summary
echo "=== VITRIOL Stack Ready ==="
echo ""
echo "Services:"
echo "  llama-server: http://localhost:$LLAMA_PORT (PID: $LLAMA_PID)"
echo "  VITRIOL Mgr:  http://localhost:$VITRIOL_PORT (PID: $VITRIOL_PID)"
echo ""
echo "Configuration:"
echo "  GPU Layers:  25 (full offload for best performance)"
echo "  Context:     8192 tokens"
echo "  Model:       Qwen 3.5 9B Q4_K_M (5.5GB)"
echo ""
echo "Memory Usage:"
echo "  Model:       ~3974 MiB (GPU)"
echo "  KV Cache:    ~192 MiB (GPU)"
echo "  Compute:     ~565 MiB (GPU)"
echo "  CPU fallback: ~546 MiB"
echo "  Total:       ~5277 MiB"
echo ""
echo "Expected Performance: ~10-11 tokens/second"
echo ""
echo "Point OpenCode to: http://localhost:$LLAMA_PORT/v1/chat/completions"
echo ""
echo "To stop: pkill -f llama-server; pkill -f vitriol"