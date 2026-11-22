#!/usr/bin/env python3
"""Ranged-resume GGUF downloader with tree-API sha256 verification.

HF CDN stalls on this host (AGENTS.md Testing Protocol §6): plain downloads go
silent after ~16 MiB. This fetcher does ranged chunk downloads with
reconnect-on-stall and verifies the final file against the LFS oid from the
Hugging Face *tree* API (NOT the resolve-HEAD etag, which is a xetHash).

Usage:
    python3 scripts/fetch-gguf.py REPO REPO_ID OUT_DIR FILE_NAME SHA256 [SIZE]

    REPO      e.g. https://huggingface.co/<org>/<repo>/resolve/main
    SHA256    lfs.oid from the tree API (the authoritative checksum)
    SIZE      optional expected byte count (from the tree API)

Exit codes: 0 = ok + verified, 1 = failure (file left in place for inspection).
"""

import hashlib
import os
import subprocess
import sys
import time

CHUNK = 8 * 1024 * 1024  # 8 MiB per range request
RETRY_SLEEP_MAX = 30
MAX_RETRIES = 60  # per-chunk reconnect budget (CDN is flaky here)


def fetch_range(url: str, start: int, end: int, dst: str) -> None:
    """Fetch url[start..end] via curl and append to dst, retrying on failure.

    curl follows the xet-bridge redirect cleanly; urllib 400s on the
    redirected Range requests (observed 2026-11-22), so the transport is
    delegated to curl with a per-call progress meter off.
    """
    for attempt in range(MAX_RETRIES):
        rc = subprocess.run(
            [
                "curl",
                "-sSfL",
                "--fail",
                "--max-time",
                "300",
                "-H",
                f"Range: bytes={start}-{end}",
                "-o",
                dst + ".chunk",
                url,
            ]
        ).returncode
        if rc == 0:
            chunk_path = dst + ".chunk"
            size = os.path.getsize(chunk_path)
            with open(dst, "ab") as out, open(chunk_path, "rb") as ch:
                out.write(ch.read(size))
            os.remove(chunk_path)
            return
        # transport error (400/404/stall): back off and retry the same range
        if attempt == MAX_RETRIES - 1:
            raise SystemExit(
                f"fetch_range gave up after {MAX_RETRIES} retries: curl rc={rc}"
            )
        sleep_s = min(2 ** min(attempt, 6), RETRY_SLEEP_MAX)
        print(
            f"  retry {attempt + 1}/{MAX_RETRIES} at offset {start} (curl rc={rc}) in {sleep_s}s",
            flush=True,
        )
        time.sleep(sleep_s)


def download_file(
    url: str, out_dir: str, name: str, want_sha: str, want_size: int
) -> None:
    """Fetch one GGUF with resume + tree-API sha256 verification."""
    dst = os.path.join(out_dir, name)
    part = dst + ".part"

    if os.path.exists(part) and os.path.getsize(part) == 0:
        os.remove(part)

    have = os.path.getsize(part) if os.path.exists(part) else 0
    if have >= want_size:
        os.replace(part, dst)
    else:
        print(f"resuming {name}: {have}/{want_size} bytes already", flush=True)
        t0 = time.monotonic()
        last_have = have
        while have < want_size:
            end = min(have + CHUNK, want_size) - 1
            fetch_range(url, have, end, part)
            have = os.path.getsize(part)
            done_mb = have / 1e6
            total_mb = want_size / 1e6
            elapsed = max(time.monotonic() - t0, 1e-9)
            rate_mib = (have - last_have) / elapsed / 1e6
            print(
                f"  {done_mb:9.1f}/{total_mb:.1f} MiB "
                f"({100 * have / want_size:5.1f}%) {rate_mib:.1f} MiB/s",
                flush=True,
            )
            t0 = time.monotonic()
            last_have = have
        os.replace(part, dst)

    # verify against the tree-API oid
    h = hashlib.sha256()
    with open(dst, "rb") as f:
        for block in iter(lambda: f.read(8 * 1024 * 1024), b""):
            h.update(block)
    got = h.hexdigest()
    if got != want_sha.lower():
        raise SystemExit(
            f"SHA256 MISMATCH\n  want {want_sha}\n  got  {got}\n  file {dst}"
        )
    print(f"OK {name}: {os.path.getsize(dst)} bytes, sha256={got}", flush=True)


def main() -> None:
    if len(sys.argv) < 6:
        raise SystemExit(__doc__)
    url, out_dir, name, want_sha = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
    want_size = int(sys.argv[5]) if len(sys.argv) > 5 else None

    if want_size is None:
        raise SystemExit("SIZE is required (tree API reports it)")

    os.makedirs(out_dir, exist_ok=True)
    download_file(url, out_dir, name, want_sha, want_size)


if __name__ == "__main__":
    main()
