#!/usr/bin/env python3
"""VITRIOL LULL Phase 0 benchmark driver.

Starts the LULL-worktree llama-server with the qwen38 dual-GPU profile,
prefills a prompt, measures 64-token decode t/s, and captures
VITRIOL_LULL instrumentation output for scripts/lull_report.py.

    python3 scripts/lull_bench.py --ctx 4096 --tag c4k
    python3 scripts/lull_bench.py --ctx 131072 --tag c131k --prefill 2048

Outputs:
    /tmp/opencode/lull-<tag>.log   raw server stderr (VITRIOL_LULL lines)
    stdout summary line            JSON-ish dict of results
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


def wait_ready(timeout_s=600):
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{PORT}/health", timeout=2) as r:
                if r.status == 200:
                    body = json.loads(r.read())
                    # /health returns {"status":"ok"} once model fully loads
                    if body.get("status") == "ok":
                        return True
        except Exception:
            pass
        time.sleep(1.0)
    return False


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
    ap.add_argument("--tag", type=str, required=True)
    ap.add_argument("--prefill", type=int, default=512, help="tokens to prefill before timing")
    ap.add_argument("--gen", type=int, default=64)
    ap.add_argument("--env", action="append", default=[], help="extra KEY=VAL server env")
    ap.add_argument("--extra", action="append", default=[], help="extra server CLI flags")
    ap.add_argument("--no-ts", action="store_true", help="omit tensor split (single GPU)")
    ap.add_argument("--model", type=str, default=None,
                    help="gguf path (default: mtp-Q4_0; Q3_K_M corrupts heap, avoid)")
    args = ap.parse_args()
    fp = ("VITRIOL-FINGERPRINT model=%s ts=%s c=%s kv=%s/%s fa=%s ub=%s mode=%s substrate=%s" %
          (os.path.basename(args.model if hasattr(args,'model') else MODEL),
           getattr(args,'ts','27,9'), args.ctx,
           getattr(args,'kv','q4_0'), getattr(args,'kv','q4_0'),
           getattr(args,'fa',None) or 'auto', getattr(args,'ub',128),
           os.environ.get('VITRIOL_MODE','?'),
           'off' if getattr(args,'no_substrate',False) else 'on'))
    fp += " pool_reset=" + os.environ.get('VITRIOL_POOL_RESET','0')

    logfile = f"/tmp/opencode/lull-{args.tag}.log"
    model = args.model or os.path.join(
        HOME, "Downloads", "mtp-Qwen3.8-27B-Q4_0.gguf")
    env = dict(os.environ)
    env["VITRIOL_LULL_PROFILE"] = "1"
    # mirror scripts/vitriol's exec-env block (defaults from DEFAULT_* there):
    # the CUDA integration behaves differently with unset vars vs the
    # battle-tested launcher environment.
    env.setdefault("VITRIOL_MODE", "stream")
    env["VITRIOL_LRU_MB"] = "0"
    env["VITRIOL_MEMORY_MODE"] = "off"
    env["VITRIOL_KV_MODE"] = "standard"
    env["VITRIOL_FROZEN_PROMPT"] = "off"
    env["VITRIOL_SEMANTIC_MODE"] = "off"
    env["VITRIOL_KV_QUANT"] = "q4_0"
    env["VITRIOL_KV_QUANT_V"] = "f16"
    env["VITRIOL_LOOKUP"] = "0"
    env["VITRIOL_ENGINE_MODE"] = "vitriol-dma"
    env["VITRIOL_EXPERT_COUNT"] = "0"
    env["VITRIOL_OUTPUT_CACHE"] = "off"
    env["VITRIOL_PREDICTIVE_PREFETCH"] = "off"
    env["VITRIOL_PIN_FIRST_N_LAYERS"] = "0"
    env["VITRIOL_PRUNE_EXPERTS"] = "0"
    env["VITRIOL_EARLY_EXIT"] = "0"
    env["VITRIOL_EARLY_EXIT_THRESHOLD"] = "0.001"
    env["VITRIOL_EARLY_EXIT_MIN_LAYERS"] = "10"
    env["VITRIOL_CHIMERA_MODE"] = "auto"
    env["VITRIOL_MODEL_PATH"] = MODEL
    for kv in args.env:
        k, _, v = kv.partition("=")
        if v == "":
            env.pop(k, None)
        else:
            env[k] = v

    cmd = [
        SERVER,
        "-m", MODEL,
        "-ngl", "99",
        "--main-gpu", "0",
        "-c", str(args.ctx),
        "-ub", "128",
        "--cache-type-k", "q4_0",
        "--cache-type-v", "q4_0",
        # NOTE: no --spec-type mtp. MTP showed zero benefit on this hardware
        # (AGENTS.md sweep), and Q3_K_M's embedded-head path corrupts the
        # heap; UD-IQ2_S carries no embedded head. Phase 0 measures raw
        # attention/KV pipeline timing — speculative decoding stays off.
        # keep checkpointing/prompt-cache machinery off for clean timing
        # NOTE: do NOT pass --ctx-checkpoints 0 (heap corruption: checkpoint
        # code mishandles disabled=0) nor --cache-ram 0 (server never becomes
        # ready). Bound them instead.
        "--ctx-checkpoints", "4",
        "--checkpoint-every-n-tokens", "8192",
        "--cache-reuse", "256",
        "--host", "127.0.0.1",
        "--port", str(PORT),
    ]
    if not args.no_ts:
        cmd += ["-ts", "27,9"]
    cmd += args.extra
    print(f"[lull_bench] launching {' '.join(cmd)}", flush=True)
    with open(logfile, "w") as lf:
        lf.write(fp + "\n")
        lf.flush()
        proc = subprocess.Popen(cmd, stdout=lf, stderr=lf, env=env,
                                start_new_session=True)
        try:
            if not wait_ready():
                print("[lull_bench] server failed to become ready; dumping log tail")
                subprocess.run(["tail", "-40", logfile])
                sys.exit(1)

            # warmup (compiles graphs, allocates KV)
            completion({"prompt": "warmup", "n_predict": 8, "cache_prompt": True})

            # prefill to requested depth: one big prompt, cached
            # ~5.2 chars/token for these words; server clamps if we overshoot
            n_fill = max(0, args.prefill - 16)
            unit = "alpha beta gamma delta epsilon zeta "
            target_chars = int(n_fill * 5.2)
            prompt = (unit * (target_chars // len(unit) + 1))[:target_chars]
            t0 = time.time()
            r_pre = completion({"prompt": prompt, "n_predict": 1,
                                "cache_prompt": True})
            prefill_s = time.time() - t0
            n_ctx_used = r_pre.get("tokens_evaluated", 0) + 1

            # measured decode round(s)
            rounds = []
            for _ in range(3):
                t0 = time.time()
                r_gen = completion({"prompt": "Continue the story:", "n_predict": args.gen,
                                    "cache_prompt": False})
                dt = time.time() - t0
                rounds.append(args.gen / dt if dt > 0 else 0.0)
            gen_s = sum(rounds) / len(rounds)

            result = {
                "argv": cmd,
                "tag": args.tag, "ctx_alloc": args.ctx,
                "ctx_used_prefill": n_ctx_used,
                "prefill_s": round(prefill_s, 3),
                "t_s_rounds": [round(x, 3) for x in rounds],
                "t_s_mean": round(gen_s, 3),
                "log": logfile,
            }
            print("[RESULT] " + json.dumps(result), flush=True)
        finally:
            os.killpg(proc.pid, signal.SIGTERM)
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid, signal.SIGKILL)


if __name__ == "__main__":
    main()
