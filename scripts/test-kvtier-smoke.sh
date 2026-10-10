#!/usr/bin/env bash
# kvtier smoke: warm-resume / trim-loudness / identity-mismatch checks against
# a throwaway CPU engine (small model, spare port, temp slot dir). Never
# touches the live engine or /scratch/kvtier.
#   T1 warm resume: spill -> SIGKILL -> relaunch -> prompt_n_cached > 0
#   T2 identity:    touch model -> relaunch -> mismatch WRN, files retained
#   T3 trim loud:   tiny cache-ram -> spill over limit -> "disk trim: removed"
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENGINE="$ROOT/llama.cpp/build/bin/llama-server"
MODEL="${KVTIER_SMOKE_MODEL:-$HOME/Models/qwen2.5-0.5b-instruct-q4_k_m.gguf}"
PORT="${KVTIER_SMOKE_PORT:-18279}"
URL="http://127.0.0.1:$PORT"

[[ -x "$ENGINE" ]] || { echo "missing engine: $ENGINE" >&2; exit 1; }
[[ -f "$MODEL" ]] || {
    echo "fetching smoke model..."
    python3 "$ROOT/scripts/fetch-hf-ranged.py" \
        Qwen/Qwen2.5-0.5B-Instruct-GGUF qwen2.5-0.5b-instruct-q4_k_m.gguf "$MODEL"
}

# preflight: smoke engines need real RAM; under a saturated host (live engine
# holds the 12 GiB prompt tier) the CPU prefill thrashes swap and false-fails
MEM_AVAIL_KB=$(awk '/MemAvailable/ {print $2}' /proc/meminfo)
if [[ "${MEM_AVAIL_KB:-0}" -lt 3145728 ]]; then
    echo "SKIP: MemAvailable=$((MEM_AVAIL_KB / 1024)) MiB < 3072 MiB (host saturated; retry when RAM frees)"
    exit 2
fi

TMP="$(mktemp -d /tmp/kvtier-smoke.XXXXXX)"
LOG="$TMP/engine.log"
ENGINE_PID=""
EXTRA_ARGS=()

cleanup() {
    [[ -n "$ENGINE_PID" ]] && kill -9 "$ENGINE_PID" 2>/dev/null || true
    rm -rf "$TMP"
}
trap cleanup EXIT

