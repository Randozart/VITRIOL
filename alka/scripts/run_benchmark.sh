#!/bin/bash
# VITRIOL Alka Benchmark Runner
# Runs llama.cpp inference benchmarks + Alka mock execution
#
# This script bridges the current state (llama.cpp -ot) with the
# target state (Alka FLOW + DMA). It runs both in parallel:
#   1. llama.cpp benchmark — measures actual tok/s
#   2. Alka mock execution — validates the Recipe logic
#
# When the kernel module (Athanor) is ready, step 2 graduates to
# real hardware execution.

set -e

# Configuration
MODEL_35B="/mnt/data/ai/koboldcpp/Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf"
MODEL_9B="/mnt/data/ai/koboldcpp/Qwen_Qwen3.5-9B-Q4_K_M.gguf"
LLAMA_SERVER="/mnt/data/ai/llama.cpp/bin/llama-server"
PORT=8279
ALKA_BIN="/home/randozart/Desktop/Projects/alka-lang/zig-out/bin/alka"
ALKA_DIR="/home/randozart/Desktop/Projects/VITRIOL/alka"
RESULTS_DIR="/home/randozart/Desktop/Projects/VITRIOL/alka/results"

mkdir -p "$RESULTS_DIR"

# Only use GTX 1070 Ti (device 0), ignore GTX 960
export CUDA_VISIBLE_DEVICES=0

echo "========================================"
echo "  VITRIOL Alka Benchmark Suite"
echo "  $(date)"
echo "========================================"
echo ""

# --- Phase 1: Alka Recipe Compilation & Mock ---
echo "=== Phase 1: Alka Recipe Validation ==="
echo ""

RECIPE="$ALKA_DIR/recipes/benchmark_35b.alka"
VIAL="$ALKA_DIR/vials/vitriol_rig.alkavl"

if [ -f "$ALKA_BIN" ]; then
    echo "Compiling benchmark recipe..."
    $ALKA_BIN "$RECIPE" "$VIAL" 2>&1 || echo "  [WARN] Compilation had issues (expected for unimplemented instructions)"

    ALKAS="${RECIPE}.alkas"
    AZOTH="${RECIPE}.azoth"

    if [ -f "$ALKAS" ]; then
        echo ""
        echo "Running mock execution..."
        $ALKA_BIN --mock "$ALKAS" 2>&1 | tee "$RESULTS_DIR/mock_output.txt"
        echo ""
        echo "Mock execution complete. Output saved to $RESULTS_DIR/mock_output.txt"
    else
        echo "  [SKIP] No .alkas binary produced — recipe may use unsupported instructions"
    fi
else
    echo "  [SKIP] Alka compiler not found at $ALKA_BIN"
fi

echo ""
echo "=== Phase 2: llama.cpp Baseline Benchmark ==="
echo ""

# Function to run a single benchmark
run_benchmark() {
    local name=$1
    local ngl=$2
    local ot_flag=$3
    local desc=$4

    echo "--- $name: $desc ---"

    # Kill any existing server
    pkill -f "llama-server.*$PORT" 2>/dev/null || true
    sleep 2

    # Build command
    local cmd="CUDA_VISIBLE_DEVICES=0 $LLAMA_SERVER -m $MODEL_35B -ngl $ngl --port $PORT --no-mmap"
    if [ -n "$ot_flag" ]; then
        cmd="$cmd $ot_flag"
    fi

    echo "  Starting: $cmd"
    eval "$cmd" > "$RESULTS_DIR/${name}.log" 2>&1 &
    local pid=$!

    # Wait for server to be ready (up to 120s for 35B model)
    echo "  Waiting for server (PID: $pid)..."
    local waited=0
    while [ $waited -lt 120 ]; do
        if curl -s "http://localhost:$PORT/health" > /dev/null 2>&1; then
            echo "  Server ready after ${waited}s"
            break
        fi
        sleep 5
        waited=$((waited + 5))
    done

    if [ $waited -ge 120 ]; then
        echo "  [FAIL] Server did not start within 120s"
        kill $pid 2>/dev/null || true
        return 1
    fi

    # Run inference tests
    local total_tokens=0
    local total_time=0
    local runs=3

    for i in $(seq 1 $runs); do
        local start=$(date +%s.%N)
        local result=$(curl -s "http://localhost:$PORT/v1/chat/completions" \
            -H "Content-Type: application/json" \
            -d "{\"messages\":[{\"role\":\"user\",\"content\":\"Write a short story about a robot that learns to paint\"}],\"max_tokens\":50}")

        local end=$(date +%s.%N)
        local elapsed=$(echo "$end - $start" | bc)

        local toks=$(echo "$result" | grep -o '"total_tokens":[0-9]*' | cut -d: -f2)
        toks=${toks:-0}

        echo "  Run $i: ${elapsed}s, $toks tokens"
        total_tokens=$((total_tokens + toks))
        total_time=$(echo "$total_time + $elapsed" | bc)
    done

    local avg_time=$(echo "scale=2; $total_time / $runs" | bc)
    local avg_tokens=$((total_tokens / runs))
    local tps="0"
    if [ $(echo "$avg_time > 0" | bc) -eq 1 ]; then
        tps=$(echo "scale=2; $avg_tokens / $avg_time" | bc)
    fi

    echo ""
    echo "  Results:"
    echo "    Average time:   ${avg_time}s"
    echo "    Average tokens: $avg_tokens"
    echo "    Tokens/sec:     $tps"
    echo ""

    # Save results
    echo "$name,$avg_time,$avg_tokens,$tps,$ngl,$ot_flag" >> "$RESULTS_DIR/benchmark_results.csv"

    # Cleanup
    kill $pid 2>/dev/null || true
    sleep 2
}

# Initialize results CSV
echo "name,avg_time,avg_tokens,tps,ngl,ot_flag" > "$RESULTS_DIR/benchmark_results.csv"

# Test 1: 35B MoE with experts on CPU (current approach)
run_benchmark "35b_ot_cpu" 20 '-ot ".*exps.*=CPU"' "Qwen3.6-35B-A3B, experts on CPU"

# Test 2: 9B dense baseline (for comparison)
run_benchmark "9b_baseline" 25 "" "Qwen3.5-9B, full GPU offload"

echo ""
echo "=== Phase 3: Summary ==="
echo ""
echo "Results saved to: $RESULTS_DIR/benchmark_results.csv"
echo ""
echo "name,avg_time,avg_tokens,tps,ngl,ot_flag"
cat "$RESULTS_DIR/benchmark_results.csv" | column -t -s',' 2>/dev/null || cat "$RESULTS_DIR/benchmark_results.csv"
echo ""
echo "========================================"
echo "  Benchmark complete"
echo "========================================"

# Cleanup
pkill -f "llama-server.*$PORT" 2>/dev/null || true
