#!/bin/bash
# REBIS head launcher — blessed flags for day-long sessions.
# Usage: rebis-servers.sh [sol|luna|both]
#
# Sol   = Qwen3.8-27B UD-IQ2_S  resident on GPU0 (:8279, Au=79)
# Luna  = Mellum2-Thinking IQ4_XS pinned on GPU1 (:8247, Ag=47)
#
# Day-long readiness encoded here:
#   --context-shift --cache-reuse 256   rolling windows (safe post-H1 gate)
#   ---ctx-checkpoints 12                bound checkpoint RAM (default 32)
#   --checkpoint-every-n-tokens 8192    fewer, larger checkpoints
#   --cache-ram 2048/1024               bounded prompt cache (OOM vector otherwise)
#   mmap weights                        no staging collisions on 15 GB RAM

set -u
# private build dir: co-tenant pipelines manage llama.cpp/build and delete
# binaries from under us — build-rebis is ours alone
BIN_SOL="$(dirname "$0")/../llama.cpp/build-rebis/bin/llama-server"
BIN_LUNA="$BIN_SOL"
SOL_MODEL="${HOME}/Downloads/Qwen3.8-27B-UD-IQ2_S.gguf"
LUNA_MODEL="${HOME}/Downloads/Mellum2-12B-A2.5B-Thinking.i1-IQ4_XS.gguf"
CTX=65536
WHICH="${1:-both}"

COMMON=(--cache-type-k q4_0 --cache-type-v q4_0 -fa on --jinja
        --context-shift --cache-reuse 256
        --ctx-checkpoints 12 --checkpoint-every-n-tokens 8192
        --host 127.0.0.1 --slots --metrics)

start_sol() {
  local BUDGET="${REBIS_REASONING_BUDGET:-1024}"
  CUDA_VISIBLE_DEVICES=0 setsid nohup "$BIN_SOL" \
    -m "$SOL_MODEL" -ngl 99 -c "$CTX" "${COMMON[@]}" \
    --cache-ram 2048 --port 8279 \
    --reasoning-budget "$BUDGET" > /tmp/qwen.log 2>&1 < /dev/null &
  echo "Sol launching :8279 (reasoning budget $BUDGET)"
}

start_luna() {
  CUDA_VISIBLE_DEVICES=1 setsid nohup "$BIN_LUNA" \
    -m "$LUNA_MODEL" -ngl 99 -c "$CTX" "${COMMON[@]}" \
    --cache-ram 1024 --port 8247 > /tmp/mellum.log 2>&1 < /dev/null &
  echo "Luna launching :8247"
}

supervise() { # supervise <start_fn> <name>
  while true; do
    "$1"
    echo "$(date +%H:%M:%S) $2 exited — respawning in 15s" >> /tmp/rebis-supervise.log
    sleep 15
  done
}

start_mercury() {
  setsid nohup ./scripts/rebis-gateway.sh >> /tmp/shim.log 2>&1 < /dev/null &
  echo "Mercury launching :${REBIS_PORT:-8280}"
}

case "$WHICH" in
  sol)  start_sol ;;
  luna) start_luna ;;
  mercury) start_mercury ;;
  both) killall -9 llama-server 2>/dev/null; sleep 2
        start_luna; sleep 20; start_sol ;;
  sol-sup)  supervise start_sol  Sol  ;;
  luna-sup) supervise start_luna Luna ;;
  mercury-sup) supervise start_mercury Mercury ;;
  both-sup)
        killall -9 llama-server 2>/dev/null
        supervise start_sol  Sol  &
        supervise start_luna Luna & ;;
  rebis)
        # THE boot command: whole trenchcoat, supervised.
        killall -9 llama-server 2>/dev/null
        pkill -f '[r]ebis_shim.py' 2>/dev/null
        start_luna; start_sol; sleep 20; start_mercury
        supervise start_sol     Sol      &
        supervise start_luna    Luna     &
        supervise start_mercury Mercury  & ;;
  *) echo "usage: $0 [sol|luna|mercury|both|rebis|sol-sup|luna-sup|mercury-sup|both-sup]"
     exit 1 ;;
esac
