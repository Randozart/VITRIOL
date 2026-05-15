#!/usr/bin/env bash
set -euo pipefail

# benchmark_alka.sh — VITRIOL Alka DMA Benchmark
#
# Benchmarks 3 configurations to answer: does Alka-informed DMA work?
#   1. Alka base load + llama.cpp CPU experts
#   2. Alka full load + llama.cpp all GPU
#   3. Native llama.cpp (no Alka) + CPU experts (control)
#
# Records: load times, tok/s, GPU util, VRAM, power, temp
#
# Usage: ./scripts/benchmark_alka.sh [--model PATH] [--skip-module]

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VITRIOL_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# --- Configuration ---
: "${VITRIOL_MODEL_DIR:=/mnt/data/ai/koboldcpp}"
: "${VITRIOL_LLAMA_DIR:=/mnt/data/ai/llama.cpp}"
: "${VITRIOL_ALKA_BIN:=$VITRIOL_ROOT/../alka-lang/zig-out/bin/alka}"

# Find llama-server (could be in build/bin or bin)
if [ -f "${VITRIOL_LLAMA_DIR}/build/bin/llama-server" ]; then
    LLAMA_SERVER="${VITRIOL_LLAMA_DIR}/build/bin/llama-server"
elif [ -f "${VITRIOL_LLAMA_DIR}/bin/llama-server" ]; then
    LLAMA_SERVER="${VITRIOL_LLAMA_DIR}/bin/llama-server"
else
    LLAMA_SERVER=""
fi

MODEL_35B="${VITRIOL_MODEL_DIR}/Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf"
GENERATOR="$VITRIOL_ROOT/scripts/generate-alka-recipe.sh"
EXECUTOR="$VITRIOL_ROOT/alka-executor/alka-executor"
MODULE="$VITRIOL_ROOT/vitriol-daemon/vitriol.ko"
VIAL="$VITRIOL_ROOT/alka/vials/vitriol_rig.alkavl"

PORT=8279
SKIP_MODULE=0
RESULTS_DIR="$VITRIOL_ROOT/alka/results/$(date +%Y-%m-%d)"
mkdir -p "$RESULTS_DIR"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --model) MODEL_35B="$2"; shift 2 ;;
        --skip-module) SKIP_MODULE=1; shift ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# --- Helper Functions ---

timestamp() { date -u +"%Y-%m-%dT%H:%M:%SZ"; }

log() {
    echo "[$(timestamp)] $*" | tee -a "$RESULTS_DIR/benchmark.log"
}

gpu_snapshot() {
    local label="$1"
    local file="$RESULTS_DIR/gpu_${label}.txt"
    nvidia-smi --query-gpu=index,name,temperature.gpu,power.draw,utilization.gpu,memory.used,memory.total \
        --format=csv,noheader,nounits 2>/dev/null | while IFS=',' read -r idx name temp power util mem_used mem_total; do
        echo "GPU $idx ($name): temp=${temp}C power=${power}W util=${util}% vram=${mem_used}/${mem_total}MB"
    done | tee "$file"
}

gpu_json() {
    local label="$1"
    local file="$RESULTS_DIR/gpu_${label}.json"
    nvidia-smi --query-gpu=temperature.gpu,power.draw,utilization.gpu,memory.used,memory.total \
        --format=csv,noheader,nounits 2>/dev/null | head -1 | \
        awk -F',' '{printf "{\"temp_c\":%s,\"power_w\":%s,\"gpu_util_pct\":%s,\"vram_used_mb\":%s,\"vram_total_mb\":%s}\n", $1, $2, $3, $4, $5}' \
        > "$file"
    cat "$file"
}

cleanup() {
    log "Cleaning up..."
    pkill -f "llama-server.*$PORT" 2>/dev/null || true
    if [ "$SKIP_MODULE" -eq 0 ] && lsmod | grep -q vitriol; then
        log "Unloading vitriol module..."
        sudo rmmod vitriol 2>/dev/null || true
    fi
}
trap cleanup EXIT

wait_for_server() {
    local port=$1
    local max_wait=$2
    local waited=0
    while [ $waited -lt $max_wait ]; do
        if curl -s "http://localhost:$port/health" > /dev/null 2>&1; then
            return 0
        fi
        sleep 2
        waited=$((waited + 2))
    done
    return 1
}

