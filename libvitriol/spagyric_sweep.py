#!/usr/bin/env python3
"""Spagyric decode-knob sweep harness.

Mode A: single-request decode t/s per config (ubatch, threads), warmup + 3 rounds.
Mode B: concurrent-request throughput per config (parallel slots), N parallel
64-token completions, aggregate = N*64/wall.

Server lifecycle managed here (launch -> wait healthy -> measure -> kill).
Writes CSV to --output. Correctness gate: output must mention merge sort (code or prose).
"""
import argparse
import csv
import json
import os
import subprocess
import threading
import time
import urllib.request
from dataclasses import dataclass

SERVER = "/home/randozart/Desktop/Projects/VITRIOL/llama.cpp/build/bin/llama-server"
BASE = "http://127.0.0.1:8080"
PROMPT = "Write a Python function for merge sort."
N_TOKENS = 64
ROUNDS = 3


@dataclass
class SweepSpec:
    """One server launch: model path, offload, context, and the knobs under test."""

    model: str
    ngl: int
    ctx: int
    threads: int
    ubatch: object  # int or None (None = server default)
    parallel: object  # int or None (None = server default)
    extra: tuple = ()  # extra server CLI args (e.g. --no-kv-offload --cache-type-k q4_0)


# Intent: print a timestamped progress line, flushed so nohup log tails stream.
def log(msg):
    """Print a timestamped progress line to stdout (flushed for nohup tails)."""
    print("[%s] %s" % (time.strftime("%H:%M:%S"), msg), flush=True)


# Intent: launch llama-server detached with the spec's knobs; return the Popen handle.
def start_server(spec):
    """Launch llama-server detached with the spec's knobs; return the Popen handle."""
    cmd = ["setsid", SERVER, "-m", spec.model, "-ngl", str(spec.ngl),
           "-c", str(spec.ctx), "-t", str(spec.threads), "--port", "8080"]
    if spec.ubatch is not None:
        cmd += ["--ubatch-size", str(spec.ubatch)]
    if spec.parallel is not None:
        cmd += ["--parallel", str(spec.parallel)]
    if spec.extra:
        cmd += list(spec.extra)
    devnull = open(os.devnull, "w")
    errlog = open("/tmp/opencode/server_stderr.log", "w")
    return subprocess.Popen(cmd, stdout=devnull, stderr=errlog,
                            stdin=devnull, start_new_session=True), devnull, errlog


# Intent: kill every llama-server instance so the next config launches clean.
def stop_server():
    """Kill every llama-server instance so the next config launches clean."""
    subprocess.run(["killall", "-9", "llama-server"], capture_output=True)
    time.sleep(2)


# Intent: poll /health until the server reports ok, or return False on timeout.
def wait_healthy(timeout=420):
    """Poll /health until the server reports ok, or return False on timeout."""
    t0 = time.time()
    while time.time() - t0 < timeout:
        try:
            r = urllib.request.urlopen(BASE + "/health", timeout=3)
            if r.status == 200 and json.loads(r.read()).get("status") == "ok":
                return True
        except Exception:
            pass
        time.sleep(2)
    return False


# Intent: send one 64-token completion; return (decode t/s, eval t/s, text).
def completion():
    """Send one 64-token completion; return (decode t/s, eval t/s, text)."""
    body = json.dumps({"prompt": PROMPT, "n_predict": N_TOKENS,
                       "temperature": 0.0}).encode()
    req = urllib.request.Request(BASE + "/v1/completions", data=body,
                                 headers={"Content-Type": "application/json"})
    resp = json.loads(urllib.request.urlopen(req, timeout=120).read())
    t = resp["timings"]
    return (t["predicted_per_second"], t["prompt_per_second"],
            resp["choices"][0]["text"])


# Intent: mode A — warmup + ROUNDS completions; return mean decode/eval t/s, legibility.
def measure_a():
    """Mode A: warmup + ROUNDS completions; return mean decode/eval t/s and legibility."""
    completion()
    dec, ev, legible = [], [], True
    for _ in range(ROUNDS):
        d, e, txt = completion()
        dec.append(d)
        ev.append(e)
        legible = legible and ("merge" in txt.lower())
    return sum(dec) / len(dec), sum(ev) / len(ev), legible


