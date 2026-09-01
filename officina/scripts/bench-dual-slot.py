#!/usr/bin/env python3
"""Dual-slot A/B benchmark for the VITRIOL engine + background-lane plan.

Protocol from .opencode/plans/dual-slot-background-lane-2026-08-31.md §6.
Measures, against the LIVE engine (must be booted with --parallel 2):

  A  serial baseline   4 x (2k-token prompt, 256-token decode), one after another
  B  parallel          same 4 jobs fired concurrently (continuous batching)
  C  foreground stall  decode stream running while an 8k prefill is admitted
                       (Sarathi-style chunked-prefill contention check)

Usage: python3 scripts/bench-dual-slot.py [--base http://127.0.0.1:8279]
Output: JSON summary on stdout. Flag provenance: record the server argv
alongside results (AGENTS.md fingerprint rule).
"""

import argparse
import concurrent.futures
import json
import sys
import time
import urllib.request

PROMPT_FILLER = (
    "// review the following module for correctness. "
    "fn example_{i}(x: u64) -> u64 {{ x.wrapping_add({i}) }}\n"
)
DECODE_TOKENS = 256
CONCURRENCY = 4


def post(base, body, timeout=300):
    req = urllib.request.Request(
        f"{base}/completion",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
    )
    t0 = time.time()
    with urllib.request.urlopen(req, timeout=timeout) as r:
        data = json.loads(r.read())
    dt = time.time() - t0
    usage = data.get("timings", {})
    return {
        "wall_s": round(dt, 3),
        "prompt_tokens": usage.get("prompt_n", data.get("tokens_evaluated", 0)),
        "gen_tokens": usage.get("predicted_n", data.get("tokens_predicted", 0)),
        "tps": usage.get("predicted_per_second"),
        "tps_prompt": usage.get("prompt_per_second"),
    }


def make_prompt(tag, fill_tokens=2000):
    # ~4 chars/token for code-ish text; 2000 tokens ≈ 8k chars
    fill = PROMPT_FILLER.format(i=tag, ) * 0  # placeholder; built below
    unit = "// module under review: bounded-input job for the background lane.\n"
    reps = max(1, (fill_tokens * 4) // len(unit))
    return f"[job {tag}] " + unit * reps


def phase(base, label, jobs, parallel):
    prompts = [{"prompt": make_prompt(i), "n_predict": DECODE_TOKENS, "temperature": 0.0}
               for i in range(jobs)]
    t0 = time.time()
    if parallel:
        with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as ex:
            results = list(ex.map(lambda b: post(base, b), prompts))
    else:
        results = [post(base, b) for b in prompts]
    wall = time.time() - t0
    gen = sum(r["gen_tokens"] for r in results)
    summary = {
        "phase": label,
        "jobs": jobs,
        "parallel": parallel,
        "wall_s": round(wall, 2),
        "gen_tokens_total": gen,
        "aggregate_tps": round(gen / wall, 1) if wall else 0,
        "per_job_tps": [r["tps"] for r in results],
    }
    print(json.dumps(summary), file=sys.stderr)
    return summary


def phase_c_stall(base):
    """Decode a long stream; mid-stream admit a big prefill; sample the dip."""
    stream_body = json.dumps({"prompt": make_prompt(90), "n_predict": 512,
                              "temperature": 0.0, "stream": True}).encode()
    stall = {"started": False}
    samples = []

    import threading
    def sampler():
        while not stop.is_set():
            try:
                with urllib.request.urlopen(f"{base}/metrics", timeout=5) as r:
                    m = r.read().decode()
                pt = dt = None
                for line in m.splitlines():
                    if line.startswith("llamacpp:prompt_tokens_total "):
                        pt = int(float(line.split()[1]))
                    elif line.startswith("llamacpp:n_decode_total "):
                        # decode STEPS, not tokens — with MTP speculative
                        # decoding on, tokens_predicted_total lags the truth
                        dt = int(float(line.split()[1]))
                if pt is not None and dt is not None:
                    samples.append((time.time(), pt, dt))
            except Exception:
                pass
            time.sleep(0.25)

    stop = threading.Event()
    th = threading.Thread(target=sampler)
    th.start()
    t0 = time.time()
    with urllib.request.urlopen(urllib.request.Request(
            f"{base}/completion", data=stream_body,
            headers={"Content-Type": "application/json"}), timeout=300) as r:
        for _ in r:
            if not stall["started"] and time.time() - t0 > 2.0:
                stall["started"] = True
                # admit the 8k-prefill interruptor
                post(base, {"prompt": make_prompt(99, fill_tokens=8000),
                            "n_predict": 1, "temperature": 0.0})
    wall = time.time() - t0
    stop.set()
    th.join()
    # foreground decode rate before vs during the interruptor prefill
    def rate(a, b):
        dt = b[0] - a[0]
        return (b[2] - a[2]) / dt if dt > 0 else 0.0
    mid = len(samples) // 2
    before = rate(samples[0], samples[mid]) if len(samples) > 2 else 0
    during = rate(samples[mid], samples[-1]) if len(samples) > 2 else 0
    return {"phase": "C-foreground-stall", "decode_wall_s": round(wall, 2),
            "decode_tps_before_interrupt": round(before, 1),
            "decode_tps_during_interrupt": round(during, 1)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default="http://127.0.0.1:8279")
    args = ap.parse_args()
    base = args.base.rstrip("/")
    with urllib.request.urlopen(f"{base}/health", timeout=10) as r:
        if r.status != 200:
            sys.exit("engine not healthy")
    out = {
        "fingerprint": "bench-dual-slot.py — record server argv alongside (AGENTS.md)",
        "A_serial": phase(base, "A-serial", CONCURRENCY, parallel=False),
        "B_parallel": phase(base, "B-parallel", CONCURRENCY, parallel=True),
        "C_stall": phase_c_stall(base),
    }
    s, p = out["A_serial"], out["B_parallel"]
    out["parallel_speedup"] = round(s["wall_s"] / p["wall_s"], 2) if p["wall_s"] else 0
    print(json.dumps(out, indent=2))


if __name__ == "__main__":
    main()
