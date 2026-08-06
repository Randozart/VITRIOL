#!/usr/bin/env bash
# launch_vitriol_full.sh — one-command full VITRIOL stack: setup (caps) + generation
# server + Copula memory stack (Hermetis + GPU embed). Includes diagnostics.
#
#   ./scripts/launch_vitriol_full.sh                          # full launch
#   ./scripts/launch_vitriol_full.sh status                   # live diagnostics
#   ./scripts/launch_vitriol_full.sh logs [gen|hermetis|embed] [N]
#   ./scripts/launch_vitriol_full.sh doctor                   # pre-flight checks
#   ./scripts/launch_vitriol_full.sh stop                     # stop everything
#   ./scripts/launch_vitriol_full.sh --no-copula | --no-setup | --copula-only
#   ./scripts/launch_vitriol_full.sh --verbose | --dry-run
#   ./scripts/launch_vitriol_full.sh --model PATH --ngl 32 ...
#
# Setup needs sudo (interactive). Without caps the generation server still runs for
# VRAM-fit models; page-locked stream mode needs `sudo vitriol setup`.
set -euo pipefail

VITRIOL_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVER="$VITRIOL_DIR/llama.cpp/build/bin/llama-server"
VITRIOL_SCRIPT="$VITRIOL_DIR/scripts/vitriol"
COPULA_SCRIPT="$VITRIOL_DIR/scripts/launch_copula.sh"
LOG_DIR="${COPULA_LOG_DIR:-/tmp/opencode}"
GEN_LOG="$LOG_DIR/vitriol_gen.log"
HERM_LOG="$LOG_DIR/copula_hermetis.log"
EMBED_LOG="$LOG_DIR/copula_embed.log"
HERM_PORT=8090
EMBED_PORT=8081

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
VERBOSE=0
DRY_RUN=0

# Find the pid bound to a port. ss -p often cannot attribute the pid (it showed
# "-" for our own gen server), so fall back to lsof then fuser. Always returns 0 so
# set -e never aborts the script on a lookup miss.
port_pid() {
    local port="$1" p=""
    p=$(ss -ltnp 2>/dev/null | grep ":$port " | grep -oE 'pid=[0-9]+' | head -1 | cut -d= -f2)
    if [ -n "$p" ]; then echo "$p"; return 0; fi
    p=$(lsof -ti ":$port" 2>/dev/null | head -1)
    if [ -n "$p" ]; then echo "$p"; return 0; fi
    # The socket tools can fail to attribute our own servers' pids; the cmdline
    # always carries --port, so pgrep is the reliable fallback.
    p=$(pgrep -f "(llama-server|hermetis_server).*--port $port" 2>/dev/null | head -1)
    if [ -n "$p" ]; then echo "$p"; return 0; fi
    p=$(fuser -n tcp "$port" 2>/dev/null | tr -s ' ' | head -1 | xargs echo)
    echo "$p"
    return 0
}
port_up() { [ "$(health_of "$1")" != "down" ]; }
health_of() { curl -s -m 3 "http://127.0.0.1:$1/health" 2>/dev/null || echo down; }
log_tail() { [ -f "$1" ] && tail -n "${2:-3}" "$1" || echo "  (no log at $1)"; }
# Fatal markers only — the benign "failed to fit params" ngl warning must not trip this.
log_err() { [ -f "$1" ] && grep -qiE "error while loading|cannot open shared|cannot open shared object|segmentation fault|ggml_assert|terminate called|abort\(|no such file|couldn't bind" "$1" && { echo "  !! log shows a fatal error:" && tail -n 4 "$1"; }; true; }
# $ORIGIN RUNPATH is ignored under AT_SECURE (file capabilities). After ANY rebuild the
# binary resets to $ORIGIN, so self-heal it with patchelf (user-owned, no sudo needed).
bin_dir() { dirname "$SERVER"; }
needs_rpath() { readelf -d "$SERVER" 2>/dev/null | grep -q '\$ORIGIN'; }
fix_rpath_local() {
    local d
    d="$(bin_dir)"
    for f in "$d"/llama-server "$d"/lib*.so*; do
        [ -f "$f" ] && patchelf --set-rpath "$d" "$f" 2>/dev/null || true
    done
    # patchelf clears file capabilities (ELF rewrite) — re-apply via setup below.
}

