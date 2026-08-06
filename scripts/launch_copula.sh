#!/usr/bin/env bash
# launch_copula.sh — bring up the Copula stack: Hermetis memory + optional GPU embed server.
#
#   launch_copula.sh            start Hermetis (:8090) + GPU embed server (:8081)
#   launch_copula.sh stop       stop both
#   COPULA_NO_EMBED=1 ...       skip the embed server (semantic falls back)
#   COPULA_EMBED_PORT=...       override embed port
#   COPULA_HERMETIS_PORT=...    override Hermetis port
#
# The generation server is started separately (your normal VITRIOL launch).
set -euo pipefail

VITRIOL_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HERMETIS="$VITRIOL_DIR/libvitriol/hermetis_server.py"
SERVER="$VITRIOL_DIR/llama.cpp/build/bin/llama-server"
EMBED_MODEL="${COPULA_EMBED_MODEL:-/home/randozart/Desktop/Projects/bge-small-en-v1.5-q8_0.gguf}"
LOG_DIR="${COPULA_LOG_DIR:-/tmp/opencode}"
HERMETIS_PORT="${COPULA_HERMETIS_PORT:-8090}"
EMBED_PORT="${COPULA_EMBED_PORT:-8081}"
EMBED_NGL="${COPULA_EMBED_NGL:-99}"

port_pid() {
    local port="$1" p=""
    p=$(ss -ltnp 2>/dev/null | grep ":$port " | grep -oE 'pid=[0-9]+' | head -1 | cut -d= -f2)
    if [ -n "$p" ]; then echo "$p"; return 0; fi
    p=$(lsof -ti ":$port" 2>/dev/null | head -1)
    if [ -n "$p" ]; then echo "$p"; return 0; fi
    p=$(pgrep -f "(llama-server|hermetis_server).*--port $port" 2>/dev/null | head -1)
    if [ -n "$p" ]; then echo "$p"; return 0; fi
    p=$(fuser -n tcp "$port" 2>/dev/null | tr -s ' ' | head -1 | xargs echo)
    echo "$p"
    return 0
}

stop() {
    for p in "$HERMETIS_PORT" "$EMBED_PORT"; do
        pid=$(port_pid "$p")
        if [ -n "$pid" ]; then
            echo "[copula] stopping :$p (pid $pid)"
            kill -9 "$pid" 2>/dev/null || true
        fi
    done
    sleep 1
    exit 0
}

[ "${1:-}" = "stop" ] && stop

if [ ! -f "$HERMETIS" ]; then
    echo "[copula] ERROR: $HERMETIS not found" >&2
    exit 1
fi

echo "[copula] VITRIOL dir: $VITRIOL_DIR"

if [ -n "$(port_pid "$HERMETIS_PORT")" ]; then
    echo "[copula] Hermetis already on :$HERMETIS_PORT — skipping"
else
    echo "[copula] starting Hermetis on :$HERMETIS_PORT"
    VITRIOL_SEMANTIC_MODE=on setsid nohup python3 "$HERMETIS" --port "$HERMETIS_PORT" \
        > "$LOG_DIR/copula_hermetis.log" 2>&1 < /dev/null &
fi

if [ "${COPULA_NO_EMBED:-0}" = "1" ]; then
    echo "[copula] embed server skipped (COPULA_NO_EMBED=1; semantic falls back to sentence-transformers/keyword)"
elif [ -n "$(port_pid "$EMBED_PORT")" ]; then
    echo "[copula] embed server already on :$EMBED_PORT — skipping"
elif [ ! -x "$SERVER" ]; then
    echo "[copula] WARN: llama-server not found at $SERVER — embed server skipped"
elif [ ! -f "$EMBED_MODEL" ]; then
    echo "[copula] WARN: embed model not found at $EMBED_MODEL — embed server skipped"
else
    echo "[copula] starting GPU embed server on :$EMBED_PORT ($(basename "$EMBED_MODEL"), ngl=$EMBED_NGL)"
    setsid nohup "$SERVER" -m "$EMBED_MODEL" --embedding -ngl "$EMBED_NGL" -c 512 -t 4 \
        --port "$EMBED_PORT" > "$LOG_DIR/copula_embed.log" 2>&1 < /dev/null &
fi

echo "[copula] started. verify:"
echo "  Hermetis: curl -s http://127.0.0.1:$HERMETIS_PORT/health   # expect {\"service\":\"hermetis\",...}"
echo "  Embed:    curl -s http://127.0.0.1:$EMBED_PORT/health      # expect {\"status\":\"ok\"}"
echo "  Stop:     $0 stop"
