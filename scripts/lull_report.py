#!/usr/bin/env python3
"""VITRIOL LULL Phase 0 — parse VITRIOL_LULL instrumentation output.

Reads server stderr (file arg or stdin), aggregates per-device busy/idle
distributions and VRAM watermarks:

    python3 scripts/lull_report.py server.log

Output lines look like:
    VITRIOL_LULL dev=0 busy_ms=12.345
    VITRIOL_LULL dev=0 idle_ms=1.234
    VITRIOL_LULL dev=1 vram_free_mb=2048 vram_total_mb=8192
"""

import sys
from collections import defaultdict


def pct(sorted_vals, p):
    if not sorted_vals:
        return float("nan")
    idx = min(len(sorted_vals) - 1, int(round(p / 100.0 * (len(sorted_vals) - 1))))
    return sorted_vals[idx]


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "/dev/stdin"
    busy = defaultdict(list)
    idle = defaultdict(list)
    vram = {}

    with open(path, errors="replace") as fh:
        for line in fh:
            if not line.startswith("VITRIOL_LULL"):
                continue
            parts = line.split()
            try:
                dev = int(parts[1].split("=")[1])
                kv = dict(p.split("=", 1) for p in parts[2:])
            except (IndexError, ValueError):
                continue
            if "busy_ms" in kv:
                busy[dev].append(float(kv["busy_ms"]))
            elif "idle_ms" in kv:
                v = float(kv["idle_ms"])
                # idle_ms=-1 means event not complete yet (queue-backed, no lull)
                if v >= 0:
                    idle[dev].append(v)
            elif "vram_free_mb" in kv:
                prev = vram.get(dev)
                vram[dev] = (min(prev[0], int(kv["vram_free_mb"])) if prev else int(kv["vram_free_mb"]),
                             max(prev[1], int(kv["vram_free_mb"])) if prev else int(kv["vram_free_mb"]),
                             int(kv["vram_total_mb"]))

    print("== VITRIOL LULL Phase 0 report ==")
    for dev in sorted(set(busy) | set(idle)):
        b = sorted(busy.get(dev, []))
        i = sorted(idle.get(dev, []))
        skipped = len(busy.get(dev, [])) - len(b)  # placeholder, busy never negative
        print(f"-- dev {dev}")
        if b:
            print(f"   busy  n={len(b):6d}  p50={pct(b,50):9.3f}ms  p95={pct(b,95):9.3f}ms  "
                  f"max={b[-1]:9.3f}ms  sum={sum(b):12.1f}ms")
        else:
            print("   busy  n=     0")
        if i:
            zero = sum(1 for v in i if v < 0.001)
            print(f"   idle  n={len(i):6d}  p50={pct(i,50):9.3f}ms  p95={pct(i,95):9.3f}ms  "
                  f"max={i[-1]:9.3f}ms  sum={sum(i):12.1f}ms  (~zero lulls: {zero}/{len(i)})")
        else:
            print("   idle  n=     0  (device always queue-backed)")
        if dev in vram:
            lo, hi, tot = vram[dev]
            print(f"   vram  free_min={lo}MiB free_max={hi}MiB total={tot}MiB")

    if not busy and not idle:
        print("no VITRIOL_LULL lines found — was VITRIOL_LULL_PROFILE=1 set?")


if __name__ == "__main__":
    main()
