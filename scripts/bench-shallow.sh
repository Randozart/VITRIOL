#!/bin/bash
# VITRIOL shallow bench: 1 warmup + N measured greedy 64-token generations.
# Usage: bench-shallow.sh <label> [runs=3]
# Prints fingerprint, median predicted_per_second, prompt t/s, cache_n,
# per-GPU VRAM, and SHA-256 of each output (greedy lossless check).
set -u

LABEL="${1:?usage: bench-shallow.sh <label> [runs]}"
RUNS="${2:-3}"
BASE="http://127.0.0.1:8279"
PORT="${PORT:-8279}"

# Fixed ~1.2k-token prompt: deterministic, spec-load enough to exercise draft.
PROMPT=$(python3 - <<'EOF'
print("Write a Python function that merges two sorted linked lists, "
      "then walk through its invariants step by step.\n\n" +
      "\n".join(f"Step {i}: consider node pair ({i}, {i+1}) and the merge "
                f"condition when left.val <= right.val." for i in range(80)))
EOF
)

echo "VITRIOL-FINGERPRINT: $(grep -m1 VITRIOL-FINGERPRINT /tmp/opencode/vitriol_gen.log 2>/dev/null || echo '(no fingerprint line in engine log)')"
echo "bench label: $LABEL runs: $RUNS"

json_field() { python3 -c "import json,sys; d=json.load(sys.stdin); print(eval('d' + sys.argv[1]))" "$1"; }

run_once() {
  python3 - "$BASE" "$PROMPT" <<'EOF'
import hashlib, json, sys, time, urllib.request
base, prompt = sys.argv[1], sys.argv[2]

def once():
    req = urllib.request.Request(
        base + "/completion",
        data=json.dumps({
            "prompt": prompt, "n_predict": 64,
            # explicit greedy: default chain (top_k=40) is not bitwise
            # deterministic across runs, which breaks the SHA lossless check
            "samplers": ["greedy"],
            "cache_prompt": True,
        }).encode(),
        headers={"Content-Type": "application/json"},
    )
    return json.load(urllib.request.urlopen(req, timeout=600))

for attempt in (1, 2, 3):
    r = once()
    t = r["timings"]
    content = r.get("content", "")
    if content:
        break
    time.sleep(2)  # transient 1-token stop (observed once on arm A)
sha = hashlib.sha256(content.encode()).hexdigest()[:16]
print(json.dumps({
    "tps": round(t["predicted_per_second"], 3),
    "prompt_tps": round(t.get("prompt_per_second", 0.0), 3),
    "cache_n": t.get("cache_n", -1),
    "predicted_n": t.get("predicted_n", -1),
    "accept": str(t.get("draft_acceptance", "-")),
    "sha16": sha,
    "attempt": attempt,
}))
EOF
}

gpu_vram() {
  nvidia-smi --query-gpu=index,memory.used,memory.total --format=csv,noheader,nounits \
    | paste -sd' ' -
}

# wait for health
for i in $(seq 1 240); do
  s=$(curl -sf -m 2 "$BASE/health" 2>/dev/null | python3 -c "import json,sys; print(json.load(sys.stdin).get('status',''))" 2>/dev/null || true)
  [[ "$s" == "ok" ]] && break
  sleep 1
done
[[ "$s" == "ok" ]] || { echo "ERROR: server not healthy after wait"; exit 2; }

echo "gpu_vram_before: $(gpu_vram)"

echo "warmup..."
run_once > /dev/null || { echo "warmup failed"; exit 1; }

tps_list=()
for i in $(seq 1 "$RUNS"); do
  out=$(run_once) || { echo "run $i failed"; exit 1; }
  echo "run$i: $out"
  tps_list+=("$(echo "$out" | json_field "['tps']")")
  sleep 1
done

median=$(printf '%s\n' "${tps_list[@]}" | sort -n | awk '{a[NR]=$1} END{print (NR%2) ? a[(NR+1)/2] : (a[NR/2]+a[NR/2+1])/2}')
echo "RESULT [$LABEL] median_tps=$median runs=${tps_list[*]} gpu_vram_after: $(gpu_vram)"
