#!/usr/bin/env python3
# PROVENANCE: own work (Apache-2.0 OR MIT). Parity-ladder concept inspired by
# OurobourOS docs/CONTRACTS.md (own work) — L0 byte-exact, L1 cos >= 0.999.
#
# diff.py A.idx A.bin B.idx B.bin [--perturb FILE:IDX:BIT]
#
# Compares two llama-vitriol-oracle captures node-by-node:
#   L0 rung: byte-exact match
#   L1 rung: f32 cos >= 0.999 and max-abs-eps report (interpret n/4 floats)
# Exits 0 if L1 passes (or L0 for fully exact), 2 on parity failure.

import json
import struct
import sys


def load(prefix_idx, prefix_bin):
    nodes = []
    with open(prefix_idx) as f:
        for line in f:
            nodes.append(json.loads(line))
    return nodes, open(prefix_bin, "rb").read()


def f32_list(raw):
    n = len(raw) // 4
    return struct.unpack(f"<{n}f", raw[: n * 4])


def cos_p(a, b):
    dot = sum(x * y for x, y in zip(a, b))
    na = sum(x * x for x in a) ** 0.5
    nb = sum(x * x for x in b) ** 0.5
    if na == 0 or nb == 0:
        return 1.0 if na == nb else 0.0
    return dot / (na * nb)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    a_idx, a_bin, b_idx, b_bin = args[0], args[1], args[2], args[3]
    perturb = None
    for a in sys.argv[1:]:
        if a.startswith("--perturb="):
            f, i, b = a.split("=", 1)[1].split(":")
            perturb = (f, int(i), int(b))

    na, da = load(a_idx, a_bin)
    nb, db = load(b_idx, b_bin)
    if perturb and perturb[0] == "A":
        off = na[perturb[1]]["off"]
        da = bytearray(da)
        da[off + perturb[2] // 8] ^= 1 << (perturb[2] % 8)
        da = bytes(da)
    if perturb and perturb[0] == "B":
        off = nb[perturb[1]]["off"]
        db = bytearray(db)
        db[off + perturb[2] // 8] ^= 1 << (perturb[2] % 8)
        db = bytes(db)

    if len(na) != len(nb):
        print(f"NOTE: node counts differ (A={len(na)} B={len(nb)}) - aligning by name")

    # align by name + occurrence (backends may fuse ops differently)
    def key_list(nodes):
        seen = {}
        out = []
        for t in nodes:
            k = t["name"]
            c = seen.get(k, 0)
            seen[k] = c + 1
            out.append((k, c))
        return out

    ka, kb = key_list(na), key_list(nb)
    idx_b = {}
    for j, k in enumerate(kb):
        idx_b.setdefault(k, []).append(j)

    pairs = []
    missing = []
    used = set()
    for i, k in enumerate(ka):
        cand = idx_b.get(k, [])
        j = None
        for c in cand:
            if c not in used:
                j = c
                break
        if j is None:
            missing.append(f"ONLY-A {k}#{i}")
        else:
            used.add(j)
            pairs.append((i, j))
    for j, k in enumerate(kb):
        if j not in used:
            missing.append(f"ONLY-B [{j}] {nb[j]['name']}")
    n = len(pairs)

    exact = 0
    l1_ok = 0
    failures = list(missing)
    for i, j in pairs:
        A, B = na[i], nb[j]
        tag = f"[{i}/{j}] {A['name']} {A['type']}{A['ne']}"
        if A["type"] != B["type"] or A["ne"] != B["ne"]:
            failures.append(f"META {tag} vs {B['type']}{B['ne']}")
            continue
        ra = da[A["off"] : A["off"] + A["n"]]
        rb = db[B["off"] : B["off"] + B["n"]]
        if len(ra) != A["n"] or len(rb) != B["n"]:
            failures.append(f"TRUNCATED {tag} (slice beyond file: idx offsets stale?)")
            continue
        if ra == rb:
            exact += 1
            l1_ok += 1
            continue
        if A["type"] == "f32":
            fa, fb = f32_list(ra), f32_list(rb)
            c = cos_p(fa, fb)
            maxd = max(abs(x - y) for x, y in zip(fa, fb))
            if c >= 0.999:
                l1_ok += 1
                print(f"L1-PASS  {tag}  cos={c:.6f} maxabs={maxd:.3e}")
            else:
                failures.append(f"L1-FAIL {tag} cos={c:.4f} maxabs={maxd:.3e}")
        else:
            failures.append(f"BYTES {tag} ({A['n']}B differ, non-f32)")

    print(f"\nnodes: {n} | byte-exact: {exact} | L1 pass: {l1_ok}/{n}")
    if failures:
        print("FAILURES:")
        for f in failures[:20]:
            print("  " + f)
    total_l1 = l1_ok == n and not missing
    print("PARITY:", "PASS" if total_l1 else "FAIL")
    sys.exit(0 if total_l1 else 2)


if __name__ == "__main__":
    main()
