#!/usr/bin/env python3
# fetch-hf.py - ranged-resume downloader for HuggingFace LFS files.
# Rationale (AGENTS.md 2026-08-31): `hf download` stalls on this host
# (~16 MiB then silence). This tool sends Range headers, reconnects on
# stall, and verifies sha256 against the tree-API lfs.oid before done.
#
# Usage: fetch-hf.py <manifest> [--log-every-mb N]
# Manifest line format (| separated):
#   url|expected_sha256|output_path
import hashlib
import os
import sys
import time
import urllib.request

STALL_TIMEOUT = 45          # seconds without data before reconnect
CHUNK = 1 << 20             # 1 MiB reads
MAX_RETRIES = 30


def fmt_speed(bps: float) -> str:
    for unit in ("B/s", "KiB/s", "MiB/s", "GiB/s"):
        if bps < 1024 or unit == "GiB/s":
            return f"{bps:.1f} {unit}"
        bps /= 1024
    return f"{bps:.1f} B/s"


def fetch(url: str, want_sha: str, out_path: str, log_every_mb: int) -> bool:
    out_path = os.path.expanduser(out_path)
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    have = os.path.getsize(out_path) if os.path.exists(out_path) else 0

    # total size via HEAD
    req = urllib.request.Request(url, method="HEAD")
    with urllib.request.urlopen(req, timeout=60) as r:
        total = int(r.headers.get("Content-Length", 0))
    if total and have == total:
        print(f"[skip] {out_path} already complete ({total} bytes)")
        complete = True
    else:
        if have:
            print(f"[resume] {out_path} at {have}/{total}")
        complete = False

    attempt = 0
    ran = False
    ok = complete  # skipped-complete files go straight to verification
    while not ok and attempt < MAX_RETRIES:
        ran = True
        attempt += 1
        try:
            have = os.path.getsize(out_path) if os.path.exists(out_path) else 0
            headers = {"Range": f"bytes={have}-"} if have else {}
            req = urllib.request.Request(url, headers=headers)
            mode = "ab" if have else "wb"
            t0 = time.time()
            last_data = time.time()
            got_since_mark = 0
            next_mark = log_every_mb << 20
            with urllib.request.urlopen(req, timeout=STALL_TIMEOUT) as r, \
                 open(out_path, mode) as f:
                while True:
                    chunk = r.read(CHUNK)
                    if not chunk:
                        break
                    f.write(chunk)
                    got_since_mark += len(chunk)
                    if got_since_mark >= next_mark:
                        dt = time.time() - t0
                        pos = have + got_since_mark
                        pct = 100.0 * pos / total if total else 0.0
                        print(f"  {os.path.basename(out_path)}: "
                              f"{pos >> 20} MiB {pct:5.1f}% "
                              f"{fmt_speed(got_since_mark / dt)}", flush=True)
                        next_mark += log_every_mb << 20
                    if time.time() - last_data > STALL_TIMEOUT:
                        raise TimeoutError("stall")
                    last_data = time.time()
            ok = True
            break
        except Exception as e:
            wait = min(2 ** attempt, 60)
            print(f"[retry {attempt}] {type(e).__name__}: {e} "
                  f"(wait {wait}s)", flush=True)
            time.sleep(wait)
    if ran and not ok:
        print(f"[FAIL] {out_path}: retries exhausted")
        return False

    size = os.path.getsize(out_path)
    if total and size != total:
        print(f"[FAIL] {out_path}: size {size} != expected {total}")
        return False
    h = hashlib.sha256()
    with open(out_path, "rb") as f:
        while chunk := f.read(1 << 22):
            h.update(chunk)
    if h.hexdigest() != want_sha:
        print(f"[FAIL] {out_path}: sha256 mismatch "
              f"(got {h.hexdigest()[:16]}..., want {want_sha[:16]}...)")
        os.remove(out_path)
        return False
    print(f"[ok] {out_path} sha256 verified", flush=True)
    return True


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    manifest = sys.argv[1]
    log_every_mb = 200
    if "--log-every-mb" in sys.argv:
        log_every_mb = int(sys.argv[sys.argv.index("--log-every-mb") + 1])
    failed = []
    with open(os.path.expanduser(manifest)) as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            url, sha, out = line.split("|")
            print(f"=== {os.path.basename(out)}", flush=True)
            if not fetch(url, sha, out, log_every_mb):
                failed.append(out)
    if failed:
        print(f"DONE with failures: {failed}")
        return 1
    print("DONE all verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())
