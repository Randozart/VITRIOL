#!/usr/bin/env bash
# Inkling 2 Mini launcher (upstream b12774 runtime) — measured ~208K window.
# Argv mirrors the 2026-11-26 capacity bracket: layer split 1,3, ub 16,
# flash attention on, Q4_0 K/V KV. Keep the alias constant for Officina.
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
runtime="${VITRIOL_INKLING_RUNTIME_DIR:-$HOME/.local/share/vitriol/runtimes/llama-b12774}"
server="$runtime/llama-server"
model="${VITRIOL_INKLING_MODEL:-$HOME/Models/inkling-2-mini/Inkling-2-Mini-IQ4_NL.gguf}"
port="${VITRIOL_INKLING_PORT:-8279}"
ctx="${VITRIOL_INKLING_CTX:-212992}"
ub="${VITRIOL_INKLING_UB:-16}"
slot_dir="${VITRIOL_INKLING_SLOT_DIR:-}"
alias="${VITRIOL_INKLING_ALIAS:-Lapis Occultus}"

export CUDA_DEVICE_ORDER=PCI_BUS_ID
export CUDA_VISIBLE_DEVICES="${VITRIOL_INKLING_GPU:-0,1}"
export LD_LIBRARY_PATH="$runtime${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

if ! { [[ $# == 0 ]] || [[ $# == 1 && $1 == --dry-run ]] || [[ $# == 3 && $1 == --detach ]]; }; then
    printf 'Usage: %s [--dry-run | --detach LOG PIDFILE]\n' "$0" >&2
    exit 2
fi

slot_args=()
if [[ -n "$slot_dir" ]]; then
    slot_args=(--slot-save-path "$slot_dir")
fi

argv=("$server" --model "$model" --alias "$alias" --host 127.0.0.1 --port "$port"
    --fit off -c "$ctx" -b 512 -ub "$ub" -ngl 99 --device CUDA0,CUDA1 --split-mode layer
    -ts 1,3 -np 1 -t 4 -fa on -ctk q4_0 -ctv q4_0 --no-warmup --jinja
    --ctx-checkpoints 4 --cache-ram 256
    --chat-template-kwargs '{"reasoning_effort":"minimal"}'
    --slots --metrics -lv 4 "${slot_args[@]}")

fingerprint() {
    printf 'VITRIOL-FINGERPRINT: launcher=inkling-upstream build=b12774 gpu=%s ctx=%s ub=%s kv=q4_0/q4_0 split=1,3 model=%s argv=' \
        "$CUDA_VISIBLE_DEVICES" "$ctx" "$ub" "$(basename "$model")"
    printf '%q ' "${argv[@]}"
    printf '\n'
}

if [[ ${1:-} == --dry-run ]]; then
    fingerprint
    exit 0
fi

if [[ ! -r "$model" ]]; then
    printf 'Required Inkling model is unreadable: %s\n' "$model" >&2
    exit 1
fi
if [[ ! -x "$server" ]]; then
    printf 'Inkling runtime is unavailable: %s\n' "$server" >&2
    exit 1
fi
if [[ -n "$slot_dir" && ( ! -d "$slot_dir" || ! -w "$slot_dir" ) ]]; then
    printf 'Inkling slot directory is unavailable or unwritable: %s\n' "$slot_dir" >&2
    exit 1
fi

bash "$script_dir/engine-launch-gate.sh" pre
fingerprint
if [[ ${1:-} == --detach ]]; then
    fingerprint > "$2"
    nohup "${argv[@]}" </dev/null >> "$2" 2>&1 &
    printf '%s\n' "$!" > "$3"
    exit 0
fi
exec "${argv[@]}"
