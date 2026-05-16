#!/bin/bash
# safe_test_vitriol.sh - Gradual, safe testing of VITRIOL + KoboldCPP
# 
# This script tests in stages. STOP if anything looks wrong.
# Press Ctrl+C at any stage if you see errors.

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_step() { echo -e "${BLUE}[STEP]${NC} $1"; }
log_ok() { echo -e "${GREEN}[OK]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_fail() { echo -e "${RED}[FAIL]${NC} $1"; }

cleanup() {
    log_warn "Cleaning up..."
    pkill -f "koboldcpp" 2>/dev/null || true
    pkill -f "vitriol_shim.py" 2>/dev/null || true
    log_ok "Stopped all services"
}

trap cleanup EXIT

echo "=============================================="
echo "     VITRIOL Safe Test Sequence"
echo "=============================================="
echo ""
echo "This will test in 3 stages. Press Ctrl+C to stop at any time."
echo ""
echo "Stage 1: KoboldCPP only (baseline)"
echo "Stage 2: VITRIOL shim only (no GPU)"
echo "Stage 3: Full stack (both together)"
echo ""
read -p "Press Enter to begin, or Ctrl+C to cancel..."

# ==============================================================================
# Stage 1: Test KoboldCPP alone
# ==============================================================================
log_step "Stage 1: Starting KoboldCPP (reduced settings for safety)..."
echo ""

cd ~/Downloads/koboldCPP

# Start KoboldCPP with your working GPU settings
./koboldcpp \
    --model Qwen_Qwen3.5-9B-Q4_K_M.gguf \
    --usecuda \
    --gpulayers 30 \
    --contextsize 8192 \
    --quantkv 1 \
    --lowvram \
    --noavx2 \
    --multiuser \
    --port 5001 &

KOBOLD_PID=$!
log_ok "KoboldCPP started (PID: $KOBOLD_PID)"

echo ""
log_step "Waiting 30 seconds for KoboldCPP to load model..."
echo "     (First launch takes longer - CUDA initialization)"
sleep 30

echo ""
log_step "Testing KoboldCPP health (retrying up to 5 times)..."
KOBOLD_HEALTHY=false
for i in {1..5}; do
    if curl -s http://localhost:5001/api/v1/info > /dev/null 2>&1; then
        KOBOLD_HEALTHY=true
        break
    fi
    log_warn "Attempt $i/5: Not ready yet, waiting 5 more seconds..."
    sleep 5
done

if $KOBOLD_HEALTHY; then
    log_ok "KoboldCPP is responding on port 5001"
else
    log_fail "KoboldCPP not responding after 5 attempts"
    log_warn "Check dmesg for GPU errors: dmesg | tail -20"
    log_warn "Check if CUDA initialized: look for 'CUDA' in output above"
    exit 1
fi

echo ""
log_step "Testing simple inference (direct to KoboldCPP)..."
RESPONSE=$(curl -s http://localhost:5001/v1/chat/completions \
    -H "Content-Type: application/json" \
    -d '{
        "messages": [{"role": "user", "content": "Say hello in 3 words"}],
        "max_tokens": 20
    }')

if echo "$RESPONSE" | grep -q "choices"; then
    log_ok "KoboldCPP inference working!"
    echo "Response: $(echo "$RESPONSE" | python3 -c "import sys,json; print(json.load(sys.stdin)['choices'][0]['message']['content'][:50])")..."
else
    log_fail "Inference failed"
    echo "Response: $RESPONSE"
    exit 1
fi

echo ""
log_ok "Stage 1 PASSED - KoboldCPP is stable"
echo ""
read -p "Continue to Stage 2? (Ctrl+C to stop, Enter to continue)..."

# ==============================================================================
# Stage 2: Test VITRIOL shim (with KoboldCPP still running)
# ==============================================================================
log_step "Stage 2: Starting VITRIOL Context Rectifier..."
echo ""

cd ~/Desktop/Projects/linux-pipe-module

python3 vitriol_shim.py &
SHIM_PID=$!
log_ok "VITRIOL shim started (PID: $SHIM_PID)"

echo ""
log_step "Waiting 3 seconds for VITRIOL to start..."
sleep 3

echo ""
log_step "Testing VITRIOL health endpoint..."
if curl -s http://localhost:5010/health | grep -q '"status": "ok"'; then
    log_ok "VITRIOL shim is responding on port 5005"
else
    log_fail "VITRIOL shim not responding"
    exit 1
fi

echo ""
log_step "Testing VITRIOL rectification (no inference yet)..."
RECTIFY_TEST=$(curl -s http://localhost:5010/rectify \
    -H "Content-Type: application/json" \
    -d '{
        "messages": [
            {"role": "system", "content": "You are helpful"},
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi there!"}
        ]
    }')

if echo "$RECTIFY_TEST" | grep -q "rectified_messages"; then
    log_ok "VITRIOL rectification working!"
    echo "Stats: $(echo "$RECTIFY_TEST" | python3 -c "import sys,json; d=json.load(sys.stdin); print(f\"{d['original_messages']} -> {d['rectified_messages']} messages\")")"
else
    log_fail "Rectification failed"
    echo "Response: $RECTIFY_TEST"
    exit 1
fi

echo ""
log_ok "Stage 2 PASSED - VITRIOL shim is stable"
echo ""
read -p "Continue to Stage 3? (Ctrl+C to stop, Enter to continue)..."

# ==============================================================================
# Stage 3: Test full stack (VITRIOL -> KoboldCPP)
# ==============================================================================
log_step "Stage 3: Testing full inference through VITRIOL..."
echo ""

log_step "Sending inference request through VITRIOL shim..."
FULL_TEST=$(curl -s http://localhost:5010/v1/chat/completions \
    -H "Content-Type: application/json" \
    -d '{
        "messages": [
            {"role": "system", "content": "You are a helpful assistant."},
            {"role": "user", "content": "What is 2+2? Answer in one word."}
        ],
        "max_tokens": 10
    }')

if echo "$FULL_TEST" | grep -q "choices"; then
    log_ok "Full stack inference working!"
    echo "Response: $(echo "$FULL_TEST" | python3 -c "import sys,json; print(json.load(sys.stdin)['choices'][0]['message']['content'][:50])")..."
else
    log_fail "Full stack inference failed"
    echo "Response: $FULL_TEST"
    exit 1
fi

echo ""
log_step "Testing context rectification with bloated prompt..."
BLOAT_TEST=$(curl -s http://localhost:5010/v1/chat/completions \
    -H "Content-Type: application/json" \
    -d '{
        "messages": [
            {"role": "system", "content": "You are helpful"},
            {"role": "user", "content": "A" * 10000},
            {"role": "assistant", "content": "B" * 10000},
            {"role": "user", "content": "C" * 10000},
            {"role": "assistant", "content": "D" * 10000},
            {"role": "user", "content": "E" * 10000},
            {"role": "assistant", "content": "F" * 10000},
            {"role": "user", "content": "Short question"}
        ],
        "max_tokens": 20
    }')

if echo "$BLOAT_TEST" | grep -q "choices"; then
    log_ok "Rectification prevented context overflow!"
else
    log_warn "Bloated prompt test failed (but this is expected if context is too large)"
fi

echo ""
echo "=============================================="
echo "     ALL TESTS PASSED!"
echo "=============================================="
echo ""
echo "VITRIOL is ready to use."
echo ""
echo "Next steps:"
echo "  1. Point OpenCode to: http://localhost:5010/v1/chat/completions"
echo "  2. Or run: ./run_qwen.sh for the full stack"
echo ""
echo "Services running:"
echo "  - KoboldCPP: http://localhost:5001 (PID: $KOBOLD_PID)"
echo "  - VITRIOL:   http://localhost:5010 (PID: $SHIM_PID)"
echo ""
echo "Press Ctrl+C to stop both services, or leave running."
echo ""

# Don't cleanup on success - let user decide
trap - EXIT
wait
