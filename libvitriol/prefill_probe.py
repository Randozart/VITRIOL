#!/usr/bin/env python3
"""Prefill/decode probe for REBIS Phase 0 — measures llama-server ingestion
and generation throughput from the server's own timings (no clock skew).

Usage: python3 libvitriol/prefill_probe.py --url http://127.0.0.1:8279 \
           --sizes 1000 4000 16000 [--decode] [--json-out results.json]
"""

import argparse
import json
import time
import urllib.request

FILLER = (
    "static inline size_t vitriol_page_align(size_t n) { "
    "return (n + PAGE_SIZE - 1) & ~(PAGE_SIZE - 1); } "
    "/* expert tile pinned via page-locked DMA buffer */ "
)


def build_prompt(url: str, approx_tokens: int) -> str:
    """Build a prompt of EXACTLY approx_tokens using the server tokenizer."""
    chars = int(approx_tokens * 4 * 0.7)  # first guess; corrected below
    out = []
    total = 0
    while total < chars:
        chunk = FILLER * 8
        out.append(chunk)
        total += len(chunk)
    text = "".join(out)
    # Trim to exact token count via the server.
    body = json.dumps({"content": text}).encode()
    req = urllib.request.Request(
        f"{url}/tokenize", data=body,
        headers={"Content-Type": "application/json"}, method="POST",
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        tokens = json.loads(resp.read())["tokens"]
    tokens = tokens[:approx_tokens]
    body = json.dumps({"tokens": tokens}).encode()
    req = urllib.request.Request(
        f"{url}/detokenize", data=body,
        headers={"Content-Type": "application/json"}, method="POST",
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read())["content"]


def probe_once(url: str, prompt: str, n_predict: int, timeout: int = 900) -> dict:
    body = json.dumps({
        "prompt": prompt,
        "n_predict": n_predict,
        "temperature": 0.0,
        "cache_prompt": False,
    }).encode()
    req = urllib.request.Request(
        f"{url}/completion", data=body,
        headers={"Content-Type": "application/json"}, method="POST",
    )
    t0 = time.time()
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        data = json.loads(resp.read())
    wall = time.time() - t0
    t = data.get("timings", {})
    return {
        "prompt_n": t.get("prompt_n"),
        "prompt_ms": t.get("prompt_ms"),
        "prompt_t_s": (t.get("prompt_n") / (t.get("prompt_ms") / 1e3))
        if t.get("prompt_ms") else None,
        "predicted_n": t.get("predicted_n"),
        "predicted_ms": t.get("predicted_ms"),
        "predicted_t_s": (t.get("predicted_n") / (t.get("predicted_ms") / 1e3))
        if t.get("predicted_ms") else None,
        "wall_s": round(wall, 2),
    }


def main() -> None:
    p = argparse.ArgumentParser(description="prefill/decode throughput probe")
    p.add_argument("--url", default="http://127.0.0.1:8279")
    p.add_argument("--sizes", type=int, nargs="+", default=[1000, 4000, 16000],
                   help="approx prompt sizes in tokens")
    p.add_argument("--rounds", type=int, default=2)
    p.add_argument("--decode", action="store_true", help="also measure decode t/s (128 tok)")
    p.add_argument("--json-out", default=None)
    args = p.parse_args()

    results = {"url": args.url, "prefill": [], "decode": None}
    for size in args.sizes:
        prompt = build_prompt(args.url, size)
        best: dict | None = None
        for _ in range(args.rounds):
            r = probe_once(args.url, prompt, n_predict=1)
            if best is None or (r["prompt_t_s"] or 0) > (best["prompt_t_s"] or 0):
                best = r
        assert best is not None
        print(f"prefill ~{size:>6} tok: n={best['prompt_n']} "
              f"{best['prompt_t_s']:.1f} tok/s (wall {best['wall_s']}s)")
        results["prefill"].append({"target_tokens": size, **best})

    if args.decode:
        prompt = build_prompt(args.url, 512)
        best = None
        for _ in range(2):
            r = probe_once(args.url, prompt, n_predict=128)
            if best is None or (r["predicted_t_s"] or 0) > (best["predicted_t_s"] or 0):
                best = r
        assert best is not None
        print(f"decode 128 tok: {best['predicted_t_s']:.2f} tok/s")
        results["decode"] = best

    if args.json_out:
        with open(args.json_out, "w") as f:
            json.dump(results, f, indent=2)
        print(f"wrote {args.json_out}")


if __name__ == "__main__":
    main()
