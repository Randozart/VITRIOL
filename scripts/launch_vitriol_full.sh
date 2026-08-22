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

# Ports come from the single source of truth (Tria Prima scheme; see vitriol-ports.sh).
# shellcheck source=vitriol-ports.sh
# shellcheck disable=SC1091
source "$VITRIOL_DIR/scripts/vitriol-ports.sh"
HERM_PORT="$VITRIOL_HERM_PORT"
EMBED_PORT="$VITRIOL_EMBED_PORT"

# Resolve a `key = value` from a named section of ~/.vitriol/config (comments and
# quotes stripped, whitespace-trimmed). Returns nonzero if section/key missing.
# Mirrors the extraction in scripts/vitriol:parse_config (vitriol:93-164).
cfg_value() {
    local cfg="$HOME/.vitriol/config" sec="" line="" key="" v=""
    [[ -f "$cfg" ]] || return 1
    while IFS= read -r line || [[ -n "$line" ]]; do
        line="${line%%\#*}"
        [[ -z "$line" ]] && continue
        if [[ "$line" =~ ^\[([^]]+)\]$ ]]; then
            sec="${BASH_REMATCH[1]}"
            continue
        fi
        [[ "$sec" != "$1" ]] && continue
        if [[ "$line" =~ ^"$2"[[:space:]]*=[[:space:]]*(.*)$ ]]; then
            v="${BASH_REMATCH[1]}"
            v="${v#"${v%%[![:space:]]*}"}"   # strip leading spaces/tabs
            v="${v%"${v##*[![:space:]]}"}"   # strip trailing spaces/tabs
            v="${v#\"}"; v="${v%\"}"; v="${v#\'}"; v="${v%\'}"
            [[ -n "$v" ]] && { echo "$v"; return 0; }
        fi
    done < "$cfg"
    return 1
}

# Defaults — but the ~/.vitriol/config `[model]`/`[server]` sections are the source
# of truth, exactly like `vitriol serve` (vitriol:116-125, 1897-1906). Previously
# the launch hardcoded the Mellum2-Claude-Thinking Q2_K defaults and IGNORED the
# config model/ctx, so a balanced-profile (Qwen, 136K ctx) launch spun up the
# wrong model at the wrong context. Caller-set VITRIOL_* env always wins, then
# config, then the built-in defaults.
# 2026-08-08: config-dir resolution added to match `vitriol serve` behaviour.
GEN_MODEL="${VITRIOL_GEN_MODEL:-$(cfg_value model path || true)}"
GEN_PORT="${VITRIOL_GEN_PORT:-$(cfg_value server port || true)}"
NGL="${VITRIOL_NGL:-$(cfg_value model ngl || true)}"
CTX="${VITRIOL_CTX:-$(cfg_value model context || true)}"
THREADS="${VITRIOL_THREADS:-$(cfg_value model threads || true)}"
PARALLEL="${VITRIOL_PARALLEL:-$(cfg_value server parallel || true)}"
# 2026-08-08: `parallel` splits the context across N slots (32768/4 = 8192/slot).
# An agent client (opencode) declares the full model context, so a slot shorter
# than the client's context causes 400 "exceeds available context size" loops.
# Exposed here so the config [server] parallel is the single source of truth.
GEN_MODEL="${GEN_MODEL:-/home/randozart/Desktop/Projects/mellum2-claude-Q2_K.gguf}"
GEN_PORT="${GEN_PORT:-8279}"
NGL="${NGL:-99}"
CTX="${CTX:-32768}"
THREADS="${THREADS:-4}"
# Single opencode session = 1 slot -> --parallel 1 gives the full CTX to that slot
# (parallel>1 splits it, shrinking effective context). ctx-shift rolls the window.
PARALLEL="${PARALLEL:-1}"
[[ -n "$GEN_MODEL" ]] && [[ ! -f "$GEN_MODEL" ]] && {
    echo "[vitriol] ERROR: model not found: $GEN_MODEL (set via config [model] path or VITRIOL_GEN_MODEL)" >&2
    exit 1
}

DO_SETUP=1
DO_COPULA=1
DO_GEN=1
VERBOSE=0
DRY_RUN=0

