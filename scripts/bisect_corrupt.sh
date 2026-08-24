#!/bin/bash
# git-bisect run script for llama.cpp heap-corruption hunt.
# Builds llama-server incrementally, runs a short c8192 repro via lull_bench,
# exits 0 (good) / 1 (bad) based on free(): invalid pointer or server failure.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cmake --build "$ROOT/llama.cpp/build" --config Release -j8 --target llama-server > /tmp/opencode/bisect-build.log 2>&1
if [ $? -ne 0 ]; then echo "BUILD FAIL"; exit 125; fi
killall -9 llama-server 2>/dev/null; sleep 1
python3 "$ROOT/scripts/lull_bench.py" --ctx 8192 --tag bisect --prefill 7000 \
    --env VITRIOL_LULL_PROFILE= > /tmp/opencode/bisect-run.log 2>&1
if grep -qE 'free\(\): invalid pointer' /tmp/opencode/lull-bisect.log 2>/dev/null; then
    echo "BAD: heap corruption"; exit 1
fi
if grep -q 'RESULT' /tmp/opencode/bisect-run.log; then
    echo "GOOD: clean run"; exit 0
fi
echo "BAD: server failed"; exit 1
