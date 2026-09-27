#!/usr/bin/env python3
# Ranged-resume HF fetcher (CDN stall doctrine): 8MB Range chunks, stall
# timeout + retry, resume from local size. Usage: fetch_hf.py <url> <out>
import os, sys, time, urllib.request

URL, OUT = sys.argv[1], sys.argv[2]
CHUNK = 8 * 1024 * 1024
STALL = 90  # seconds without progress before reconnect

def local_size():
    return os.path.getsize(OUT) if os.path.exists(OUT) else 0

# total size probe
req = urllib.request.Request(URL, method="HEAD")
with urllib.request.urlopen(req, timeout=60) as r:
    total = int(r.headers["Content-Length"])
print(f"total {total/2**30:.1f} GiB", flush=True)

while True:
    pos = local_size()
    if pos >= total:
        print(f"DONE {pos} bytes", flush=True)
        break
    try:
        t0 = time.time()
        req = urllib.request.Request(URL, headers={"Range": f"bytes={pos}-"})
        with urllib.request.urlopen(req, timeout=STALL) as r, open(OUT, "ab") as f:
            while True:
                chunk = r.read(CHUNK)
                if not chunk:
                    break
                f.write(chunk)
                pos += len(chunk)
                if pos % (256 * 1024 * 1024) < CHUNK:
                    rate = (pos - local_size() + CHUNK) / max(time.time() - t0, 1) / 2**20
                    print(f"{pos/2**30:.1f}/{total/2**30:.1f} GiB ({rate:.0f} MiB/s)", flush=True)
    except Exception as e:
        print(f"reconnect at {pos/2**30:.2f} GiB: {e}", flush=True)
        time.sleep(3)
