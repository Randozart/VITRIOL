#!/usr/bin/env python3
"""Audit the VITRIOL Optane/KV disk cache (--slot-save-path).

Read-only. Parses every cache-*.bin, recovers the token prefix, checks the
filename hash and (for v2 files) the model/config identity block, and prints
a table plus totals.

Format (server-task.cpp):
  magic   u32  'ggpc' = 0x63706767
  version u32  1 = no identity, 2 = identity block follows
  identity  48 bytes (v2 only):
      model_key u64, build_key u64, n_ctx u32, type_k u32, type_v u32,
      n_stream u32, reserved[4] u32
  n_main  u64   bytes of main (target) KV state
  n_drft  u64   bytes of draft (MTP) KV state
  n_pack  u64   bytes of serialized server_tokens
  data_main[n_main] data_drft[n_drft] packed[n_pack]

packed server_tokens (server-common.cpp):
  u32 marker (LLAMA_TOKEN_NULL), u32 version, u32 n_tokens, n_tokens*u32 ids,
  then media keys/chunks (empty for text-only sessions).
"""

import argparse
import json
import os
import struct
import sys
from collections import Counter

MAGIC = 0x63706767
FNV_OFFSET = 1469598103934665603
FNV_PRIME = 1099511628211
LLAMA_TOKEN_NULL = 0xFFFFFFFF

VERDICT = {0: "unknown", 1: "ok", 2: "mismatch", 3: "v1-unverified"}


def fnv_token_hash(tokens):
    h = FNV_OFFSET
    n = len(tokens)

    def mix(v):
        nonlocal h
        h ^= v & 0xFFFFFFFFFFFFFFFF
        h = (h * FNV_PRIME) & 0xFFFFFFFFFFFFFFFF

    mix(n)
    head = min(n, 64)
    for i in range(head):
        mix(tokens[i])
    for i in range(n - min(n, 64), n):
        mix(tokens[i])
    return h


def parse_tokens(packed):
    if len(packed) < 12:
        raise ValueError("packed too small")
    (marker,) = struct.unpack_from("<I", packed, 0)
    if marker != LLAMA_TOKEN_NULL:
        # legacy plain token list
        n = len(packed) // 4
        return list(struct.unpack_from("<%dI" % n, packed, 0))
    (version,) = struct.unpack_from("<I", packed, 4)
    (n_tokens,) = struct.unpack_from("<I", packed, 8)
    if 12 + 4 * n_tokens > len(packed):
        raise ValueError("token blob overruns packed size")
    return list(struct.unpack_from("<%dI" % n_tokens, packed, 12))


def parse_file(path):
    size = os.path.getsize(path)
    with open(path, "rb") as f:
        head = f.read(8)
        if len(head) < 8:
            raise ValueError("truncated header")
        magic, version = struct.unpack("<II", head)
        if magic != MAGIC:
            raise ValueError("bad magic %08x" % magic)
        identity = None
        if version >= 2:
            raw = f.read(48)
            if len(raw) < 48:
                raise ValueError("truncated identity")
            model_key, build_key, n_ctx, type_k, type_v, n_stream = struct.unpack_from(
                "<QQIIII", raw, 0)
            identity = {
                "model_key": model_key,
                "build_key": build_key,
                "n_ctx": n_ctx,
                "type_k": type_k,
                "type_v": type_v,
                "n_stream": n_stream,
            }
        raw = f.read(24)
        if len(raw) < 24:
            raise ValueError("truncated sizes")
        n_main, n_drft, n_pack = struct.unpack("<QQQ", raw)
        if n_pack <= 0 or n_pack > (1 << 30):
            raise ValueError("bad n_pack %d" % n_pack)
        f.seek(size - n_pack)
        packed = f.read(n_pack)
        if len(packed) != n_pack:
            raise ValueError("token blob short read")

    tokens = parse_tokens(packed)
    return {
        "version": version,
        "identity": identity,
        "n_main": n_main,
        "n_drft": n_drft,
        "n_pack": n_pack,
        "n_tokens": len(tokens),
        "token_hash": fnv_token_hash(tokens),
    }


def filename_hash(name):
    # v1: cache-<16hex>.bin ; v2: cache-<modelkey16>-<tokenhash16>.bin
    base = name[len("cache-"):-len(".bin")]
    parts = base.split("-")
    return parts[-1]


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dir", default="/scratch/kvtier",
                    help="disk cache directory (default: /scratch/kvtier)")
    ap.add_argument("--json", action="store_true", help="emit JSON instead of a table")
    args = ap.parse_args()

    if not os.path.isdir(args.dir):
        print("no such directory: %s" % args.dir, file=sys.stderr)
        return 2

    entries = []
    for name in sorted(os.listdir(args.dir)):
        if not (name.startswith("cache-") and name.endswith(".bin")):
            continue
        path = os.path.join(args.dir, name)
        rec = {"name": name, "size": os.path.getsize(path)}
        try:
            info = parse_file(path)
            rec.update(info)
            rec["hash_ok"] = filename_hash(name) == "%016x" % info["token_hash"]
            rec["verdict"] = "ok" if info["version"] >= 2 else "v1-unverified"
            rec["error"] = None
        except Exception as exc:  # noqa: BLE001 - report and continue
            rec["error"] = str(exc)
            rec["verdict"] = "damaged"
        entries.append(rec)

    total = sum(e["size"] for e in entries)
    by_verdict = Counter(e["verdict"] for e in entries)
    by_version = Counter(e.get("version", 0) for e in entries)
    hash_bad = [e for e in entries if e.get("error") is None and not e.get("hash_ok", True)]

    if args.json:
        print(json.dumps({
            "dir": args.dir,
            "count": len(entries),
            "bytes": total,
            "by_verdict": dict(by_verdict),
            "by_version": dict(by_version),
            "entries": entries,
        }, indent=2))
        return 0

    print("dir: %s" % args.dir)
    print("%-44s %10s %10s %8s %4s %18s %s" % (
        "name", "bytes", "tokens", "version", "hash", "model_key", "verdict"))
    for e in sorted(entries, key=lambda x: -x["size"]):
        if e.get("error"):
            print("%-44s %10d %10s %8s %4s %18s %s" % (
                e["name"], e["size"], "-", "-", "-", "-", "DAMAGED: " + e["error"]))
            continue
        mk = e["identity"]["model_key"] if e.get("identity") else 0
        print("%-44s %10d %10d %8d %4s %18x %s" % (
            e["name"], e["size"], e["n_tokens"], e["version"],
            "ok" if e.get("hash_ok") else "BAD", mk, e["verdict"]))

    print()
    print("files: %d   bytes: %d (%.1f GiB)" % (len(entries), total, total / (1 << 30)))
    print("verdicts: %s" % dict(by_verdict))
    print("versions: %s" % dict(by_version))
    if hash_bad:
        print("WARNING: %d file(s) whose filename hash does not match the token blob" % len(hash_bad))
    return 0


if __name__ == "__main__":
    sys.exit(main())