# Wire the ~/.vitriol/config memory-architecture settings into the server env.
# Mirrors scripts/vitriol's `exec env VITRIOL_*` mapping (~lines 1774-1797) so the
# launch (TUI) path activates the same RAM-Shot/stream strategy as `vitriol serve`.
# 2026-08-07: the launch path ignored these, so stream mode defaulted off — 12 GB
# MoE models attempted a full CUDA alloc and OOM'd (or stalled pre-mlock) instead
# of streaming experts. Caller-set VITRIOL_* env always wins over config.
apply_vitriol_env() {
    local cfg="$HOME/.vitriol/config" sec="" k v env_name=""
    [[ -f "$cfg" ]] || return 0
    while IFS= read -r line || [[ -n "$line" ]]; do
        case "$line" in
            \[*\]) sec="${line#\[}"; sec="${sec%\]}"; sec="${sec// /}" ;;
            *=*)
                k="${line%%=*}"; k="${k// /}"
                v="${line#*=}"; v="${v%%\#*}"; v="${v// /}"
                [[ -z "$v" ]] && continue
                case "$sec:$k" in
                    vitriol:mode)                 env_name="VITRIOL_MODE" ;;
                    vitriol:lru_mb)               env_name="VITRIOL_LRU_MB" ;;
                    vitriol:output_cache)         env_name="VITRIOL_OUTPUT_CACHE" ;;
                    vitriol:predictive_prefetch)  env_name="VITRIOL_PREDICTIVE_PREFETCH" ;;
                    vitriol:pin_first_n_layers)   env_name="VITRIOL_PIN_FIRST_N_LAYERS" ;;
                    vitriol:prune_experts)        env_name="VITRIOL_PRUNE_EXPERTS" ;;
                    vitriol:reasoning)             env_name="VITRIOL_REASONING" ;;
                    vitriol:verbose)              env_name="VITRIOL_VERBOSE" ;;
                    vitriol:early_exit)           env_name="VITRIOL_EARLY_EXIT" ;;
                    vitriol:early_exit_threshold) env_name="VITRIOL_EARLY_EXIT_THRESHOLD" ;;
                    vitriol:early_exit_stagnation) env_name="VITRIOL_EARLY_EXIT_STAGNATION" ;;
                    vitriol:early_exit_min_layers) env_name="VITRIOL_EARLY_EXIT_MIN_LAYERS" ;;
                    memory:mode)                  env_name="VITRIOL_MEMORY_MODE" ;;
                    memory:semantic_mode)         env_name="VITRIOL_SEMANTIC_MODE" ;;
                    kv:mode)                      env_name="VITRIOL_KV_MODE" ;;
                    kv:quant_mode)                env_name="VITRIOL_KV_QUANT" ;;
                    kv:frozen_prompt)             env_name="VITRIOL_FROZEN_PROMPT" ;;
                    engine:mode)                  env_name="VITRIOL_ENGINE_MODE" ;;
                    model:expert_count)           env_name="VITRIOL_EXPERT_COUNT" ;;
                    model:disk_offload)           env_name="VITRIOL_DISK_OFFLOAD" ;;
                    chimera:mode)                 env_name="VITRIOL_CHIMERA_MODE" ;;
                    lookup:tokens)                env_name="VITRIOL_LOOKUP" ;;
                    sampling:repeat_penalty)      env_name="VITRIOL_SAMPLING_REPEAT_PENALTY" ;;
                    sampling:dry_multiplier)      env_name="VITRIOL_SAMPLING_DRY_MULTIPLIER" ;;
                    sampling:dry_base)            env_name="VITRIOL_SAMPLING_DRY_BASE" ;;
                    sampling:dry_allowed_length)  env_name="VITRIOL_SAMPLING_DRY_ALLOWED_LENGTH" ;;
                    sampling:dry_penalty_last_n)  env_name="VITRIOL_SAMPLING_DRY_PENALTY_LAST_N" ;;
                    sampling:top_k)               env_name="VITRIOL_SAMPLING_TOP_K" ;;
                    sampling:top_p)               env_name="VITRIOL_SAMPLING_TOP_P" ;;
                    sampling:min_p)               env_name="VITRIOL_SAMPLING_MIN_P" ;;
                    *) env_name="" ;;
                esac
                [[ -z "$env_name" ]] && continue
                # Strict-flag booleans: the binary only honours literal "1".
                case "$env_name" in
                    VITRIOL_VERBOSE|VITRIOL_PREDICTIVE_PREFETCH|VITRIOL_OUTPUT_CACHE|VITRIOL_EARLY_EXIT|VITRIOL_DISK_OFFLOAD)
                        case "$v" in on|true|1) v="1" ;; *) continue ;; esac ;;
                esac
                if [[ -z "${!env_name:-}" ]]; then
                    export "$env_name=$v"
                    [[ "$VERBOSE" = "1" ]] && echo "[vitriol]   $env_name=$v (from $cfg)"
                fi
                ;;
        esac
    done < "$cfg"
    return 0
}

