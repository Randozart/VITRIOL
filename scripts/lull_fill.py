#!/usr/bin/env python3
"""VITRIOL LULL — chunked deep-context fill + decode measurement.

Fills the KV cache to a target depth via many small cache_prompt requests
(dodging the single-huge-prefill heap-corruption trigger), optionally with
VITRIOL_KV_SCORE=probe + VITRIOL_KV_MODE=sparse, then times 64-token decode.

    python3 scripts/lull_fill.py --ctx 8192 --depth 7680
    python3 scripts/lull_fill.py --ctx 32768 --depth 31744 --sparse

Outputs RESULT json line; server log at /tmp/opencode/lullfill-<tag>.log.
"""
import argparse
import json
import os
import signal
import subprocess
import sys
import time
import urllib.request

HOME = os.path.expanduser("~")
REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SERVER = os.path.join(REPO, "llama.cpp", "build", "bin", "llama-server")
MODEL = os.path.join(HOME, "Downloads", "Qwen3.8-27B-UD-IQ2_S.gguf")
PORT = 8299


def completion(payload, timeout_s=1800):
    req = urllib.request.Request(
        f"http://127.0.0.1:{PORT}/completion",
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=timeout_s) as r:
        return json.loads(r.read())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ctx", type=int, required=True)
    ap.add_argument("--depth", type=int, required=True, help="target filled tokens")
    ap.add_argument("--chunk", type=int, default=256)
    ap.add_argument("--gen", type=int, default=64)
    ap.add_argument("--tag", type=str, default=None)
    ap.add_argument("--sparse", action="store_true", help="enable VITRIOL_KV_MODE=sparse")
    ap.add_argument("--model", type=str, default=MODEL, help="gguf path")
    ap.add_argument("--ts", type=str, default="27,9", help="tensor split")
    ap.add_argument("--ub", type=int, default=128)
    ap.add_argument("--fa", type=str, default=None, help="force flash attention on/off")
    args = ap.parse_args()

    tag = args.tag or f"fill{args.depth}{'s' if args.sparse else ''}"
    logfile = f"/tmp/opencode/lullfill-{tag}.log"

    env = dict(os.environ)
    env["VITRIOL_MODE"] = "stream"
    env["VITRIOL_KV_SCORE"] = "probe"
    if args.sparse:
        env["VITRIOL_KV_MODE"] = "sparse"

    cmd = [
        SERVER,
        "-m", args.model,
        "-ngl", "99",
        "--main-gpu", "0",
        "-ts", args.ts,
        "-c", str(args.ctx),
        "-ub", str(args.ub),
        *([] if args.fa is None else ["-fa", args.fa]),
        "--cache-type-k", "q4_0",
        "--cache-type-v", "q4_0",
        # NOTE: do NOT pass --ctx-checkpoints 0 (heap corruption: checkpoint
        # code mishandles disabled=0) nor --cache-ram 0 (server never becomes
        # ready). Bound them instead.
        "--ctx-checkpoints", "4",
        "--checkpoint-every-n-tokens", "8192",
        "--cache-reuse", "256",
        "--host", "127.0.0.1",
        "--port", str(PORT),
    ]
    print(f"[lull_fill] launching ctx={args.ctx} depth={args.depth} sparse={args.sparse}", flush=True)
    with open(logfile, "w") as lf:
        proc = subprocess.Popen(cmd, stdout=lf, stderr=lf, env=env,
                                start_new_session=True)
        result = {}
        try:
            ready = False
            for _ in range(420):
                try:
                    if json.loads(urllib.request.urlopen(
                            f"http://127.0.0.1:{PORT}/health", timeout=2).read()).get("status") == "ok":
                        ready = True
                        break
                except Exception:
                    pass
                time.sleep(1)
            if not ready:
                print("[lull_fill] server never became ready")
                sys.exit(1)

            # adaptive fill: measure the true words→tokens ratio from round 1,
            # then size each round so we never exceed min(depth, ctx-256)
            n_words_total = int(args.depth * 5.15 / 6)
            chunk_words = max(8, int(args.chunk * 5.15 / 6))
            words = []
            t0 = time.time()
            rounds = 0
            r = {}
            tok_per_word = None
            while True:
                remaining_tok = args.depth - r.get("tokens_evaluated", 0)
                if remaining_tok <= 0:
                    break
                tpw = tok_per_word or 3.5
                add_words = max(8, min(chunk_words, int(remaining_tok / tpw)))
                base = len(words)
                words.extend("w%d" % (base + i) for i in range(add_words))
                rounds += 1
                r = completion({
                    "prompt": " ".join(words),
                    "n_predict": 1,
                    "cache_prompt": True,
                })
                got = r.get("tokens_evaluated", 0)
                if len(words):
                    tok_per_word = got / len(words)
                if rounds % 20 == 0:
                    print(f"[lull_fill] round {rounds}: {got} tokens cached", flush=True)
            fill_s = time.time() - t0
            depth_reached = r.get("tokens_evaluated", 0)

            # measured decode at depth: fresh short prompts, full-cache attention
            ts = []
            for i in range(3):
                t0 = time.time()
                completion({"prompt": f"Continue the analysis {i}:", "n_predict": args.gen,
                            "cache_prompt": False})
                dt = time.time() - t0
                ts.append(round(args.gen / dt, 3))

            result = {
                "tag": tag, "ctx_alloc": args.ctx, "depth_reached": depth_reached,
                "fill_s": round(fill_s, 1), "t_s_rounds": ts,
                "t_s_mean": round(sum(ts) / len(ts), 3), "log": logfile,
            }
        except Exception as e:
            result = {"tag": tag, "error": f"{type(e).__name__}: {e}", "log": logfile}
        finally:
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
            except Exception:
                pass

    txt = open(logfile, errors="replace").read()
    result["evict_lines"] = txt.count("VITRIOL_KV_EVICT")
    result["probe_marker"] = "probe active" in txt
    result["corrupt"] = ("free(): invalid pointer" in txt) or ("CUDA error" in txt)
    print("[RESULT] " + json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
