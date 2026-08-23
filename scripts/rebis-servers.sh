#!/bin/bash
# REBIS head launcher & watchdog — blessed flags for day-long sessions.
# Usage: rebis-servers.sh [sol|luna|mercury|both|rebis|*-sup variants]
#
# Sol   = Qwen3.8-27B UD-IQ2_S  resident on GPU0 (:8279, Au=79)
# Luna  = Mellum2-Thinking IQ4_XS pinned on GPU1 (:8247, Ag=47)
#
# Day-long readiness encoded here:
#   --context-shift --cache-reuse 256    rolling windows (safe post-H1 gate)
#   --ctx-checkpoints 12                 bound checkpoint RAM (default 32)
#   --checkpoint-every-n-tokens 8192     fewer, larger checkpoints
#   --cache-ram 2048/1024                bounded prompt cache (OOM vector otherwise)
#   mmap weights                         no staging collisions on 15 GB RAM
#
# Supervision model (fixed 2026-08-22): supervised mode EXECs the server in
# the foreground and only spawns when the port has no healthy answer —
# earlier design backgrounded servers inside the supervisor, which spawned a
# duplicate every cycle (bind-fail churn + log truncation wiped telemetry).

set -u
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN_SOL="$SCRIPT_DIR/../llama.cpp/build-rebis/bin/llama-server"
BIN_LUNA="$BIN_SOL"
SOL_MODEL="${HOME}/Downloads/Qwen3.8-27B-UD-IQ2_S.gguf"
LUNA_MODEL="${HOME}/Downloads/Mellum2-12B-A2.5B-Thinking.i1-IQ4_XS.gguf"
CTX=65536
WHICH="${1:-both}"
BACKOFF="${REBIS_BACKOFF:-15}"

COMMON=(--cache-type-k q4_0 --cache-type-v q4_0 -fa on --jinja
        --context-shift --cache-reuse 256
        --ctx-checkpoints 8 --checkpoint-every-n-tokens 16384
        --host 127.0.0.1 --slots --metrics)

port_healthy() { curl -s -m1 "http://127.0.0.1:$1/health" 2>/dev/null | grep -q '"ok"'; }

# Foreground runners — these BLOCK until the server dies.
run_sol() {
  local BUDGET="${REBIS_REASONING_BUDGET:-1024}"
  CUDA_VISIBLE_DEVICES=0 "$BIN_SOL" \
    -m "$SOL_MODEL" -ngl 99 -c "$CTX" "${COMMON[@]}" \
    --cache-ram 1024 --port 8279 \
    --reasoning-budget "$BUDGET" >> /tmp/qwen.log 2>&1
}

run_luna() {
  CUDA_VISIBLE_DEVICES=1 "$BIN_LUNA" \
    -m "$LUNA_MODEL" -ngl 99 -c "$CTX" "${COMMON[@]}" \
    --cache-ram 512 --port 8247 >> /tmp/mellum.log 2>&1
}

run_mercury() {
  "$SCRIPT_DIR/rebis-gateway.sh" >> /tmp/shim.log 2>&1
}

# One-shot background starts (used by non-supervised modes).
start_sol()     { run_sol &  disown; echo "Sol launching :8279"; }
start_luna()    { run_luna & disown; echo "Luna launching :8247"; }
start_mercury() { run_mercury & disown; echo "Mercury launching :${REBIS_PORT:-8280}"; }

supervise() { # supervise <runner_fn> <port> <name>
  local fn="$1" port="$2" name="$3"
  while true; do
    if port_healthy "$port"; then
      sleep 30                      # healthy instance already serving; stand down
      continue
    fi
    echo "$(date +%H:%M:%S) $name not answering :$port — starting"
    "$fn"                           # blocks for the server's lifetime
    echo "$(date +%H:%M:%S) $name exited — respawning in ${BACKOFF}s" \
      >> /tmp/rebis-supervise.log
    sleep "$BACKOFF"
  done
}

case "$WHICH" in
  sol)     run_sol ;;
  luna)    run_luna ;;
  mercury) run_mercury ;;
  both)    killall -9 llama-server 2>/dev/null; sleep 2
           start_luna; sleep 20; start_sol ;;
  sol-sup)     supervise run_sol     8279 Sol ;;
  luna-sup)    supervise run_luna    8247 Luna ;;
  mercury-sup) supervise run_mercury 8280 Mercury ;;
  both-sup)
        killall -9 llama-server 2>/dev/null
        supervise run_sol  8279 Sol &
        supervise run_luna 8247 Luna & ;;
  rebis)
        # THE boot command: whole trenchcoat, watched.
        killall -9 llama-server 2>/dev/null
        pkill -f '[r]ebis_shim.py' 2>/dev/null
        start_mercury   # stateless; no supervision (avoids dual-spawn flapping)
        start_luna
        start_sol
        supervise run_sol  8279 Sol  &
        supervise run_luna 8247 Luna & ;;
  stop)
        # Tear down the entire REBIS: supervisors, heads, gateway.
        # Patterns avoid self-match (this script's argv contains
        # "rebis-servers.sh stop" — no -sup, no gateway, no shim).
        pkill -f 'rebis-servers.sh .*-sup' 2>/dev/null
        pkill -f '[r]ebis-gateway' 2>/dev/null
        pkill -f '[r]ebis_shim' 2>/dev/null
        sleep 1
        killall -9 llama-server 2>/dev/null
        echo "REBIS torn down (heads + gateway + supervisors)" ;;
  *) echo "usage: $0 [sol|luna|mercury|both|rebis|stop|sol-sup|luna-sup|mercury-sup|both-sup]"
     exit 1 ;;
esac
