#!/usr/bin/env python3
"""Ranged HF CDN downloader for this host (AGENTS rule 6): plain hf download
stalls after ~16 MiB. Range + reconnect-on-stall + sha256 verify against the
tree-API lfs.oid (the resolve-HEAD etag is the xetHash, NOT the sha256).

usage: fetch-hf-ranged.py <repo> <path> <dest> [expected_sha256]
e.g.:  fetch-hf-ranged.py Qwen/Qwen2.5-0.5B-Instruct-GGUF \
         qwen2.5-0.5b-instruct-q4_k_m.gguf ~/Models/qwen2.5-0.5b-instruct-q4_k_m.gguf
"""
import hashlib
import json
import os
import sys
import time
import urllib.request

CHUNK = 4 * 1024 * 1024
STALL_S = 30
MAX_TRIES = 40


def tree_oid(repo: str, path: str) -> str:
    url = f"https://huggingface.co/api/models/{repo}/tree/main?recursive=true"
    with urllib.request.urlopen(url, timeout=60) as r:
        entries = json.load(r)
    for e in entries:
        if e.get("path") == path and e.get("lfs"):
            return e["lfs"]["oid"]
    raise SystemExit(f"lfs oid not found for {repo}/{path}")


def main() -> None:
    if len(sys.argv) < 4:
        raise SystemExit(__doc__)
    repo, path, dest = sys.argv[1], sys.argv[2], sys.argv[3]
    expected = sys.argv[4] if len(sys.argv) > 4 else tree_oid(repo, path)
    url = f"https://huggingface.co/{repo}/resolve/main/{path}"

    done = os.path.getsize(dest) if os.path.exists(dest) else 0
    tries = 0
    with open(dest, "ab") as f:
        while True:
            tries += 1
            if tries > MAX_TRIES:
                raise SystemExit("too many retries, giving up")
            req = urllib.request.Request(url, headers={"Range": f"bytes={done}-"})
            try:
                with urllib.request.urlopen(req, timeout=STALL_S) as r:
                    if r.status == 200 and done > 0:
                        # server ignored Range: restart from scratch
                        f.seek(0)
                        f.truncate()
                        done = 0
                    total = int(r.headers.get("Content-Length", 0)) + done
                    while True:
                        chunk = r.read(CHUNK)
                        if not chunk:
                            break
                        f.write(chunk)
                        done += len(chunk)
                        print(f"\r{done}/{total or '?'} bytes", end="", flush=True)
            except Exception as e:  # noqa: BLE001 - reconnect on any stall
                print(f"\nreconnect after {e!r} at {done} bytes", flush=True)
                time.sleep(2)
                continue
            break
    print()

    h = hashlib.sha256()
    with open(dest, "rb") as f:
        for chunk in iter(lambda: f.read(CHUNK), b""):
            h.update(chunk)
    if h.hexdigest() != expected:
        raise SystemExit(f"sha256 mismatch: {h.hexdigest()} != {expected}")
    print(f"ok {dest} sha256={expected}")


if __name__ == "__main__":
    main()