run_inference_benchmark() {
    local port=$1
    local prompt="$2"
    local max_tokens="$3"
    local output_file="$4"

    local start=$(date +%s%N)
    local result=$(curl -s "http://localhost:$port/v1/chat/completions" \
        -H "Content-Type: application/json" \
        -d "{\"messages\":[{\"role\":\"user\",\"content\":\"$prompt\"}],\"max_tokens\":$max_tokens}" 2>/dev/null)
    local end=$(date +%s%N)

    local elapsed_ms=$(( (end - start) / 1000000 ))
    local elapsed_s=$(echo "scale=3; $elapsed_ms / 1000" | bc)

    local completion_tokens=$(echo "$result" | grep -o '"completion_tokens":[0-9]*' | cut -d: -f2 || echo "0")
    local total_tokens=$(echo "$result" | grep -o '"total_tokens":[0-9]*' | cut -d: -f2 || echo "0")

    local tps=$(echo "scale=2; $completion_tokens / $elapsed_s" | bc 2>/dev/null || echo "0")

    echo "{\"elapsed_ms\":$elapsed_ms,\"elapsed_s\":$elapsed_s,\"completion_tokens\":${completion_tokens:-0},\"total_tokens\":${total_tokens:-0},\"tokens_per_second\":$tps}" > "$output_file"
    echo "$tps"
}

# --- Main ---

log "========================================"
log "VITRIOL Alka Benchmark Suite"
log "========================================"
log "35B Model: $MODEL_35B"
log "Results:   $RESULTS_DIR"
log ""

# Check prerequisites
if [ ! -f "$MODEL_35B" ]; then
    log "ERROR: 35B model not found: $MODEL_35B"
    exit 1
fi

if [ ! -f "$LLAMA_SERVER" ]; then
    log "ERROR: llama-server not found: $LLAMA_SERVER"
    log "Run: ./scripts/build-llama-server.sh"
    exit 1
fi

# ── Step 0: Load kernel module ────────────────────────────────────

if [ "$SKIP_MODULE" -eq 0 ]; then
    log "Step 0: Loading vitriol kernel module..."
    if [ ! -f "$MODULE" ]; then
        log "Building kernel module..."
        make -C "$VITRIOL_ROOT/vitriol-daemon" 2>&1 | tail -3
    fi

    if lsmod | grep -q vitriol; then
        log "Module already loaded, reloading..."
        sudo rmmod vitriol 2>/dev/null || true
        sleep 1
    fi

    log "Loading module: $MODULE"
    if ! sudo insmod "$MODULE" 2>&1 | tee "$RESULTS_DIR/module_load.log"; then
        log "ERROR: Failed to load module. Run manually: sudo insmod $MODULE"
        exit 1
    fi
    sleep 1

    if [ ! -e /dev/vitriol ]; then
        log "WARNING: /dev/vitriol not created. Check dmesg:"
        dmesg | tail -10 | tee -a "$RESULTS_DIR/module_load.log"
    else
        log "Module loaded: /dev/vitriol ready"
    fi

    dmesg | tail -20 >> "$RESULTS_DIR/module_load.log"
    log ""
fi

# ── Step 1: Generate Alka recipes ─────────────────────────────────

log "Step 1: Generating Alka recipes from GGUF..."
bash "$GENERATOR" "$MODEL_35B" --vessel GPU_1070TI 2>&1 | tee -a "$RESULTS_DIR/recipe_gen.log"
log ""

BASE_ALKAS="$VITRIOL_ROOT/alka/generated/$(basename "$MODEL_35B" .gguf)_base.alka.alkas"
FULL_ALKAS="$VITRIOL_ROOT/alka/generated/$(basename "$MODEL_35B" .gguf)_full.alka.alkas"
BASE_AZOTH="$VITRIOL_ROOT/alka/generated/$(basename "$MODEL_35B" .gguf)_base.alka.azoth"
FULL_AZOTH="$VITRIOL_ROOT/alka/generated/$(basename "$MODEL_35B" .gguf)_full.alka.azoth"

# ── Step 2: Run benchmarks ────────────────────────────────────────

RESULTS_CSV="$RESULTS_DIR/results.csv"
echo "run,config,alka_load_s,llama_load_s,inference_s,tokens,tps,gpu_util_pct,vram_mb,power_w,temp_c" > "$RESULTS_CSV"

# ── Run 1: Alka base + llama.cpp CPU experts ─────────────────────

log "=== Run 1: Alka base load + llama.cpp CPU experts ==="
log "Recipe: $(basename "$BASE_ALKAS")"

gpu_snapshot "pre_run1"

ALKA_START=$(date +%s%N)
if [ -f "$BASE_ALKAS" ] && [ -e /dev/vitriol ]; then
    sudo "$EXECUTOR" "$BASE_ALKAS" "$VIAL" --rollback "$BASE_AZOTH" 2>&1 | tee "$RESULTS_DIR/run1_alka.log"
else
    log "Skipping Alka execution (no stream or no /dev/vitriol)"
    echo "dry-run" > "$RESULTS_DIR/run1_alka.log"