# Find the pid bound to a port. ss -p often cannot attribute the pid (it showed
# "-" for our own gen server), so fall back to lsof then fuser. Always returns 0 so
# set -e never aborts the script on a lookup miss.
port_pid() {
    local port="$1" p=""
    p=$(ss -ltnp 2>/dev/null | grep ":$port " | grep -oE 'pid=[0-9]+' | head -1 | cut -d= -f2)
    if [ -n "$p" ]; then echo "$p"; return 0; fi
    # 2026-08-07: ss -ltnp can fail to attribute our servers' pids (observed on
    # 8279/4779). Never fall back to lsof -ti:/fuser here — those list CLIENT
    # holders too, so stop() would kill -9 the TUI's keep-alive poller socket.
    # The cmdline always carries --port, so pgrep by binary+port is precise and
    # never touches non-server processes. If nothing matches, report no pid.
    p=$(pgrep -f "(llama-server|hermes-server|hermetis_server).*--port $port" 2>/dev/null | head -1)
    if [ -n "$p" ]; then echo "$p"; return 0; fi
    echo ""
    return 1
}
port_up() { [ "$(health_of "$1")" != "down" ]; }
# Retrying health check: a just-restarted server may answer nothing for a
# second; retry briefly before declaring "down" (2026-08-07: status wrongly
# reported down right after a restart).
health_of() {
    local out=""
    for _ in 1 2 3; do
        out=$(curl -s -m 2 "http://127.0.0.1:$1/health" 2>/dev/null)
        if [ -n "$out" ]; then echo "$out"; return 0; fi
        sleep 1
    done
    echo down
}
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
    else
        # 2026-08-07: never fuser -k / ss -K here — both kill every process
        # holding a localhost socket (incl. the TUI's poller). A live listener
        # with no attributed pid is reported, not nuked.
        echo "[vitriol] no gen server pid found on :$GEN_PORT"
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
    local comp="all" n="20" follow=0
    for a in "$@"; do
        case "$a" in
            --follow|-f) follow=1 ;;
            all|gen|hermetis|embed) comp="$a" ;;
            *[0-9]*) n="$a" ;;
            *) echo "usage: $0 logs [gen|hermetis|embed|all] [N] [--follow|-f]" >&2; exit 1 ;;
        esac
    done
    tailit() {
        if [ -f "$1" ]; then
            if [ "$3" = "1" ]; then tail -n "$2" -f "$1"; else tail -n "$2" "$1"; fi
        else
            echo "  (no log at $1)"
        fi
    }
    case "$comp" in
        gen) tailit "$GEN_LOG" "$n" "$follow" ;;
        hermetis) tailit "$HERM_LOG" "$n" "$follow" ;;
        embed) tailit "$EMBED_LOG" "$n" "$follow" ;;
        all)
            if [ "$follow" = "1" ]; then
                tail -n "$n" -f "$GEN_LOG" "$HERM_LOG" "$EMBED_LOG" 2>/dev/null
            else
                echo "== gen =="; tailit "$GEN_LOG" "$n" 0
                echo "== hermetis =="; tailit "$HERM_LOG" "$n" 0
                echo "== embed =="; tailit "$EMBED_LOG" "$n" 0
            fi
            ;;
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