stop() {
    local pid=""
    pid=$(port_pid "$GEN_PORT")
    if [ -n "$pid" ]; then
        echo "[vitriol] stopping gen server :$GEN_PORT (pid $pid)"
        kill -9 "$pid" 2>/dev/null || true
    elif port_up "$GEN_PORT"; then
        echo "[vitriol] gen server on :$GEN_PORT has no visible pid — killing by socket"
        fuser -k -9 "$GEN_PORT"/tcp 2>/dev/null || true
        ss -K "dst 127.0.0.1" 2>/dev/null || true
    else
        echo "[vitriol] no gen server on :$GEN_PORT"
    fi
    if [ -x "$COPULA_SCRIPT" ]; then
        "$COPULA_SCRIPT" stop
    fi
    echo "[vitriol] stopped"
    exit 0
}

status() {
    echo "[vitriol] status ($(date +%H:%M:%S))"
    echo "  gen     :$(health_of "$GEN_PORT")  pid=$(port_pid "$GEN_PORT")  log=$GEN_LOG"
    log_err "$GEN_LOG"
    echo "  hermetis:$(health_of "$HERM_PORT")  pid=$(port_pid "$HERM_PORT")  log=$HERM_LOG"
    echo "  embed   :$(health_of "$EMBED_PORT")  pid=$(port_pid "$EMBED_PORT")  log=$EMBED_LOG"
    log_err "$EMBED_LOG"
    echo "  gpu: $(nvidia-smi --query-gpu=memory.used,memory.free,utilization.gpu --format=csv,noheader 2>/dev/null || echo n/a)"
    echo ""
    echo "  logs: $0 logs [gen|hermetis|embed] [N]"
    exit 0
}

logs() {
    local comp="${1:-all}"
    local n="${2:-20}"
    case "$comp" in
        gen) log_tail "$GEN_LOG" "$n" ;;
        hermetis) log_tail "$HERM_LOG" "$n" ;;
        embed) log_tail "$EMBED_LOG" "$n" ;;
        all) echo "== gen =="; log_tail "$GEN_LOG" "$n"; echo "== hermetis =="; log_tail "$HERM_LOG" "$n"; echo "== embed =="; log_tail "$EMBED_LOG" "$n" ;;
        *) echo "usage: $0 logs [gen|hermetis|embed|all] [N]" >&2; exit 1 ;;
    esac
    exit 0
}

doctor() {
    echo "[vitriol] doctor"
    local fail=0
    chk() { if [ "$2" = "ok" ]; then echo "  PASS  $1"; else echo "  FAIL  $1"; fail=1; fi; }
    [ -x "$SERVER" ] && chk "binary $SERVER" ok || chk "binary $SERVER" missing
    [ -f "$GEN_MODEL" ] && chk "model $GEN_MODEL" ok || chk "model $GEN_MODEL" missing
    if [ -x "$SERVER" ]; then
        if ldd "$SERVER" 2>&1 | grep -q "not found"; then
            chk "shared libs (ldd)" "not-found:" && ldd "$SERVER" | grep "not found" | head -3
        else
            chk "shared libs (ldd)" ok
        fi
        getcap "$SERVER" 2>/dev/null | grep -q cap_ipc_lock && chk "cap_ipc_lock" ok || chk "cap_ipc_lock" "missing (sudo $VITRIOL_SCRIPT setup)"
        if readelf -d "$SERVER" 2>/dev/null | grep -q '\$ORIGIN'; then
            chk "RUNPATH (AT_SECURE)" "fresh \$ORIGIN — launch auto-fixes, or run setup"
        else
            chk "RUNPATH (AT_SECURE)" ok
        fi
    fi
    [ -z "$(port_pid "$GEN_PORT")" ] && chk "port :$GEN_PORT free" ok || chk "port :$GEN_PORT free" "in use"
    local dfree=$(df -BG "$LOG_DIR" 2>/dev/null | awk 'NR==2{print $4}' | tr -d 'G')
    if [ "${dfree:-0}" -ge 10 ]; then
        chk "disk free (>=10G)" ok
    else
        chk "disk free (>=10G)" "${dfree}G"
    fi
    [ "$fail" = "0" ] && echo "  --- all checks passed ---"
    exit $fail
}