fi
ALKA_END=$(date +%s%N)
ALKA_LOAD_S=$(echo "scale=3; ($ALKA_END - $ALKA_START) / 1000000000" | bc)
log "Alka load time: ${ALKA_LOAD_S}s"

gpu_snapshot "post_alka_run1"

# Start llama-server
LLAMA_START=$(date +%s%N)
CUDA_VISIBLE_DEVICES=0 "$LLAMA_SERVER" \
    -m "$MODEL_35B" \
    -ngl 20 \
    -ot ".*exps.*=CPU" \
    --port $PORT \
    --no-mmap \
    -c 4096 \
    > "$RESULTS_DIR/run1_llama.log" 2>&1 &
LLAMA_PID=$!

if wait_for_server $PORT 180; then
    LLAMA_END=$(date +%s%N)
    LLAMA_LOAD_S=$(echo "scale=3; ($LLAMA_END - $LLAMA_START) / 1000000000" | bc)
    log "llama.cpp load time: ${LLAMA_LOAD_S}s"
else
    log "ERROR: Server did not start within 180s"
    LLAMA_LOAD_S="timeout"
    kill $LLAMA_PID 2>/dev/null || true
    sleep 2
fi

if [ "$LLAMA_LOAD_S" != "timeout" ]; then
    gpu_snapshot "during_run1"
    GPU1=$(gpu_json "during_run1")

    TPS=$(run_inference_benchmark $PORT "Explain quantum computing in 3 sentences" 50 "$RESULTS_DIR/run1_inference.json")
    log "Inference: ${TPS} tok/s"

    INF_DATA=$(cat "$RESULTS_DIR/run1_inference.json")
    INF_S=$(echo "$INF_DATA" | grep -o '"elapsed_s":[0-9.]*' | cut -d: -f2)
    TOKENS=$(echo "$INF_DATA" | grep -o '"completion_tokens":[0-9]*' | cut -d: -f2)

    echo "1,alka_base+cpu_experts,$ALKA_LOAD_S,$LLAMA_LOAD_S,$INF_S,$TOKENS,$TPS,$(echo "$GPU1" | grep -o '"gpu_util_pct":[0-9.]*' | cut -d: -f2),$(echo "$GPU1" | grep -o '"vram_used_mb":[0-9.]*' | cut -d: -f2),$(echo "$GPU1" | grep -o '"power_w":[0-9.]*' | cut -d: -f2),$(echo "$GPU1" | grep -o '"temp_c":[0-9.]*' | cut -d: -f2)" >> "$RESULTS_CSV"
fi

pkill -f "llama-server.*$PORT" 2>/dev/null || true
sleep 3
log ""

# ── Run 2: Alka full load + llama.cpp all GPU ────────────────────

log "=== Run 2: Alka full load + llama.cpp all GPU ==="
log "Recipe: $(basename "$FULL_ALKAS")"

gpu_snapshot "pre_run2"

ALKA_START=$(date +%s%N)
if [ -f "$FULL_ALKAS" ] && [ -e /dev/vitriol ]; then
    sudo "$EXECUTOR" "$FULL_ALKAS" "$VIAL" --rollback "$FULL_AZOTH" 2>&1 | tee "$RESULTS_DIR/run2_alka.log"
else
    log "Skipping Alka execution (no stream or no /dev/vitriol)"
    echo "dry-run" > "$RESULTS_DIR/run2_alka.log"
fi
ALKA_END=$(date +%s%N)
ALKA_LOAD_S=$(echo "scale=3; ($ALKA_END - $ALKA_START) / 1000000000" | bc)
log "Alka load time: ${ALKA_LOAD_S}s"

gpu_snapshot "post_alka_run2"

# Start llama-server with all layers on GPU
LLAMA_START=$(date +%s%N)
CUDA_VISIBLE_DEVICES=0 "$LLAMA_SERVER" \
    -m "$MODEL_35B" \
    -ngl 41 \
    --port $PORT \
    --no-mmap \
    -c 4096 \
    > "$RESULTS_DIR/run2_llama.log" 2>&1 &
LLAMA_PID=$!

if wait_for_server $PORT 300; then
    LLAMA_END=$(date +%s%N)
    LLAMA_LOAD_S=$(echo "scale=3; ($LLAMA_END - $LLAMA_START) / 1000000000" | bc)
    log "llama.cpp load time: ${LLAMA_LOAD_S}s"
else
    log "ERROR: Server did not start within 300s (expected — 35B full may not fit in 8GB)"
    LLAMA_LOAD_S="oom"
    kill $LLAMA_PID 2>/dev/null || true
    sleep 2
fi

