#!/usr/bin/env python3
"""VITRIOL LULL — prompt-cache reuse auditor.

Parses a llama-server log for per-request prefill behavior and reports
how effectively previously-decoded context is being reused.

Metrics per task:
  - prompt tokens PROCESSED (from "prompt eval time = X ms / N tokens")
  - checkpoint restores ("restored context checkpoint")
  - forced full re-processes ("forcing full prompt re-processing")

    python3 scripts/lull_reuse_audit.py /tmp/opencode/vitriol_gen.log
"""
import re
import sys
from collections import defaultdict

EVAL_RE = re.compile(r"prompt eval time =\s*([\d.]+) ms /\s*(\d+) tokens")
RESTORE_RE = re.compile(r"restored context checkpoint \(pos_min = (\d+), pos_max = (\d+), n_tokens = \d+, n_past = (\d+)")
FORCED_RE = re.compile(r"forcing full prompt re-processing")
TASK_CTX = re.compile(r"\| task (\d+) \|")


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "/tmp/opencode/vitriol_gen.log"
    tasks = defaultdict(lambda: {"processed": 0, "restores": 0, "forced": 0, "eval_ms": 0.0})
    cur = None

    for line in open(path, errors="replace"):
        if "update_slots" in line:
            m = TASK_CTX.search(line)
            if m:
                cur = int(m.group(1))
        m = EVAL_RE.search(line)
        if m and cur is not None:
            tasks[cur]["eval_ms"] += float(m.group(1))
            tasks[cur]["processed"] += int(m.group(2))
        if RESTORE_RE.search(line) and cur is not None:
            tasks[cur]["restores"] += 1
        if FORCED_RE.search(line) and cur is not None:
            tasks[cur]["forced"] += 1

    if not tasks:
        print("no task activity found in", path)
        return

    total_processed = sum(t["processed"] for t in tasks.values())
    turns = len(tasks)
    clean = sum(1 for t in tasks.values() if t["forced"] == 0)
    forced_turns = sum(1 for t in tasks.values() if t["forced"] > 0)
    restored_turns = sum(1 for t in tasks.values() if t["restores"] > 0)

    print("== VITRIOL reuse audit ==")
    print(f"turns (tasks with prompt eval): {turns}")
    print(f"  clean (no forced reprocess): {clean}")
    print(f"  used checkpoint restore:     {restored_turns}")
    print(f"  FORCED full re-prefill:      {forced_turns}")
    print(f"total prompt tokens processed: {total_processed}")
    big = sorted(tasks.items(), key=lambda kv: -kv[1]["processed"])[:5]
    print("heaviest prefills (task, processed, eval_ms, forced, restores):")
    for tid, t in big:
        print(f"  task {tid}: {t['processed']} tok, {t['eval_ms']:.0f} ms,"
              f" forced={t['forced']}, restores={t['restores']}")
    verdict = "HEALTHY" if forced_turns == 0 or restored_turns >= forced_turns else "REUSE DEGRADED"
    print(f"verdict: {verdict}")


if __name__ == "__main__":
    main()
