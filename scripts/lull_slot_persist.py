#!/usr/bin/env python3
"""VITRIOL slot persistence — startup restore + periodic disk autosave.

Works with llama-server launched with --slot-save-path DIR. On start, waits
for /health then restores slot{N}.bin into slot N (warm context after any
restart). Then loops: every VITRIOL_AUTOSAVE_SECS, saves each idle slot
holding context to disk. OOM kills have no graceful window; this bounds data
loss to one interval.

Env:
  VITRIOL_PORT           server port            (default 8279)
  VITRIOL_AUTOSAVE_SECS  save interval seconds  (default 300)
  VITRIOL_SLOT_SAVE_DIR  checkpoint dir         (default ~/.vitriol/checkpoints)
  VITRIOL_OOM_SHIELD_ADJ raise non-essential big consumers' oom_score_adj
                         to this value each tick (0 disables; default 300)
  VITRIOL_OOM_SHIELD_MB  min RSS MiB for shielding (default 300)

OOM note: systemd --user clamps negative OOMScoreAdjust (no CAP_SYS_RESOURCE),
and same-uid writes can only RAISE another process's score. Shield therefore
makes opencode/firefox/etc. preferred oom victims instead of protecting the
server directly.

urllib-only by design (runs in a bare systemd user unit).
"""
import json
import os
import sys
import time
import urllib.request
import urllib.error

PORT = int(os.environ.get("VITRIOL_PORT", "8279"))
INTERVAL = int(os.environ.get("VITRIOL_AUTOSAVE_SECS", "300"))
SAVE_DIR = os.environ.get("VITRIOL_SLOT_SAVE_DIR",
                          os.path.expanduser("~/.vitriol/checkpoints"))
BASE = f"http://127.0.0.1:{PORT}"


def log(msg):
    print(f"[slot-persist {time.strftime('%H:%M:%S')}] {msg}", flush=True)


def post(path, body):
    req = urllib.request.Request(
        BASE + path,
        json.dumps(body).encode(),
        {"Content-Type": "application/json"},
        method="POST")
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.load(r)


def get(path):
    with urllib.request.urlopen(BASE + path, timeout=10) as r:
        return json.load(r)


def wait_health(timeout=1800):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            if get("/health").get("status") == "ok":
                return True
        except Exception:
            pass
        time.sleep(3)
    return False


def restore_all():
    """Restore slot{N}.bin into slot N. Tolerates missing/invalid files."""
    if not os.path.isdir(SAVE_DIR):
        return
    try:
        slots = get("/slots")
    except Exception as e:
        log(f"cannot list slots for restore: {e}")
        return
    for s in slots:
        sid = s["id"]
        fname = f"slot{sid}.bin"
        fpath = os.path.join(SAVE_DIR, fname)
        if not os.path.exists(fpath):
            continue
        try:
            res = post(f"/slots/{sid}?action=restore", {"filename": fname})
            log(f"restored {fname}: {res.get('n_restored', '?')} tokens "
                f"({res.get('timings', {}).get('restore_ms', '?')} ms)")
        except urllib.error.HTTPError as e:
            log(f"restore {fname} rejected: HTTP {e.code} "
                f"(model changed? stale state?) — skipping")
        except Exception as e:
            log(f"restore {fname} failed: {e}")


def save_idle():
    """Save every non-processing slot. The /slots payload does not expose
    token counts, so emptiness is judged server-side: saving an empty slot
    yields a tiny file and restoring it is a no-op."""
    try:
        slots = get("/slots")
    except Exception as e:
        log(f"save tick: cannot list slots: {e}")
        return
    os.makedirs(SAVE_DIR, exist_ok=True)
    for s in slots:
        if s.get("is_processing"):
            continue
        sid = s["id"]
        fname = f"slot{sid}.bin"
        fpath = os.path.join(SAVE_DIR, fname)
        try:
            res = post(f"/slots/{sid}?action=save", {"filename": fname})
            log(f"saved {fname}: {res.get('n_saved', '?')} tokens "
                f"({res.get('n_written', 0)} bytes)")
        except urllib.error.HTTPError as e:
            log(f"save {fname} skipped: HTTP {e.code}")
            if os.path.exists(fpath):
                os.remove(fpath)  # drop stale ring entry if slot was reset
        except Exception as e:
            log(f"save {fname} failed: {e}")


def server_pid():
    """Current llama-server PID or None. Used as a generation signal: a new
    PID means the server restarted (e.g. systemd resurrected it post-OOM)
    and disk checkpoints should be replayed into the fresh instance."""
    for p in os.listdir("/proc"):
        if not p.isdigit():
            continue
        try:
            with open(f"/proc/{p}/comm") as f:
                if f.read().strip() == "llama-server":
                    return int(p)
        except Exception:
            continue
    return None


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "loop"
    if mode == "save":
        save_idle()
        return
    if not wait_health():
        log("server never became healthy — exiting")
        sys.exit(1)
    log(f"server healthy at {BASE}; save dir={SAVE_DIR} interval={INTERVAL}s")
    restore_all()
    last_pid = server_pid()
    log(f"baseline server pid {last_pid}")
    last_save = 0.0
    while True:
        time.sleep(5)
        oom_shield()
        pid = server_pid()
        if pid != last_pid:
            log(f"server generation change: {last_pid} -> {pid}")
            last_pid = pid
            if pid is not None and wait_health(600):
                log("new instance healthy — replaying disk checkpoints")
                restore_all()
        if time.time() - last_save >= INTERVAL:
            if pid is not None:
                save_idle()
            last_save = time.time()


PROTECTED = {
    "llama-server", "hermes", "lull_slot_persist", "vitriol-tui",
    "systemd", "python3",
}
SHIELD_ADJ = int(os.environ.get("VITRIOL_OOM_SHIELD_ADJ", "300"))
SHIELD_MIN_RSS_MB = int(os.environ.get("VITRIOL_OOM_SHIELD_MB", "300"))


def _proc_rss_mb(pid):
    try:
        with open(f"/proc/{pid}/statm") as f:
            return int(f.read().split()[1]) * (os.sysconf("SC_PAGE_SIZE") >> 20)
    except Exception:
        return 0


def oom_shield():
    """Raise oom_score_adj of big non-essential consumers so the kernel's
    oom-killer prefers them over llama-server. Same-uid ptrace writes allow
    raising (never lowering) another process's score."""
    if SHIELD_ADJ <= 0:
        return
    me = os.getpid()
    shielded = 0
    for pid in os.listdir("/proc"):
        if not pid.isdigit() or int(pid) == me:
            continue
        try:
            with open(f"/proc/{pid}/comm") as f:
                comm = f.read().strip()
            if comm in PROTECTED:
                continue
            rss = _proc_rss_mb(pid)
            if rss < SHIELD_MIN_RSS_MB:
                continue
            with open(f"/proc/{pid}/oom_score_adj") as f:
                cur = int(f.read())
            if cur >= SHIELD_ADJ:
                continue
            with open(f"/proc/{pid}/oom_score_adj", "w") as f:
                f.write(str(SHIELD_ADJ))
            shielded += 1
            log(f"oom-shield: {comm}(pid {pid}, {rss} MiB) adj {cur}→{SHIELD_ADJ}")
        except Exception:
            continue
    if shielded:
        log(f"oom-shield: raised {shielded} process(es)")


if __name__ == "__main__":
    main()
