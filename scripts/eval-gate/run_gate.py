"""Round-2 compile-gate driver.
usage: run_gate.py --port 8279 --arm base|adapter --out DIR [--rounds 3] [--max-tokens 3500]
"""
import argparse, json, pathlib, re, subprocess, sys, time, urllib.request

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from problems import PROBLEMS, COMMON

CODE_TIMEOUT = 90
RUN_TIMEOUT = 15
CLIPPY_TIMEOUT = 120


def chat(port, messages, max_tokens):
    body = json.dumps({"messages": messages, "temperature": 0.0,
                       "max_tokens": max_tokens, "cache_prompt": True}).encode()
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/v1/chat/completions",
        data=body, headers={"Content-Type": "application/json"})
    t0 = time.time()
    with urllib.request.urlopen(req, timeout=1500) as r:
        resp = json.load(r)
    wall = time.time() - t0
    text = resp["choices"][0]["message"]["content"]
    usage = resp.get("usage", {})
    timings = resp.get("timings", {})
    return text, resp["choices"][0].get("finish_reason"), usage, timings, wall


def extract_code(text):
    fences = re.findall(r"```(?:rust)?\s*\n(.*?)```", text, re.S)
    if fences:
        return "\n".join(fences), len(fences)
    if "```" in text:
        m = re.search(r"```(?:rust)?\s*\n(.*)", text, re.S)
        return m.group(1), -1
    return text, 0


def run_gate(code, workdir):
    src = workdir / "main.rs"
    src.write_text(code)
    res = {"compile_ok": False, "run_ok": False, "clippy_ok": None,
           "stage": None, "log": ""}
    c = subprocess.run(["rustc", "--edition", "2021", "-O", "-o",
                        str(workdir / "bin"), str(src)],
                       capture_output=True, text=True, timeout=CODE_TIMEOUT)
    if c.returncode != 0:
        res["stage"], res["log"] = "rustc", c.stderr[-6000:]
        return res
    res["compile_ok"] = True
    try:
        run = subprocess.run([str(workdir / "bin")], capture_output=True,
                             text=True, timeout=RUN_TIMEOUT)
    except subprocess.TimeoutExpired:
        res["stage"], res["log"] = "run", "TIMEOUT 15s"
        return res
    if run.returncode != 0:
        res["stage"], res["log"] = "run", (run.stdout + run.stderr)[-6000:]
        return res
    res["run_ok"] = True
    cl = subprocess.run(["clippy-driver", "--edition", "2021", "-D", "warnings",
                         "-o", str(workdir / "discard"), str(src)],
                        capture_output=True, text=True, timeout=CLIPPY_TIMEOUT)
    res["clippy_ok"] = cl.returncode == 0
    if not res["clippy_ok"]:
        res["stage"], res["log"] = "clippy", cl.stderr[-6000:]
    return res


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, required=True)
    ap.add_argument("--arm", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--rounds", type=int, default=3)
    ap.add_argument("--max-tokens", type=int, default=3500)
    args = ap.parse_args()

    outdir = pathlib.Path(args.out) / args.arm
    outdir.mkdir(parents=True, exist_ok=True)
    summary = []

    for name, task in PROBLEMS:
        pdir = outdir / name
        pdir.mkdir(parents=True, exist_ok=True)
        messages = [{"role": "user", "content": task + "\n\n" + COMMON}]
        rec = {"problem": name, "arm": args.arm, "gens": 0, "tokens": 0,
               "wall_s": 0.0, "ts_list": [], "first_shot": False,
               "green": False, "history": [], "finish_reasons": []}
        print(f"=== {args.arm}/{name}", flush=True)
        for attempt in range(args.rounds + 1):
            text, finish, usage, timings, wall = chat(args.port, messages, args.max_tokens)
            code, nfences = extract_code(text)
            (pdir / f"attempt{attempt}.md").write_text(text)
            (pdir / f"attempt{attempt}.rs").write_text(code)
            rec["gens"] += 1
            rec["tokens"] += usage.get("completion_tokens", 0)
            rec["wall_s"] += wall
            if timings.get("predicted_per_second"):
                rec["ts_list"].append(round(timings["predicted_per_second"], 2))
            rec["finish_reasons"].append(finish)
            gate = run_gate(code, pdir)
            entry = {"attempt": attempt, **{k: v for k, v in gate.items() if k != "log"},
                     "log_tail": gate["log"][-500:], "fences": nfences}
            rec["history"].append(entry)
            print(json.dumps(entry), flush=True)
            if gate["compile_ok"] and gate["run_ok"] and gate["clippy_ok"]:
                rec["green"] = True
                if attempt == 0:
                    rec["first_shot"] = True
                break
            if attempt < args.rounds:
                messages.append({"role": "assistant", "content": text})
                messages.append({"role": "user", "content":
                    f"The code failed the {gate['stage']} stage. "
                    f"Output:\n```\n{gate['log']}\n```\n"
                    "Return the COMPLETE corrected single-file program, one ```rust block."})
        rec["rounds_to_green"] = (len(rec["history"]) - 1) if rec["green"] else None
        rec["mean_ts"] = round(sum(rec["ts_list"]) / len(rec["ts_list"]), 2) if rec["ts_list"] else None
        final_code = (pdir / f"attempt{len(rec['history'])-1}.rs").read_text()
        rec["loc"] = len(final_code.splitlines())
        rec["unsafe_count"] = len(re.findall(r"\bunsafe\b", final_code))
        summary.append(rec)
        (outdir / f"{name}.result.json").write_text(json.dumps(rec, indent=1))
        print(f"--- {name}: green={rec['green']} rounds={rec['rounds_to_green']} "
              f"tokens={rec['tokens']} wall={rec['wall_s']:.0f}s", flush=True)

    (outdir / "summary.json").write_text(json.dumps(summary, indent=1))
    greens = [r for r in summary if r["green"]]
    print(f"DONE {args.arm}: green {len(greens)}/{len(summary)} "
          f"first_shot {sum(1 for r in greens if r['first_shot'])}/{len(summary)} "
          f"total_tokens {sum(r['tokens'] for r in summary)} "
          f"total_wall {sum(r['wall_s'] for r in summary):.0f}s", flush=True)


if __name__ == "__main__":
    main()
