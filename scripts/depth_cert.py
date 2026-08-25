#!/usr/bin/env python3
"""Single-shot depth certification runner.

Loads Qwen3.8-27B with given split/window/KV, sends ONE mega-prefill sized
via /tokenize to --target tokens, then benches 3x64 greedy decode at depth.
Reports RESULT json; server log at /tmp/opencode/dc-<tag>.log.
"""
import argparse, json, os, signal, subprocess, time, urllib.request

SERVER = "/home/randozart/Desktop/Projects/VITRIOL/llama.cpp/build/bin/llama-server"

def post(port, payload, t=7200):
    req = urllib.request.Request(f"http://127.0.0.1:{port}/completion",
        data=json.dumps(payload).encode(), headers={"Content-Type": "application/json"})
    return json.loads(urllib.request.urlopen(req, timeout=t).read())

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True)
    ap.add_argument("--tag", required=True)
    ap.add_argument("--window", type=int, default=131072)
    ap.add_argument("--target", type=int, required=True)
    ap.add_argument("--ts", default="26,10")
    ap.add_argument("--ub", type=int, default=64)
    ap.add_argument("--kv", default="tq3_0")
    ap.add_argument("--no-substrate", action="store_true")
    ap.add_argument("--no-vmm", action="store_true")
    ap.add_argument("--sample-util", action="store_true", help="sample GPU util% during run")
    ap.add_argument("--spec", action="store_true", help="enable MTP speculative decoding n=1")
    ap.add_argument("--spec-draft-model", type=str, default=None, help="separate MTP head gguf")
    ap.add_argument("--no-mmap", action="store_true")
    ap.add_argument("--gen", type=int, default=64)
    ap.add_argument("--fa", type=str, default=None)
    ap.add_argument("--mode", default="off", help="VITRIOL_MODE: off|stream|sync|async")
    a = ap.parse_args()
    fp = ("VITRIOL-FINGERPRINT model=%s ts=%s c=%s kv=%s/%s fa=%s ub=%d mode=%s substrate=%s" %
          (os.path.basename(a.model), a.ts, a.window,
           a.kv, a.kv, a.fa or 'auto', a.ub,
           a.mode,
           'off' if a.no_substrate else 'on'))
    fp += " pool_reset=" + os.environ.get('VITRIOL_POOL_RESET','0')

    env = dict(os.environ)
    env["VITRIOL_MODE"] = a.mode
    env["VITRIOL_KV_QUANT"] = a.kv
    env["VITRIOL_KV_QUANT_V"] = a.kv
    if not a.no_substrate:
        env["VITRIOL_KV_SCORE"] = "probe"
        env["VITRIOL_KV_MODE"] = "sparse"
    else:
        env.pop("VITRIOL_KV_SCORE", None)
        env.pop("VITRIOL_KV_MODE", None)
    if a.no_vmm:
        env["GGML_CUDA_NO_VMM"] = "1"

    cmd = [SERVER, "-m", a.model,
           "-ngl", "99", "-c", str(a.window), "-ub", str(a.ub),
           "-t", "4",
           *([] if a.ts == "none" else ["-ts", a.ts]),
           *([] if not a.fa else ["-fa", a.fa]),
           *([] if not a.spec else (["--spec-type", "mtp", "--spec-draft-n-max", "1"] +
              ([] if not a.spec_draft_model else ["--spec-draft-model", a.spec_draft_model]))),
           *(["--no-mmap"] if a.no_mmap else []),
           "--cache-type-k", a.kv, "--cache-type-v", a.kv,
           "--ctx-checkpoints", "4", "--checkpoint-every-n-tokens", "8192",
           "--host", "127.0.0.1", "--port", "8299"]
    log = f"/tmp/opencode/dc-{a.tag}.log"
    with open(log, "w") as lf:
        lf.write(fp + "\n")
        lf.flush()
        p = subprocess.Popen(cmd, stdout=lf, stderr=lf, env=env, start_new_session=True)
        res = {"tag": a.tag}
        util_samples = []
        stop_util = [False]
        if a.sample_util:
            import threading as _th
            def _sampler():
                while not stop_util[0]:
                    out = subprocess.run(["nvidia-smi",
                        "--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"],
                        capture_output=True, text=True).stdout
                    try:
                        util_samples.append([int(x) for x in out.split()])
                    except Exception:
                        pass
                    time.sleep(2)
            _th.Thread(target=_sampler, daemon=True).start()
        try:
            ok = False
            for _ in range(420):
                try:
                    if json.loads(urllib.request.urlopen(
                            "http://127.0.0.1:8299/health", timeout=2).read()).get("status") == "ok":
                        ok = True; break
                except Exception:
                    pass
                time.sleep(1)
            if not ok:
                res["error"] = "never ready"
                return
            # tokenize-sized single shot
            n = max(100, int(a.target / 5.63))
            while True:
                words = " ".join("f%d" % i for i in range(n))
                tr = urllib.request.Request("http://127.0.0.1:8299/tokenize",
                    data=json.dumps({"content": words}).encode(),
                    headers={"Content-Type": "application/json"})
                nt = len(json.loads(urllib.request.urlopen(tr, timeout=180).read()).get("tokens", []))
                if abs(nt - a.target) <= a.target * 0.02 or n <= 200:
                    break
                n = max(100, int(n * a.target / nt))
            print(f"[{a.tag}] single-shot {nt} tokens ...", flush=True)
            t0 = time.time()
            r = post(8299, {"prompt": words, "n_predict": 1, "temperature": 0})
            tok = r.get("tokens_evaluated", 0)
            res["filled"] = tok
            res["fill_min"] = round((time.time() - t0) / 60, 1)
            print(f"[{a.tag}] filled {tok}", flush=True)
            ts = []
            for i in range(3):
                t0 = time.time()
                post(8299, {"prompt": f"Continue {i}:", "n_predict": a.gen,
                            "cache_prompt": False, "temperature": 0})
                dt = time.time() - t0
                ts.append(round(a.gen / dt, 2))
            res["t_s_rounds"] = ts
            res["t_s_mean"] = round(sum(ts) / len(ts), 2)
            res["argv"] = cmd
            if a.sample_util and util_samples:
                n = len(util_samples)
                res["gpu_util_mean"] = [round(sum(x[i] for x in util_samples)/n) for i in (0,1)]
        except Exception as e:
            res["error"] = f"{type(e).__name__}: {e}"
        finally:
            try: os.killpg(os.getpgid(p.pid), signal.SIGKILL)
            except Exception: pass

    txt = open(log, errors="replace").read()
    res["probe"] = "probe active" in txt
    res["oom"] = "out of memory" in txt
    res["launchfail"] = "failed to launch" in txt
    print("[RESULT] " + json.dumps(res), flush=True)

if __name__ == "__main__":
    main()
