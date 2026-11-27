#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
server="${script_dir}/../engine/build/bin/vitriol-server"
model_dir="${VITRIOL_KINTSUGI_MODELS:-$HOME/kintsugi/models}"
model="$model_dir/Kintsugi-27B-12G.gguf"
draft="$model_dir/dflash-Qwen3.8-27B-Q4_0.gguf"
port="${VITRIOL_KINTSUGI_PORT:-8279}"
slot_dir="${VITRIOL_KINTSUGI_SLOT_DIR:-}"
profile="${VITRIOL_KINTSUGI_PROFILE:-kintsugi}"

export CUDA_DEVICE_ORDER=PCI_BUS_ID
export CUDA_VISIBLE_DEVICES="${VITRIOL_KINTSUGI_GPU:-1}"
export LLAMA_HEAD2=512 GGML_CUDA_SHARED_POOL=1

if ! { [[ $# == 0 ]] || [[ $# == 1 && $1 == --dry-run ]] || [[ $# == 3 && $1 == --detach ]]; }; then
    printf 'Usage: %s [--dry-run | --detach LOG PIDFILE]\n' "$0" >&2
    exit 2
fi

slot_args=()
if [[ -n "$slot_dir" ]]; then
    slot_args=(--slot-save-path "$slot_dir")
fi

required_files=("$model")
case "$profile" in
    kintsugi)
        export VITRIOL_MODE=off VITRIOL_RS_VRAM_FRACTION=1 VITRIOL_KV_VRAM_CELLS=32768
        export LLAMA_RS_REPLAY=0 VITRIOL_RS_REPLAY_OP=0
        required_files+=("$draft")
        argv=("$server" --model "$model" --alias "Lapis Occultus" --host 127.0.0.1 --port "$port"
            --fit off -c 65536 -b 512 -ub 128 -ngl 99 --device CUDA0 --split-mode none
            -np 1 -t 4 -fa on -ctk q4_0 -ctv q4_0 --swa-full --ctx-checkpoints 4 --cache-ram 256 --reasoning off
            --model-draft "$draft" --spec-draft-device CUDA0 -ngld 99 --spec-type draft-dflash
            --spec-draft-n-max 7 -ctkd q4_0 -ctvd q4_0 --slots --metrics "${slot_args[@]}")
        ;;
    kintsugi-reference)
        runtime="${VITRIOL_KINTSUGI_REFERENCE_DIR:-$HOME/kintsugi/Kintsugi-27B}"
        server="$runtime/bin/llama-server"
        template="$runtime/templates/kintsugi.jinja"
        required_files+=("$template")
        export LD_LIBRARY_PATH="$runtime/bin"
        export GGML_ESCHA_PREFILL=i8 GGML_ESCHA_I8_MIN_ROWS=256 GGML_CUDA_BATCH_INVARIANT=1
        export GGML_CUDA_POOL_RESERVE_MIB=96 GGML_CUDA_DEDICATED_CEILING_MIB=11500
        export LLAMA_RS_REPLAY=1 LLAMA_HEAD2_TYPE=q4_0 LLAMA_MTP_DRAFT_UBATCH=256
        export LLAMA_ARG_CHECKPOINT_EVERY_NT=8192
        argv=("$server" -m "$model" --alias "Lapis Occultus" --host 127.0.0.1 --port "$port"
            --fit off -c 229376 -np 1 -ngl 99 -t 6 -b 1024 -ub 512 --device CUDA0 --split-mode none
            -fa on -ctk q4_0 -ctv q4_0 --no-warmup --ctx-checkpoints 4 --cache-ram 256 --reasoning off
            --slots --metrics --kv-vram-cells 112640 --kq-mask-packed --spec-type "ngram-mod,draft-mtp"
            --spec-draft-n-max 3 --spec-draft-n-max-tail 4 --spec-draft-window 16384 --spec-lookup-n-max 32
            -ctkd q8_0 -ctvd q8_0 "${slot_args[@]}" --chat-template-file "$template")
        ;;
    *)
        printf 'Unsupported Kintsugi profile: %s\n' "$profile" >&2
        exit 2
        ;;
esac

fingerprint() {
    if [[ "$profile" == "kintsugi-reference" ]]; then
        printf 'VITRIOL-FINGERPRINT: gpu=%s profile=%s escha=%s escha_min_rows=%s batch_invariant=%s head2=%s head2_type=%s pool=%s pool_reserve=%s ceiling=%s replay=%s mtp_ub=%s ckpt_every=%s libpath=%s argv=' \
            "$CUDA_VISIBLE_DEVICES" "$profile" "$GGML_ESCHA_PREFILL" "$GGML_ESCHA_I8_MIN_ROWS" "$GGML_CUDA_BATCH_INVARIANT" \
            "$LLAMA_HEAD2" "$LLAMA_HEAD2_TYPE" "$GGML_CUDA_SHARED_POOL" "$GGML_CUDA_POOL_RESERVE_MIB" \
            "$GGML_CUDA_DEDICATED_CEILING_MIB" "$LLAMA_RS_REPLAY" "$LLAMA_MTP_DRAFT_UBATCH" "$LLAMA_ARG_CHECKPOINT_EVERY_NT" \
            "$LD_LIBRARY_PATH"
    else
        printf 'VITRIOL-FINGERPRINT: gpu=%s mode=%s head2=%s pool=%s rs=%s kvc=%s replay=%s argv=' \
            "$CUDA_VISIBLE_DEVICES" "$VITRIOL_MODE" "$LLAMA_HEAD2" "$GGML_CUDA_SHARED_POOL" \
            "$VITRIOL_RS_VRAM_FRACTION" "$VITRIOL_KV_VRAM_CELLS" "$LLAMA_RS_REPLAY"
    fi
    printf '%q ' "${argv[@]}"
    printf '\n'
}

if [[ ${1:-} == --dry-run ]]; then
    fingerprint
    exit 0
fi
for file in "${required_files[@]}"; do
    if [[ ! -r "$file" ]]; then
        printf 'Required Kintsugi file is unreadable: %s\n' "$file" >&2
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
