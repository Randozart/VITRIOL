# VITRIOL Flash-Next Test Plan — 2026-09-06

## Upstream Sync Strategy

20 commits behind upstream/master. Critical: `64a155d24 sync: ggml` strips
VITRIOL files. Strategy:

1. Fetch upstream/master
2. Create a merge branch
3. Merge with `-X ours` for VITRIOL files (keep our changes)
4. Resolve conflicts in ggml-sycl.cpp manually
5. Verify build compiles
6. Run tests on merged code

## Test Suite

### Test 1: LRU Stats Verification (5 min)

**Goal**: Verify verbose logging shows LRU hit/miss/eviction stats.

**Method**:
1. Start server with VITRIOL_VERBOSE=1
2. Send 5-10 completion requests
3. Check logs for "VITRIOL-SYCL LRU: hits=... misses=... evictions=..."

**Pass criteria**: Hit rate > 50% after warm-up, eviction count reasonable.

### Test 2: Correctness Test (15 min)

**Goal**: Verify VITRIOL doesn't degrade model output quality.

**Method**:
1. Start VITRIOL streaming server
2. Start CPU-only server (different port, -ngl 0 without VITRIOL)
3. Send same prompt to both with temperature=0
4. Compare output text (should be identical or near-identical)

**Pass criteria**: Output text matches or differs by < 5% of tokens.

### Test 3: Concurrent Request Test (10 min)

**Goal**: Verify LRU works under concurrent load.

**Method**:
1. Start server with --parallel 4
2. Send 4 concurrent requests
3. Measure per-stream and aggregate throughput

**Pass criteria**: No crashes, aggregate throughput > single-stream.

### Test 4: Long Context Test (10 min)

**Goal**: Verify performance with realistic prompt lengths.

**Method**:
1. Start server with c=4096
2. Send prompts of 10, 50, 100 tokens
3. Measure pp and tg at each length

**Pass criteria**: pp degrades gracefully (not exponentially) with prompt length.

### Test 5: Memory Pressure Test (10 min)

**Goal**: Verify VITRIOL doesn't cause OOM.

**Method**:
1. Monitor free -h during model load
2. Send multiple requests
3. Check for OOM kills in dmesg

**Pass criteria**: No OOM kills, memory usage stable.