# Intent: mode B — fire n concurrent completions; return aggregate t/s, wall, mean decode, legibility.
def measure_b(n):
    """Mode B: fire n concurrent completions; return aggregate t/s, wall, mean decode, legibility."""
    completion()
    results = []
    lock = threading.Lock()

    # Intent: run one completion and stash the result under the lock.
    def work():
        """Run one completion and stash the result under the lock."""
        try:
            r = completion()
            with lock:
                results.append(r)
        except Exception:
            with lock:
                results.append(None)

    t0 = time.time()
    ths = [threading.Thread(target=work) for _ in range(n)]
    for t in ths:
        t.start()
    for t in ths:
        t.join()
    wall = time.time() - t0
    ok = [r for r in results if r]
    tps = (len(ok) * N_TOKENS) / wall if wall > 0 else 0.0
    mean_dec = (sum(r[0] for r in ok) / len(ok)) if ok else 0.0
    legible = all(("merge" in r[2].lower()) for r in ok)
    return tps, wall, mean_dec, legible


# Intent: parse args, run the config grid, write the CSV.
def main():
    """Parse args, run the config grid, write the CSV."""
    ap = argparse.ArgumentParser(description="Spagyric decode-knob sweep")
    ap.add_argument("--model", required=True)
    ap.add_argument("--ngl", type=int, required=True)
    ap.add_argument("--ctx", type=int, required=True)
    ap.add_argument("--output", required=True)
    ap.add_argument("--ubatch-list", type=int, nargs="+", default=[64, 128, 256, 512])
    ap.add_argument("--threads-list", type=int, nargs="+", default=[2, 8])
    ap.add_argument("--parallel-list", type=int, nargs="+", default=[2, 4, 8])
    ap.add_argument("--ubatch-threads", type=int, default=256,
                    help="ubatch value at which threads are swept")
    ap.add_argument("--extra", default="",
                    help="extra llama-server args for every config (e.g. '--no-kv-offload --cache-type-k q4_0')")
    args = ap.parse_args()
    extra = tuple(args.extra.split())
    ctxs = [args.ctx]  # default: single ctx; override via --ctx-list below

    fields = ["model", "knob", "value", "decode_tps", "eval_tps",
              "concurrent_tps", "wall_s", "correct"]
    rows = []

    # Intent: launch one spec, measure in the given mode, append the CSV row.
    def run_cfg(tag, value, spec, mode):
        """Launch one spec, measure in the given mode, append the CSV row."""
        log("start %s=%s (t=%s ubatch=%s parallel=%s)" % (tag, value, spec.threads,
                                                          spec.ubatch, spec.parallel))
        proc, devnull, errlog = start_server(spec)
        try:
            if not wait_healthy():
                log("  FAILED to become healthy")
                rows.append([args.model, tag, value, "", "", "", "", "NO_HEALTH"])
                return
            time.sleep(1)
            if mode == "A":
                d, e, leg = measure_a()
                log("  A: decode=%.2f eval=%.2f correct=%s" % (d, e, leg))
                rows.append([args.model, tag, value, round(d, 2), round(e, 2),
                             "", "", "PASS" if leg else "FAIL"])
            else:
                tps, wall, md, leg = measure_b(spec.parallel)
                log("  B: parallel=%d aggregate=%.2f t/s wall=%.1fs correct=%s"
                    % (spec.parallel, tps, wall, leg))
                rows.append([args.model, tag, value, "", "", round(tps, 2),
                             round(wall, 1), "PASS" if leg else "FAIL"])
        finally:
            devnull.close()
            errlog.close()
            stop_server()

    for ub in args.ubatch_list:
        run_cfg("ubatch", ub, SweepSpec(args.model, args.ngl, args.ctx, 4, ub, 1, extra), "A")
    for t in args.threads_list:
        run_cfg("threads", t, SweepSpec(args.model, args.ngl, args.ctx, t,
                                        args.ubatch_threads, 1, extra), "A")
    for p in args.parallel_list:
        run_cfg("parallel", p, SweepSpec(args.model, args.ngl, args.ctx, 4, None, p, extra), "B")

    with open(args.output, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(fields)
        w.writerows(rows)
    log("done -> %s" % args.output)


if __name__ == "__main__":
    main()