help() {
    cat <<EOF
vitriol — full VITRIOL stack: setup (caps) + gen server + Copula memory.

USAGE
  vitriol [COMMAND] [FLAGS]

COMMANDS
  (none)            launch the full stack (self-heals RUNPATH, then gen + Copula)
  stop              stop gen server + Copula (Hermetis + embed)
  status            live diagnostics: per-component health, pid, log, GPU, fatal errors
  logs [c] [N] [--follow|-f]   tail logs; c = gen|hermetis|embed|all (default all),
                     N = last N lines (default 20), --follow/-f = live tail
  doctor            pre-flight checks: binary, model, ldd, caps, RUNPATH, ports, disk
  help | -h | --help  this message

FLAGS (launch)
  --model=PATH      gen model (default: Mellum2-12B-A2.5B-Instruct-Q4_K_M.gguf)
  --ngl=N           gpu layers (default 24)
  --ctx=N           context (default 32768)
  --threads=N       threads (default 4)
  --parallel=N      parallel slots (default 2)
  --gen-port=P      gen port (default 8279)
  --no-copula       gen server only
  --no-setup        skip the sudo caps step
  --copula-only     memory stack only
  --verbose         print the exact launch command
  --dry-run         print what would launch, run nothing

EXAMPLES
  vitriol                       full launch
  vitriol status                what is running + logs + gpu
  vitriol logs gen --follow     watch the gen server live
  vitriol doctor                pre-flight checks
  vitriol stop                  tear everything down
EOF
    exit 0
}

