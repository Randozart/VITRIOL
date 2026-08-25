#!/usr/bin/env python3
"""VITRIOL slot persistence — startup restore + periodic disk autosave + hang watchdog.

Works with llama-server launched with --slot-save-path DIR. On start, waits
for /health then restores slot{N}.bin into slot N (warm context after any
restart). Then loops every VITRIOL_AUTOSAVE_SECS:

  - saves idle slots whose task counter changed since the previous tick
    (unchanged slots keep their checkpoint — no multi-GiB rewrites)
  - skips the tick entirely when /slots answers slowly (swap-thrash sentinel;
    never piles writes onto a struggling server)
  - forces `systemctl --user restart vitriol-server.service` when the SAME
    server PID stays health-deaf for ~60 s (Restart=only-on-exit leaves
    hung processes hung forever; this closes that gap)
  - raises oom_score_adj of big non-essential consumers (oom-shield)
  - warns when MemAvailable drops below a floor

Env:
  VITRIOL_PORT           server port            (default 8279)
  VITRIOL_AUTOSAVE_SECS  save interval seconds  (default 300)
  VITRIOL_SLOT_SAVE_DIR  checkpoint dir         (default ~/.vitriol/checkpoints)
  VITRIOL_OOM_SHIELD_ADJ raise non-essential big consumers' oom_score_adj
                         to this value each tick (0 disables; default 300)
  VITRIOL_OOM_SHIELD_MB  min RSS MiB for shielding (default 300)
  VITRIOL_HANG_STRIKES   health-fail polls before forced restart
                         (default 12 ≈ 60 s at 5 s polls; 0 disables)

OOM note: systemd --user clamps negative OOMScoreAdjust (no CAP_SYS_RESOURCE),
and same-uid writes can only RAISE another process's score. Shield therefore
makes opencode/firefox/etc. preferred oom victims instead of protecting the
server directly.

urllib-only by design (runs in a bare systemd user unit); the one subprocess
call is the hang-watchdog systemctl restart.
"""
import json
import os
import subprocess
import sys
import time
import urllib.request
import urllib.error

PORT = int(os.environ.get("VITRIOL_PORT", "8279"))
INTERVAL = int(os.environ.get("VITRIOL_AUTOSAVE_SECS", "300"))
SAVE_DIR = os.environ.get("VITRIOL_SLOT_SAVE_DIR",
                          os.path.expanduser("~/.vitriol/checkpoints"))
BASE = f"http://127.0.0.1:{PORT}"
HANG_STRIKES_MAX = int(os.environ.get("VITRIOL_HANG_STRIKES", "12"))


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


def activity_signature():
    """Two monotonic counters from /metrics. Identical signatures between
    ticks ⇒ no request touched the server at all — every idle checkpoint is
    still current and the whole save tick can be skipped (a 48k-token
    checkpoint is ~1 GiB; rewriting it while nothing happened is churn)."""
    try:
        req = urllib.request.Request(BASE + "/metrics")
        with urllib.request.urlopen(req, timeout=5) as r:
            txt = r.read().decode("utf-8", "replace")
        vals = {}
        for line in txt.splitlines():
            if line.startswith("llamacpp:prompt_tokens_total ") or \
               line.startswith("llamacpp:tokens_predicted_total "):
                k, v = line.split()
                vals[k] = v
        return (vals.get("llamacpp:prompt_tokens_total"),
                vals.get("llamacpp:tokens_predicted_total"))
    except Exception:
        return None  # unknown — behave conservatively


def save_idle(last_sig):
    """Save idle slots, unless the server's activity counters prove nothing
    changed since the previous successful pass."""
    sig = activity_signature()
    if sig is not None and last_sig.get("sig") == sig:
        return  # silent skip: counters frozen, checkpoints already current
    try:
        slots = get("/slots")
    except Exception as e:
        log(f"SERVER SLOW — skipping save tick ({e})")
        return
    os.makedirs(SAVE_DIR, exist_ok=True)
    ok = True
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
            ok = False
            if os.path.exists(fpath):
                os.remove(fpath)  # drop stale ring entry if slot was reset
        except Exception as e:
            log(f"save {fname} failed: {e}")
            ok = False
    if ok and sig is not None:
        last_sig["sig"] = sig


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


def mem_available_mb():
    try:
        with open("/proc/meminfo") as f:
            for line in f:
                if line.startswith("MemAvailable:"):
                    return int(line.split()[1]) >> 10
    except Exception:
        pass
    return -1


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "loop"
    if mode == "save":
        save_idle({})
        return
    if not wait_health():
        log("server never became healthy — exiting")
        sys.exit(1)
    log(f"server healthy at {BASE}; save dir={SAVE_DIR} interval={INTERVAL}s")
    restore_all()
    last_pid = server_pid()
    log(f"baseline server pid {last_pid}")
    last_save = 0.0
    last_sig = {}
    hang_strikes = 0
    while True:
        time.sleep(5)
        oom_shield()
        mem = mem_available_mb()
        if mem != -1 and mem < 500:
            log(f"WARNING: MemAvailable {mem} MiB — swap-thrash territory")
        pid = server_pid()
        if pid != last_pid:
            log(f"server generation change: {last_pid} -> {pid}")
            last_pid = pid
            hang_strikes = 0
            if pid is not None and wait_health(600):
                log("new instance healthy — replaying disk checkpoints")
                restore_all()
        elif pid is not None:
            # same instance as before — is it health-deaf? Restart=always
            # only acts on exit; a hung process stays hung unless we act.
            try:
                deaf = get("/health").get("status") != "ok"
            except Exception:
                deaf = True
            if deaf:
                hang_strikes += 1
                if HANG_STRIKES_MAX > 0 and hang_strikes >= HANG_STRIKES_MAX:
                    log(f"HANG DETECTED after {hang_strikes} strikes "
                        f"(~{hang_strikes * 5}s health-deaf, pid {pid}) "
                        f"— forcing restart")
                    subprocess.run(
                        ["systemctl", "--user", "restart",
                         "vitriol-server.service"],
                        check=False)
                    last_pid = None  # generation branch picks up + restores
            else:
                if hang_strikes:
                    log(f"hang cleared after {hang_strikes} strike(s)")
                hang_strikes = 0
        if time.time() - last_save >= INTERVAL:
            if pid is not None:
                save_idle(last_sig)
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
