#!/bin/bash
# VITRIOL Benchmark Script
# Compares performance of different VITRIOL modes

set -e

# --- Configuration (overridable via environment) ---
: "${VITRIOL_MODEL_DIR:=/mnt/data/ai/koboldcpp}"
: "${VITRIOL_LLAMA_DIR:=/mnt/data/ai/llama.cpp}"

MODEL_PATH="${VITRIOL_MODEL_DIR}/Qwen_Qwen3.5-9B-Q4_K_M.gguf"
LLAMA_SERVER="${VITRIOL_LLAMA_DIR}/bin/llama-server"
PORT_BASE=5000

# Only use primary GPU (override with VITRIOL_GPU)
export CUDA_VISIBLE_DEVICES="${VITRIOL_GPU:-0}"

echo "=== VITRIOL Benchmark Suite ==="
echo "Model: $MODEL_PATH"
echo "Using GPU: GTX 1070 Ti only (CUDA_VISIBLE_DEVICES=0)"
echo ""

# Function to run a test
run_test() {
    local mode=$1
    local port=$2
    local desc=$3
    
    echo "----------------------------------------"
    echo "Test: $mode - $desc"
    echo "----------------------------------------"
    
    # Kill any existing server
    pkill -f "llama-server.*$port" 2>/dev/null || true
    sleep 1
    
    # Start server with mode
    case $mode in
        "disabled")
            VITRIOL_MODE=disabled $LLAMA_SERVER -m "$MODEL_PATH" -ngl 25 --port $port --no-mmap &
            ;;
        "sync")
            VITRIOL_MODE=sync $LLAMA_SERVER -m "$MODEL_PATH" -ngl 25 --port $port --no-mmap &
            ;;
        "async")
            VITRIOL_MODE=async VITRIOL_ASYNC_PREFETCH=1 $LLAMA_SERVER -m "$MODEL_PATH" -ngl 25 --port $port --no-mmap &
            ;;
        "stream")
            VITRIOL_MODE=stream $LLAMA_SERVER -m "$MODEL_PATH" -ngl 15 --port $port --no-mmap &
            ;;
    esac
    
    # Wait for load
    sleep 20
    
    # Check if ready
    if ! curl -s http://localhost:$port/health > /dev/null 2>&1; then
        echo "FAILED: Server did not start"
        return 1
    fi
    
    echo "Running inference test..."
    
    # Run multiple prompts and measure
    local total_time=0
    local tokens_generated=0
    
    for i in {1..3}; do
        local start=$(date +%s.%N)
        local result=$(curl -s http://localhost:$port/v1/chat/completions \
            -H "Content-Type: application/json" \
            -d "{\"messages\":[{\"role\":\"user\",\"content\":\"Write a short story about a robot\"}],\"max_tokens\":50}")
        local end=$(date +%s.%N)
        
        local elapsed=$(echo "$end - $start" | bc)
        local toks=$(echo "$result" | grep -o '"total_tokens":[0-9]*' | cut -d: -f2)
        
        echo "  Run $i: ${elapsed}s, $toks tokens"
        total_time=$(echo "$total_time + $elapsed" | bc)
        tokens_generated=$((tokens_generated + toks))
    done
    
    local avg_time=$(echo "scale=2; $total_time / 3" | bc)
    local avg_tokens=$((tokens_generated / 3))
    local tps=$(echo "scale=2; $avg_tokens / $avg_time" | bc)
    
    echo ""
    echo "Results:"
    echo "  Average time: ${avg_time}s"
    echo "  Average tokens: $avg_tokens"
    echo "  Tokens/sec: $tps"
    echo ""
    
    # Stop server
    pkill -f "llama-server.*$port" 2>/dev/null || true
    sleep 2
    
    # Store results
    echo "$mode,$avg_time,$avg_tokens,$tps" >> /tmp/vitriol_results.csv
}

# Clean up previous results
rm -f /tmp/vitriol_results.csv
echo "mode,avg_time,avg_tokens,tps" > /tmp/vitriol_results.csv

# Run tests
run_test "disabled" $((PORT_BASE + 1)) "Baseline (no VITRIOL)"
run_test "sync" $((PORT_BASE + 2)) "Sync mode (preload all)"
run_test "async" $((PORT_BASE + 3)) "Async double-buffer prefetch"
run_test "stream" $((PORT_BASE + 4)) "Stream mode (on-demand)"

# Print summary
echo "========================================"
echo "SUMMARY"
echo "========================================"
cat /tmp/vitriol_results.csv | column -t -s','
echo "========================================"

# Cleanup
rm -f /tmp/vitriol_results.csv
pkill -f "llama-server" 2>/dev/null || true

echo ""
echo "Benchmark complete!"