[ "${1:-}" = "stop" ] && stop
[ "${1:-}" = "status" ] && status
[ "${1:-}" = "logs" ] && { shift; logs "$@"; }
[ "${1:-}" = "doctor" ] && doctor

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
        --verbose) VERBOSE=1 ;;
        --dry-run) DRY_RUN=1 ;;
    esac
done

echo "[vitriol] VITRIOL dir: $VITRIOL_DIR"
[ "$DRY_RUN" = "1" ] && echo "[vitriol] DRY RUN — no commands executed"

# ── Self-heal RUNPATH (no sudo) ───────────────────────────────────────────────
# A rebuild resets the binary RUNPATH to $ORIGIN, which the loader ignores under
# AT_SECURE (caps). patchelf it to the absolute path; caps are re-applied by setup.
if [ "$DRY_RUN" = "0" ] && [ -x "$SERVER" ] && needs_rpath; then
    echo "[vitriol] binary RUNPATH is \$ORIGIN (breaks under AT_SECURE) — fixing..."
    fix_rpath_local
fi

# ── Setup (caps) ──────────────────────────────────────────────────────────────
if [ "$DO_SETUP" = "1" ] && [ -x "$SERVER" ] && [ "$DRY_RUN" = "0" ]; then
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
    if [ -n "$(port_pid "$GEN_PORT")" ] || port_up "$GEN_PORT"; then
        echo "[vitriol] gen server already on :$GEN_PORT — skipping"
    else
        CMD=("$SERVER" -m "$GEN_MODEL" -ngl "$NGL" -c "$CTX" -t "$THREADS" \
             --parallel "$PARALLEL" --port "$GEN_PORT")
        echo "[vitriol] starting gen server on :$GEN_PORT ($(basename "$GEN_MODEL"), ngl=$NGL ctx=$CTX t=$THREADS p=$PARALLEL)"
        if [ "$VERBOSE" = "1" ] || [ "$DRY_RUN" = "1" ]; then
            echo "[vitriol]   cmd: ${CMD[*]}"
        fi
        if [ "$DRY_RUN" = "0" ]; then
            setsid nohup "${CMD[@]}" > "$GEN_LOG" 2>&1 < /dev/null &
            # Hardening: check PROCESS liveness, not port binding — the port only
            # binds after the ~50s model load, so a loading server is alive-but-unbound.
            # A dead-on-arrival launch (lib errors) dies within ~1s.
            alive=1
            for i in $(seq 1 6); do
                if ! kill -0 $! 2>/dev/null; then alive=0; break; fi
                sleep 1
            done
            if [ "$alive" = "1" ]; then
                echo "[vitriol]   gen process up (pid $!); model may still be loading (port binds after load)"
            else
                echo "[vitriol] ERROR: gen server exited immediately — see log tail:" >&2
                log_tail "$GEN_LOG" 8
                exit 1
            fi
        fi
    fi
fi

# ── Copula stack ──────────────────────────────────────────────────────────────
if [ "$DO_COPULA" = "1" ]; then
    if [ -x "$COPULA_SCRIPT" ]; then
        [ "$DRY_RUN" = "0" ] && "$COPULA_SCRIPT"
    else
        echo "[vitriol] WARN: $COPULA_SCRIPT not found — Copula skipped"
    fi
fi

# ── Verify ────────────────────────────────────────────────────────────────────
[ "$DRY_RUN" = "0" ] && sleep 3
echo ""
echo "[vitriol] status:"
if [ "$DO_GEN" = "1" ]; then
    echo "  gen:     $(health_of "$GEN_PORT")  (log: $GEN_LOG)"
    log_err "$GEN_LOG"
fi
if [ "$DO_COPULA" = "1" ]; then
    echo "  hermetis: $(health_of "$HERM_PORT")"
    echo "  embed:    $(health_of "$EMBED_PORT")"
fi
echo ""
echo "  OpenCode: restart it, then point the provider at http://127.0.0.1:$GEN_PORT/v1"
echo "  Diagnostics: $0 status | logs [gen|hermetis|embed] [N] | doctor | stop"
