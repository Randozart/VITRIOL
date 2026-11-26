#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
engine="${VITRIOL_DECISION_SERVER:-$root/engine/build/bin/vitriol-server}"
model="${VITRIOL_DECISION_GGUF:-$HOME/Models/qwen2.5-0.5b-instruct-q4_k_m.gguf}"
alias="${VITRIOL_DECISION_MODEL:-qwen2.5-0.5b-decision}"
port="${VITRIOL_DECISION_PORT:-18435}"

export CUDA_DEVICE_ORDER=PCI_BUS_ID
export CUDA_VISIBLE_DEVICES="${VITRIOL_DECISION_GPU:-0}"
export VITRIOL_MODE=off

argv=("$engine" -m "$model" --alias "$alias"
    --host 127.0.0.1 --port "$port" --cors-origins localhost --device CUDA0 --split-mode none
    --fit off -ngl 99 -c 4096 -b 256 -ub 128 -np 1 -t 4 -fa on
    --ctx-checkpoints 4 --cache-ram 256 --reasoning off)

printf 'VITRIOL-FINGERPRINT: gpu=%s mode=%s argv=' "$CUDA_VISIBLE_DEVICES" "$VITRIOL_MODE"
printf '%q ' "${argv[@]}"
printf '\n'
case "${1:-}" in
    --dry-run) exit 0 ;;
    '') exec "${argv[@]}" ;;
    *) printf 'Usage: %s [--dry-run]\n' "$0" >&2; exit 2 ;;
esac
