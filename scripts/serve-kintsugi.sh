#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
server="${script_dir}/../engine/build/bin/vitriol-server"
model_dir="${VITRIOL_KINTSUGI_MODELS:-$HOME/kintsugi/models}"
model="$model_dir/Kintsugi-27B-12G.gguf"
draft="$model_dir/dflash-Qwen3.8-27B-Q4_0.gguf"
port="${VITRIOL_KINTSUGI_PORT:-8279}"
slot_dir="${VITRIOL_KINTSUGI_SLOT_DIR:-}"

export CUDA_DEVICE_ORDER=PCI_BUS_ID
export CUDA_VISIBLE_DEVICES="${VITRIOL_KINTSUGI_GPU:-1}"
export VITRIOL_MODE=off LLAMA_HEAD2=512 GGML_CUDA_SHARED_POOL=1
# Keep the recurrent snapshots on device. Tier KV to leave room for both models.
export VITRIOL_RS_VRAM_FRACTION=1 VITRIOL_KV_VRAM_CELLS=32768
export LLAMA_RS_REPLAY=0 VITRIOL_RS_REPLAY_OP=0

if ! { [[ $# == 0 ]] || [[ $# == 1 && $1 == --dry-run ]] || [[ $# == 3 && $1 == --detach ]]; }; then
    printf 'Usage: %s [--dry-run | --detach LOG PIDFILE]\n' "$0" >&2
    exit 2
fi

slot_args=()
if [[ -n "$slot_dir" ]]; then
    slot_args=(--slot-save-path "$slot_dir")
fi

argv=("$server" --model "$model" --alias "Lapis Occultus" --host 127.0.0.1 --port "$port"
    --fit off -c 65536 -b 512 -ub 128 -ngl 99 --device CUDA0 --split-mode none
    -np 1 -t 4 -fa on -ctk q4_0 -ctv q4_0 --swa-full --ctx-checkpoints 4 --cache-ram 256 --reasoning off
    --model-draft "$draft" --spec-draft-device CUDA0 -ngld 99 --spec-type draft-dflash
    --spec-draft-n-max 7 -ctkd q4_0 -ctvd q4_0 --slots --metrics "${slot_args[@]}")

fingerprint() {
    printf 'VITRIOL-FINGERPRINT: gpu=%s mode=%s head2=%s pool=%s rs=%s kvc=%s replay=%s argv=' \
        "$CUDA_VISIBLE_DEVICES" "$VITRIOL_MODE" "$LLAMA_HEAD2" "$GGML_CUDA_SHARED_POOL" \
        "$VITRIOL_RS_VRAM_FRACTION" "$VITRIOL_KV_VRAM_CELLS" "$LLAMA_RS_REPLAY"
    printf '%q ' "${argv[@]}"
    printf '\n'
}

if [[ ${1:-} == --dry-run ]]; then
    fingerprint
    exit 0
fi
for file in "$model" "$draft"; do
    if [[ ! -r "$file" ]]; then
        printf 'Kintsugi model is unreadable: %s\n' "$file" >&2
        exit 1
    fi
done
if [[ ! -x "$server" ]]; then
    printf 'VITRIOL server binary is unavailable: %s\n' "$server" >&2
    exit 1
fi
if [[ -n "$slot_dir" && ( ! -d "$slot_dir" || ! -w "$slot_dir" ) ]]; then
    printf 'Kintsugi slot directory is unavailable or unwritable: %s\n' "$slot_dir" >&2
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
