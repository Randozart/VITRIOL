#!/usr/bin/env bash
# Engine launch gate (2026-11-26): preflight + postlaunch readiness, hard-failing.
# Usage: engine-launch-gate.sh pre|post <log-file> [port]
set -euo pipefail
MODE="${1:?pre|post}"; LOG="${2:-}"; PORT="${3:-}"

fail() { echo "GATE FAIL: $*" >&2; exit 1; }

if [ "$MODE" = pre ]; then
    # 1. no engine processes left (verified, not assumed)
    for name in llama-server vitriol-server; do
        if pgrep -x "$name" >/dev/null 2>&1; then
            fail "a $name process is still resident: $(pgrep -ax $name)"
        fi
    done
    # 2. nvidia-smi must succeed and show only desktop residents
    APPS="$(nvidia-smi --query-compute-apps=pid,process_name,used_memory --format=csv,noheader)" || fail "nvidia-smi failed"
    echo "compute apps: ${APPS:-<none>}"
    if echo "$APPS" | grep -qiE 'llama|vitriol'; then fail "engine process still holds VRAM"; fi
    # 3. free VRAM per card
    nvidia-smi --query-gpu=index,memory.free --format=csv,noheader || fail "nvidia-smi free query failed"
    echo "GATE PRE: PASS"
elif [ "$MODE" = post ]; then
    [ -n "$LOG" ] && [ -f "$LOG" ] || fail "post needs the server log path"
    [ -n "$PORT" ] || fail "post needs the port"
    # a. health ok
    for i in $(seq 1 240); do
        if curl -s --max-time 2 "http://127.0.0.1:$PORT/health" 2>/dev/null | grep -q '"ok"'; then break; fi
        [ "$i" = 240 ] && fail "health never reached ok (720 s)"
        sleep 3
    done
    # b. no compute-buffer failures in the log
    if grep -qE 'cudaMalloc failed|retrying without pipeline|failed to allocate compute' "$LOG"; then
        fail "log shows compute-buffer failure (CPU-fallback risk): see $LOG"
    fi
    # c. GPU actually computes: util must exceed 50% during a probe decode
    curl -s --max-time 300 "http://127.0.0.1:$PORT/v1/chat/completions" -H 'Content-Type: application/json' \
        -d '{"messages":[{"role":"user","content":"hello"}],"max_tokens":32,"temperature":0}' >/dev/null &
    PROBE=$!
    sleep 2
    UTIL="$(nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader,nounits | sort -rn | head -1)"
    wait $PROBE
    echo "probe decode peak GPU util: $UTIL%"
    [ "${UTIL:-0}" -gt 50 ] || fail "GPU util ${UTIL}% during probe decode — engine is computing on CPU"
    echo "GATE POST: PASS"
fi
