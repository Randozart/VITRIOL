#!/usr/bin/env bash
# launch_vitriol_full.sh — one-command full VITRIOL stack: setup (caps) + generation
# server + Copula memory stack (Hermetis + GPU embed).
#
#   ./scripts/launch_vitriol_full.sh                          # full launch
#   ./scripts/launch_vitriol_full.sh --model PATH --ngl 32 ... # overrides
#   ./scripts/launch_vitriol_full.sh stop                      # stop everything
#   ./scripts/launch_vitriol_full.sh --no-copula               # gen server only
#   ./scripts/launch_vitriol_full.sh --no-setup                # skip sudo setup
#   ./scripts/launch_vitriol_full.sh --copula-only             # memory stack only
#
# Setup needs sudo (interactive). Without caps the generation server still runs for
# VRAM-fit models; page-locked stream mode needs `sudo vitriol setup`.
set -euo pipefail

VITRIOL_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVER="$VITRIOL_DIR/llama.cpp/build/bin/llama-server"
VITRIOL_SCRIPT="$VITRIOL_DIR/scripts/vitriol"
COPULA_SCRIPT="$VITRIOL_DIR/scripts/launch_copula.sh"
LOG_DIR="${COPULA_LOG_DIR:-/tmp/opencode}"

# Defaults (Mellum2 — the recommended OpenCode model; values from profiles/mellum2)
GEN_MODEL="${VITRIOL_GEN_MODEL:-/home/randozart/Desktop/Projects/Mellum2-12B-A2.5B-Instruct-Q4_K_M.gguf}"
GEN_PORT="${VITRIOL_GEN_PORT:-8279}"
NGL="${VITRIOL_NGL:-24}"
CTX="${VITRIOL_CTX:-32768}"
THREADS="${VITRIOL_THREADS:-4}"
PARALLEL="${VITRIOL_PARALLEL:-2}"

DO_SETUP=1
DO_COPULA=1
DO_GEN=1

for arg in "$@"; do
    case "$arg" in
        --model=*) GEN_MODEL="${arg#*=}" ;;
        --ngl=*)   NGL="${arg#*=}" ;;
        --ctx=*)   CTX="${arg#*=}" ;;
        --threads=*) THREADS="${arg#*=}" ;;
        --parallel=*) PARALLEL="${arg#*=}" ;;
        --gen-port=*) GEN_PORT="${arg#*=}" ;;
        --no-setup) DO_SETUP=0 ;;
        --no-copula) DO_COPULA=0 ;;
        --copula-only) DO_GEN=0 ;;
    esac
done

port_pid() { ss -ltnp 2>/dev/null | grep ":$1 " | grep -oE 'pid=[0-9]+' | head -1 | cut -d= -f2; }

stop() {
    pid=$(port_pid "$GEN_PORT")
    if [ -n "$pid" ]; then
        echo "[vitriol] stopping gen server :$GEN_PORT (pid $pid)"
        kill -9 "$pid" 2>/dev/null || true
    fi
    if [ -x "$COPULA_SCRIPT" ]; then
        "$COPULA_SCRIPT" stop
    fi
    echo "[vitriol] stopped"
    exit 0
}

[ "${1:-}" = "stop" ] && stop

echo "[vitriol] VITRIOL dir: $VITRIOL_DIR"

# ── Setup (caps) ──────────────────────────────────────────────────────────────
if [ "$DO_SETUP" = "1" ] && [ -x "$SERVER" ]; then
    if getcap "$SERVER" 2>/dev/null | grep -q cap_ipc_lock; then
        echo "[vitriol] caps already set — skipping setup"
    else
        echo "[vitriol] llama-server lacks cap_ipc_lock; running sudo setup..."
        if sudo "$VITRIOL_SCRIPT" setup 2>&1; then
            echo "[vitriol] setup ok"
        else
            echo "[vitriol] WARN: setup failed (no sudo/tty?). VRAM-fit models still run;"
            echo "         page-locked stream mode needs: sudo $VITRIOL_SCRIPT setup"
        fi
    fi
fi

# ── Generation server ─────────────────────────────────────────────────────────
if [ "$DO_GEN" = "1" ]; then
    if [ ! -x "$SERVER" ]; then
        echo "[vitriol] ERROR: $SERVER not found — build first" >&2
        exit 1
    fi
    if [ -n "$(port_pid "$GEN_PORT")" ]; then
        echo "[vitriol] gen server already on :$GEN_PORT — skipping"
    else
        echo "[vitriol] starting gen server on :$GEN_PORT ($(basename "$GEN_MODEL"), ngl=$NGL ctx=$CTX t=$THREADS p=$PARALLEL)"
        setsid nohup "$SERVER" -m "$GEN_MODEL" -ngl "$NGL" -c "$CTX" -t "$THREADS" \
            --parallel "$PARALLEL" --port "$GEN_PORT" \
            > "$LOG_DIR/vitriol_gen.log" 2>&1 < /dev/null &
    fi
fi

# ── Copula stack ──────────────────────────────────────────────────────────────
if [ "$DO_COPULA" = "1" ]; then
    if [ -x "$COPULA_SCRIPT" ]; then
        "$COPULA_SCRIPT"
    else
        echo "[vitriol] WARN: $COPULA_SCRIPT not found — Copula skipped"
    fi
fi

# ── Verify ────────────────────────────────────────────────────────────────────
sleep 4
echo ""
echo "[vitriol] status:"
if [ "$DO_GEN" = "1" ]; then
    echo "  gen:     $(curl -s -m 3 "http://127.0.0.1:$GEN_PORT/health" 2>/dev/null || echo down)"
fi
if [ "$DO_COPULA" = "1" ]; then
    echo "  hermetis: $(curl -s -m 3 http://127.0.0.1:8090/health 2>/dev/null || echo down)"
    echo "  embed:    $(curl -s -m 3 http://127.0.0.1:8081/health 2>/dev/null || echo down)"
fi
echo ""
echo "  OpenCode: restart it, then point the provider at http://127.0.0.1:$GEN_PORT/v1"
echo "  Stop all: $0 stop"
