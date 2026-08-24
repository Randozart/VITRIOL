#!/usr/bin/env python3
"""Probe: does the heap corruption track the model file?

Launches worktree llama-server with a given gguf + flag set, runs one
completion after /health goes ok, reports corruption marker presence.
"""
import json, os, signal, subprocess, sys, time, urllib.request

SERVER = "llama.cpp/build/bin/llama-server"
PORT = 8299

def run(tag, model, extra, env_extra=None):
    env = dict(os.environ)
    env["VITRIOL_MODE"] = "stream"
    env.pop("VITRIOL_LULL_PROFILE", None)
    for kv in (env_extra or []):
        k, _, v = kv.partition("=")
        if v == "":
            env.pop(k, None)
        else:
            env[k] = v
    cmd = [SERVER, "-m", model, "-ngl", "99", "-c", "8192",
           "--host", "127.0.0.1", "--port", str(PORT)] + extra
    log = f"/tmp/opencode/probe-{tag}.log"
    with open(log, "w") as lf:
        p = subprocess.Popen(cmd, stdout=lf, stderr=lf, env=env)
        ready = False
        try:
            for _ in range(420):
                if p.poll() is not None:
                    break
                try:
                    r = json.loads(urllib.request.urlopen(
                        f"http://127.0.0.1:{PORT}/health", timeout=2).read())
                    if r.get("status") == "ok":
                        ready = True
                        break
                except Exception:
                    pass
                time.sleep(1)
            print(f"[{tag}] ready={ready}", flush=True)
            if ready:
                req = urllib.request.Request(
                    f"http://127.0.0.1:{PORT}/completion",
                    data=json.dumps({"prompt": "hi", "n_predict": 8}).encode(),
                    headers={"Content-Type": "application/json"})
                resp = urllib.request.urlopen(req, timeout=300)
                body = resp.read()
                print(f"[{tag}] completion ok ({len(body)} bytes)", flush=True)
                time.sleep(3)
                # second request exercises cache-reuse teardown paths
                urllib.request.urlopen(req, timeout=300).read()
                print(f"[{tag}] second ok", flush=True)
        except Exception as e:
            print(f"[{tag}] EXC: {type(e).__name__}: {e}", flush=True)
        finally:
            try:
                os.killpg(p.pid, signal.SIGKILL)
            except Exception:
                pass
    txt = open(log, errors="replace").read()
    corrupt = "free(): invalid pointer" in txt or "corrupt" in txt.lower()
    sibling = "partial load" in txt
    print(f"[{tag}] corrupt={corrupt} partial_load={sibling} log={log}", flush=True)
    return corrupt

if __name__ == "__main__":
    tag = sys.argv[1]
    model = sys.argv[2]
    extra = sys.argv[3:]
    sys.exit(1 if run(tag, model, extra) else 0)