[ "${1:-}" = "help" ] || [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ] && help
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
    apply_vitriol_env
    # Serve-parity env: vitriol `serve` exports these defaults unconditionally
    # (scripts/vitriol ~1776-1799); the launch path must set the same so the server
    # sees them even when the config lacks the keys. 2026-08-08: added so launch == serve.
    export VITRIOL_MODEL_PATH="${VITRIOL_MODEL_PATH:-$GEN_MODEL}"
    export VITRIOL_DISK_OFFLOAD="${VITRIOL_DISK_OFFLOAD:-0}"
    export VITRIOL_EARLY_EXIT="${VITRIOL_EARLY_EXIT:-0}"
    export VITRIOL_EARLY_EXIT_THRESHOLD="${VITRIOL_EARLY_EXIT_THRESHOLD:-0.001}"
    export VITRIOL_EARLY_EXIT_MIN_LAYERS="${VITRIOL_EARLY_EXIT_MIN_LAYERS:-10}"
    export VITRIOL_EARLY_EXIT_STAGNATION="${VITRIOL_EARLY_EXIT_STAGNATION:-3}"
    export VITRIOL_REASONING="${VITRIOL_REASONING:-off}"
    # 2026-08-08: reasoning is a per-model trait (Qwen3.6 needs `off`). The
    # old launch forced `--reasoning-format deepseek` unconditionally, which
    # routed all output into message.reasoning_content and left content empty.
    # Match vitriol serve (vitriol:2006) — `--reasoning off` for config `off`.
    [ "$VITRIOL_REASONING" = "off" ] && REASONING_ARGS="--reasoning off" || REASONING_ARGS=""
    # 2026-08-08: sampling guards, tunable via [sampling] in ~/.vitriol/config.
    # IQ2_M-class quants (2.7bpw) fall into a </tool_call> repetition attractor
    # with no repeat penalty or DRY guard — the "looped again" failure. Defaults
    # match llama-server's stock (penalty 1.0 = off, DRY 0.0 = off) so the config
    # is the single source of truth; enable them by setting the values.
    export VITRIOL_SAMPLING_REPEAT_PENALTY="${VITRIOL_SAMPLING_REPEAT_PENALTY:-1.0}"
    export VITRIOL_SAMPLING_DRY_MULTIPLIER="${VITRIOL_SAMPLING_DRY_MULTIPLIER:-0.0}"
    export VITRIOL_SAMPLING_DRY_BASE="${VITRIOL_SAMPLING_DRY_BASE:-1.75}"
    export VITRIOL_SAMPLING_DRY_ALLOWED_LENGTH="${VITRIOL_SAMPLING_DRY_ALLOWED_LENGTH:-2}"
    export VITRIOL_SAMPLING_DRY_PENALTY_LAST_N="${VITRIOL_SAMPLING_DRY_PENALTY_LAST_N:--1}"
    export VITRIOL_SAMPLING_TOP_K="${VITRIOL_SAMPLING_TOP_K:-40}"
    export VITRIOL_SAMPLING_TOP_P="${VITRIOL_SAMPLING_TOP_P:-0.95}"
    export VITRIOL_SAMPLING_MIN_P="${VITRIOL_SAMPLING_MIN_P:-0.05}"
    SAMPLING_ARGS=""
    for kv in "repeat-penalty:$VITRIOL_SAMPLING_REPEAT_PENALTY" \
              "dry-multiplier:$VITRIOL_SAMPLING_DRY_MULTIPLIER" \
              "dry-base:$VITRIOL_SAMPLING_DRY_BASE" \
              "dry-allowed-length:$VITRIOL_SAMPLING_DRY_ALLOWED_LENGTH" \
              "dry-penalty-last-n:$VITRIOL_SAMPLING_DRY_PENALTY_LAST_N" \
              "top-k:$VITRIOL_SAMPLING_TOP_K" \
              "top-p:$VITRIOL_SAMPLING_TOP_P" \
              "min-p:$VITRIOL_SAMPLING_MIN_P"; do
        flag="${kv%%:*}"; val="${kv#*:}"
        case "$flag:$val" in
            repeat-penalty:1.0|dry-multiplier:0.0) ;;  # stock = off, skip
            *) SAMPLING_ARGS="$SAMPLING_ARGS --$flag $val" ;;
        esac
    done
    # 2026-08-18: wire the config's [gpu]/[kv]/[spec] settings into server flags.
    # Before this, the TUI launch ignored tensor_split, KV quant, and MTP spec,
    # so a 131K ctx Qwen3.8 ran with default f16 KV + no -ts -> context OOM
    # ("cannot create context"). Mirrors `vitriol serve` (vitriol:1960-2010).
    TENSOR_SPLIT="${VITRIOL_TENSOR_SPLIT:-$(cfg_value gpu tensor_split || true)}"
    KV_QUANT_K="${VITRIOL_KV_QUANT_K:-$(cfg_value kv quant_mode || true)}"
    KV_QUANT_V="${VITRIOL_KV_QUANT_V:-$(cfg_value kv quant_mode_v || true)}"
    SPEC_TYPE="${VITRIOL_SPEC_TYPE:-$(cfg_value spec type || true)}"
    SPEC_DRAFT_N_MAX="${VITRIOL_SPEC_DRAFT_N_MAX:-$(cfg_value spec draft_n_max || true)}"
    # default ubatch: 128 is the tuned optimum for MTP on this 2-GPU pair
    # (512 makes the MTP pp compute buffer OOM the 1070 Ti).
    UBATCH="${VITRIOL_UBATCH:-$(cfg_value model ubatch || true)}"
    UBATCH="${UBATCH:-128}"

    TS_ARGS=()
    [[ -n "$TENSOR_SPLIT" ]] && TS_ARGS=(-ts "$TENSOR_SPLIT" --main-gpu 0)
    KV_ARGS=()
    [[ -n "$KV_QUANT_K" ]] && KV_ARGS+=(--cache-type-k "$KV_QUANT_K")
    [[ -n "$KV_QUANT_V" ]] && KV_ARGS+=(--cache-type-v "$KV_QUANT_V")
    SPEC_ARGS=()
    [[ -n "$SPEC_TYPE" ]] && SPEC_ARGS+=(--spec-type "$SPEC_TYPE")
    if [[ -n "$SPEC_DRAFT_N_MAX" ]] && [[ "$SPEC_DRAFT_N_MAX" != "0" ]]; then
        SPEC_ARGS+=(--spec-draft-n-max "$SPEC_DRAFT_N_MAX")
    fi

    if [ ! -x "$SERVER" ]; then
        echo "[vitriol] ERROR: $SERVER not found — build first" >&2
        exit 1
    fi
    if [ -n "$(port_pid "$GEN_PORT")" ] || port_up "$GEN_PORT"; then
        echo "[vitriol] gen server already on :$GEN_PORT — skipping"
    else
        CMD=("$SERVER" -m "$GEN_MODEL" -ngl "$NGL" -c "$CTX" -t "$THREADS" \
             --no-mmap \
             --parallel "$PARALLEL" $REASONING_ARGS $SAMPLING_ARGS --flash-attn on --jinja \
             "${TS_ARGS[@]}" "${KV_ARGS[@]}" "${SPEC_ARGS[@]}" -ub "$UBATCH" \
             --context-shift --cache-reuse 256 --slots --metrics --port "$GEN_PORT")
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