if [ "$LLAMA_LOAD_S" != "oom" ]; then
    gpu_snapshot "during_run2"
    GPU2=$(gpu_json "during_run2")

    TPS=$(run_inference_benchmark $PORT "Explain quantum computing in 3 sentences" 50 "$RESULTS_DIR/run2_inference.json")
    log "Inference: ${TPS} tok/s"

    INF_DATA=$(cat "$RESULTS_DIR/run2_inference.json")
    INF_S=$(echo "$INF_DATA" | grep -o '"elapsed_s":[0-9.]*' | cut -d: -f2)
    TOKENS=$(echo "$INF_DATA" | grep -o '"completion_tokens":[0-9]*' | cut -d: -f2)

    echo "2,alka_full+all_gpu,$ALKA_LOAD_S,$LLAMA_LOAD_S,$INF_S,$TOKENS,$TPS,$(echo "$GPU2" | grep -o '"gpu_util_pct":[0-9.]*' | cut -d: -f2),$(echo "$GPU2" | grep -o '"vram_used_mb":[0-9.]*' | cut -d: -f2),$(echo "$GPU2" | grep -o '"power_w":[0-9.]*' | cut -d: -f2),$(echo "$GPU2" | grep -o '"temp_c":[0-9.]*' | cut -d: -f2)" >> "$RESULTS_CSV"
fi

pkill -f "llama-server.*$PORT" 2>/dev/null || true
sleep 3
log ""

# ── Run 3: Native llama.cpp (no Alka) + CPU experts ──────────────

log "=== Run 3: Native llama.cpp (no Alka) + CPU experts ==="

gpu_snapshot "pre_run3"

LLAMA_START=$(date +%s%N)
CUDA_VISIBLE_DEVICES=0 "$LLAMA_SERVER" \
    -m "$MODEL_35B" \
    -ngl 20 \
    -ot ".*exps.*=CPU" \
    --port $PORT \
    --no-mmap \
    -c 4096 \
    > "$RESULTS_DIR/run3_llama.log" 2>&1 &
LLAMA_PID=$!

if wait_for_server $PORT 180; then
    LLAMA_END=$(date +%s%N)
    LLAMA_LOAD_S=$(echo "scale=3; ($LLAMA_END - $LLAMA_START) / 1000000000" | bc)
    log "llama.cpp load time: ${LLAMA_LOAD_S}s"
else
    log "ERROR: Server did not start within 180s"
    LLAMA_LOAD_S="timeout"
    kill $LLAMA_PID 2>/dev/null || true
    sleep 2
fi

if [ "$LLAMA_LOAD_S" != "timeout" ]; then
    gpu_snapshot "during_run3"
    GPU3=$(gpu_json "during_run3")

    TPS=$(run_inference_benchmark $PORT "Explain quantum computing in 3 sentences" 50 "$RESULTS_DIR/run3_inference.json")
    log "Inference: ${TPS} tok/s"

    INF_DATA=$(cat "$RESULTS_DIR/run3_inference.json")
    INF_S=$(echo "$INF_DATA" | grep -o '"elapsed_s":[0-9.]*' | cut -d: -f2)
    TOKENS=$(echo "$INF_DATA" | grep -o '"completion_tokens":[0-9]*' | cut -d: -f2)

    echo "3,native+cpu_experts,0,$LLAMA_LOAD_S,$INF_S,$TOKENS,$TPS,$(echo "$GPU3" | grep -o '"gpu_util_pct":[0-9.]*' | cut -d: -f2),$(echo "$GPU3" | grep -o '"vram_used_mb":[0-9.]*' | cut -d: -f2),$(echo "$GPU3" | grep -o '"power_w":[0-9.]*' | cut -d: -f2),$(echo "$GPU3" | grep -o '"temp_c":[0-9.]*' | cut -d: -f2)" >> "$RESULTS_CSV"
fi

pkill -f "llama-server.*$PORT" 2>/dev/null || true
sleep 3
log ""

# ── Summary ───────────────────────────────────────────────────────

log "========================================"
log "BENCHMARK SUMMARY"
log "========================================"
log ""
log "Results CSV: $RESULTS_CSV"
log ""
column -t -s',' "$RESULTS_CSV" | tee "$RESULTS_DIR/summary.txt"
log ""
log "GPU snapshots:"
ls "$RESULTS_DIR"/gpu_*.txt 2>/dev/null | while read -r f; do
    log "  $(basename "$f"):"
    cat "$f" | sed 's/^/    /'
done
log ""
log "Alka stream sizes:"
ls -la "$VITRIOL_ROOT/alka/generated/"*.alkas 2>/dev/null | awk '{print "  " $NF ": " $5 " bytes"}' | tee -a "$RESULTS_DIR/summary.txt"
log ""
log "All logs: $RESULTS_DIR/"
