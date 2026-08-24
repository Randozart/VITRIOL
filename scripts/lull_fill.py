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
        "-m", MODEL,
        "-ngl", "99",
        "-ts", "27,9",
        "--main-gpu", "0",
        "-c", str(args.ctx),
        "-ub", "128",
        "--cache-type-k", "q4_0",
        "--cache-type-v", "q4_0",
        "--ctx-checkpoints", "0",
        "--cache-reuse", "0",
        "--cache-ram", "0",
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

            unit = "alpha beta gamma delta epsilon zeta "
            # ~5.15 chars/token for these common words; 6 words per unit
            n_words_total = int(args.depth * 5.15 / 6)
            chunk_words = max(8, int(args.chunk * 5.15 / 6))
            words = []
            t0 = time.time()
            rounds = 0
            r = {}
            while len(words) < n_words_total:
                words.extend("w%d" % i for i in range(len(words), len(words) + chunk_words))
                rounds += 1
                r = completion({
                    "prompt": " ".join(words),
                    "n_predict": 1,
                    "cache_prompt": True,
                })
                if rounds % 20 == 0:
                    print(f"[lull_fill] round {rounds}: ~{r.get('tokens_evaluated',0)} tokens cached", flush=True)
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
