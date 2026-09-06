#!/usr/bin/env python3
"""E34 analyzer: Zipf curves, never-touched fraction, cross-task transfer
from VITRIOL expert-usage profile CSVs.
Usage: vitriol-profile-analyze.py profile1.csv [profile2.csv ...]
With 2+ profiles, also reports cross-task top-K set overlap (warm-start
value) and the combined curve.
"""
import re, sys

N_AS_DEFAULT = 512

def load(path):
    slots, sizes = {}, {}
    for line in open(path):
        line = line.strip()
        m = re.match(r"# slot (\d+) base=(\S+) expert_size=(\d+)", line)
        if m:
            s = int(m.group(1)); slots[s] = {}; sizes[s] = int(m.group(3))
        elif line and not line.startswith(("#", "slot,")):
            s, e, c = map(int, line.split(","))
            slots[s][e] = c
    return slots, sizes

def mass_at_k(counts, K):
    vals = sorted(counts.values(), reverse=True)
    tot = sum(vals)
    return sum(vals[:min(K, len(vals))]) / tot if tot else 0.0

def top_set(counts, K):
    return {e for e, _ in sorted(counts.items(), key=lambda x: -x[1])[:K]}

def analyze(path, n_as=N_AS_DEFAULT):
    slots, sizes = load(path)
    ks = [8, 16, 32, 64, 96, 128, 192, 256, 384]
    unt = [1 - len(c) / n_as for c in slots.values()]
    total = sum(sum(c.values()) for c in slots.values())
    print(f"===== {path} =====")
    print(f"  tensors={len(slots)}  selections={total/1e6:.2f}M")
    print(f"  never-touched experts: {100*sum(unt)/len(unt):.1f}% mean "
          f"(min {100*min(unt):.1f}%, max {100*max(unt):.1f}%)")
    for K in ks:
        ms = [mass_at_k(c, K) for c in slots.values()]
        print(f"  top-{K:>3}: {100*sum(ms)/len(ms):>5.1f}% of traffic mass")
    return slots, sizes

def transfer(a, b, ks=(32, 64, 96, 128, 256)):
    print("===== TRANSFER (top-K set overlap, first->second profile) =====")
    for K in ks:
        ovs = [len(top_set(a[s], K) & top_set(b[s], K)) / K
               for s in a if s in b]
        print(f"  K={K:>3}: mean {100*sum(ovs)/len(ovs):.1f}% "
              f"(min {100*min(ovs):.1f}%, max {100*max(ovs):.1f}%)")

def combined(profiles, ks=(32, 64, 96, 128, 256), n_as=N_AS_DEFAULT):
    comb = {}
    for slots, _ in profiles:
        for s, c in slots.items():
            d = comb.setdefault(s, {})
            for e, v in c.items():
                d[e] = d.get(e, 0) + v
    print("===== COMBINED (all profiles summed) =====")
    for K in ks:
        ms = [mass_at_k(c, K) for c in comb.values()]
        print(f"  top-{K:>3}: {100*sum(ms)/len(ms):>5.1f}%")
    unt = [1 - len(c) / n_as for c in comb.values()]
    print(f"  combined never-touched: {100*sum(unt)/len(unt):.1f}%")

def main():
    args = sys.argv[1:]
    if not args:
        print(__doc__); sys.exit(1)
    loaded = []
    for p in args:
        slots, sizes = analyze(p)
        loaded.append((slots, sizes))
    if len(loaded) >= 2:
        transfer(loaded[0][0], loaded[1][0])
        combined(loaded)

main()