say() { printf '\n== %s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; echo "---- last 40 log lines ----"; tail -40 "$LOG"; exit 1; }

start_engine() { # $1.. extra env assignments as KEY=VAL, then flag passthrough via $EXTRA_ARGS
    local envs=()
    while [[ "${1:-}" == *=* ]]; do envs+=("$1"); shift; done
    env "${envs[@]}" "$ENGINE" \
        -m "$MODEL" -c "${NCTX:-4096}" -np 1 -t 2 -ngl 0 --device none -fit off \
        --port "$PORT" --host 127.0.0.1 --no-webui \
        --slot-save-path "$TMP/mirror" --cache-ram "${CRAM:-512}" \
        --ctx-checkpoints 4 "${EXTRA_ARGS[@]}" >"$LOG" 2>&1 &
    ENGINE_PID=$!
    local i=0
    until curl -sf "$URL/health" >/dev/null 2>&1; do
        i=$((i + 1))
        [[ $i -gt 120 ]] && fail "engine not healthy after 120s"
        kill -0 "$ENGINE_PID" 2>/dev/null || fail "engine died during boot (see log)"
        sleep 1
    done
}

stop_engine() { # SIGKILL on purpose: exercises crash-path persistence
    kill -9 "$ENGINE_PID" 2>/dev/null || true
    wait "$ENGINE_PID" 2>/dev/null || true
    ENGINE_PID=""
}

complete() { # $1=prompt -> echoes JSON response
    python3 - "$URL" "$1" <<'EOF'
import json, sys, urllib.request
url, prompt = sys.argv[1], sys.argv[2]
req = urllib.request.Request(url + "/completion",
    data=json.dumps({"prompt": prompt, "n_predict": 8, "temperature": 0}).encode(),
    headers={"Content-Type": "application/json"})
with urllib.request.urlopen(req, timeout=300) as r:
    print(r.read().decode())
EOF
}

cached_n() { python3 -c 'import json,sys; print(json.loads(sys.stdin.read())["timings"]["cache_n"])'; }

P1="The kvtier smoke prompt. $(printf 'context fill token. %.0s' {1..300})"
P2="An unrelated second conversation topic. $(printf 'filler text here. %.0s' {1..300})"

say "T1: warm resume (spill -> SIGKILL -> relaunch -> cached > 0)"
mkdir -p "$TMP/mirror"
start_engine
complete "$P1" >/dev/null          # conversation A prefills, slot holds it
complete "$P2" >/dev/null          # conversation B -> get_available_slot saves A; RAM budget 512 -> A spills
ls "$TMP/mirror"/cache-*.bin >/dev/null 2>&1 || fail "no mirror file after spill"
stop_engine

start_engine
R="$(complete "$P1")"
C="$(printf '%s' "$R" | cached_n)"
[[ "$C" -gt 100 ]] || fail "warm resume: cache_n=$C (want > 100) - disk restore or rehydrate broken"
say "T1 ok: cache_n=$C"

say "T2: identity mismatch retained (touch model, PRUNE=0 default)"
NF0=$(find "$TMP/mirror" -name 'cache-*.bin' | wc -l)
touch "$MODEL"
stop_engine
start_engine
complete "$P1" >/dev/null || true
grep -q "identity mismatch" "$LOG" || fail "no identity-mismatch WRN after model touch"
NF1=$(find "$TMP/mirror" -name 'cache-*.bin' | wc -l)
[[ "$NF1" -ge "$NF0" ]] || fail "mirror files deleted on identity mismatch (prune leak): $NF0 -> $NF1"
say "T2 ok: mismatch logged, files retained ($NF0 -> $NF1)"
stop_engine

say "T3: trim loudness (small cache-ram + VITRIOL_KV_DISK_TRIM_X=1)"
rm -rf "$TMP/mirror" && mkdir -p "$TMP/mirror"
# cache-ram is MiB; disk-trim limit = cache-ram * 12 MiB (base cap) * TRIM_X.
# CRAM=16 -> 192 MiB limit; each spilled state ~12 MiB -> trims by ~16 files
CRAM=16 start_engine VITRIOL_KV_DISK_TRIM_X=1
for i in $(seq 1 25); do
    complete "Conversation number $i. $(printf 'context fill token. %.0s' {1..250})" >/dev/null
done
grep -q "disk trim: removed" "$LOG" || fail "no 'disk trim: removed' WRN (trim not loud)"
say "T3 ok: trim WRN present"
stop_engine

say "T4: kv-policy endpoint (runtime eviction policy)"
start_engine
R=$(curl -s -X POST "$URL/kv-policy" -d '{"floor":0.10,"protect":512,"sweep":true}')
printf '%s' "$R" | python3 -c '
import json, sys
v = json.load(sys.stdin)
assert abs(v["floor"] - 0.10) < 1e-3, f"floor not applied: {v}"
assert v["protect"] == 512, f"protect not applied: {v}"
assert v["buffer_bytes"] > 0, f"buffer gauge missing: {v}"
'
R=$(curl -s -X POST "$URL/kv-policy" -d '{"floor":0}')
printf '%s' "$R" | python3 -c '
import json, sys
v = json.load(sys.stdin)
assert v["floor"] == 0, f"floor undo failed: {v}"
assert v["protect"] == 512, f"protect must persist when omitted: {v}"
'
say "T4 ok: kv-policy set/echo/persist"
stop_engine

say "T5: ctx resize round trip (spool -> swap -> warm restore)"
NCTX=8192 start_engine VITRIOL_CTX_RESIZE=1
complete "$P1" >/dev/null
R=$(curl -s -X POST "$URL/ctx/resize" -d '{"n_ctx":4096}')
printf '%s' "$R" | python3 -c '
import json, sys
v = json.load(sys.stdin)
assert v["n_ctx_old"] == 8192 and v["n_ctx_new"] == 4096, f"resize down failed: {v}"
assert v["n_spooled"] == 1, f"slot not spooled: {v}"
'
C="$(complete "$P1" | cached_n)"
[[ "$C" -gt 100 ]] || fail "post-resize warm restore: cache_n=$C"
R=$(curl -s -X POST "$URL/ctx/resize" -d '{"n_ctx":8192}')
printf '%s' "$R" | python3 -c '
import json, sys
v = json.load(sys.stdin)
assert v["n_ctx_old"] == 4096 and v["n_ctx_new"] == 8192, f"resize up failed: {v}"
'
C="$(complete "$P1" | cached_n)"
[[ "$C" -gt 100 ]] || fail "post-resize-up warm restore: cache_n=$C"
say "T5 ok: resize round trip, warm restore both ways"
stop_engine

say "ALL PASS"
