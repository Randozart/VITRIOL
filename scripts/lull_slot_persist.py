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
  VITRIOL_PROACTIVE_MB   MemAvailable floor for proactive bounce: staying
                         below it for VITRIOL_PROACTIVE_TICKS consecutive
                         polls triggers checkpoint + clean server restart
                         BEFORE the swap-thrash hang (default 250; 0 disables)
  VITRIOL_PROACTIVE_TICKS sustained-low ticks before bounce (default 24 ≈ 2 min)
  VITRIOL_BOUNCE_COOLDOWN_SECS  min seconds between bounces (default 600)

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
PROACTIVE_MB = int(os.environ.get("VITRIOL_PROACTIVE_MB", "250"))
PROACTIVE_TICKS = int(os.environ.get("VITRIOL_PROACTIVE_TICKS", "24"))
BOUNCE_COOLDOWN = int(os.environ.get("VITRIOL_BOUNCE_COOLDOWN_SECS", "600"))


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
    changed since the previous successful pass.

    Empty-slot guard: --cache-idle-slots clears a slot's KV into the host RAM
    cache when a newer task claims it, which makes an occupied slot LOOK
    empty. Blindly saving would overwrite a multi-GiB warm checkpoint with a
    1 KiB stub (observed 2026-08-26 07:00:59). So every save lands in
    slotN.tmp.bin first and only replaces slotN.bin if it carries tokens —
    or the previous checkpoint was itself trivial."""
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
        tmp_path = os.path.join(SAVE_DIR, f"slot{sid}.tmp.bin")
        try:
            res = post(f"/slots/{sid}?action=save", {"filename": f"slot{sid}.tmp.bin"})
            n_saved = res.get("n_saved", 0) or 0
            written = res.get("n_written", 0) or 0
            prev_size = os.path.getsize(fpath) if os.path.exists(fpath) else 0
            if n_saved == 0 and prev_size > 10 * 1024 * 1024:
                # cleared-by-cache-idle slot: keep the richer stale checkpoint
                try:
                    os.remove(tmp_path)
                except OSError:
                    pass
                log(f"slot{sid} empty ({written} B) — preserved existing "
                    f"{prev_size >> 20} MiB checkpoint")
                continue
            os.replace(tmp_path, fpath)
            log(f"saved {fname}: {n_saved} tokens ({written} bytes)")
        except urllib.error.HTTPError as e:
            # server refused the save (e.g. slot was reset); keep any
            # existing checkpoint — stale-but-warm beats gone
            log(f"save {fname} skipped: HTTP {e.code}")
            ok = False
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
    low_streak = 0
    cooldown_until = 0.0
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
            low_streak = 0
            if pid is not None and wait_health(600):
                log("new instance healthy — replaying disk checkpoints")
                restore_all()
                last_sig.clear()
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

        # Proactive bounce: sustained memory exhaustion ends one of two ways —
        # an OOM kill or a swap-thrash hang (observed twice). Both are worse
        # than a scheduled, checkpointed restart. Fire while the server is
        # still responsive so the pre-bounce save actually lands.
        if (PROACTIVE_MB > 0 and mem != -1 and mem < PROACTIVE_MB
                and pid is not None and pid == last_pid
                and time.time() >= cooldown_until):
            low_streak += 1
            if low_streak >= PROACTIVE_TICKS:
                log(f"PROACTIVE BOUNCE: MemAvailable {mem} MiB for "
                    f"{low_streak} polls (~{low_streak * 5}s) — checkpoint "
                    f"and restart before the wedge")
                try:
                    save_idle(last_sig)  # best-effort; bounded timeouts
                except Exception as e:
                    log(f"pre-bounce save failed (continuing): {e}")
                subprocess.run(
                    ["systemctl", "--user", "restart",
                     "vitriol-server.service"],
                    check=False)
                last_pid = None
                cooldown_until = time.time() + BOUNCE_COOLDOWN
                log("bounce issued; cooldown until "
                    + time.strftime('%H:%M:%S', time.localtime(cooldown_until)))
        elif mem == -1 or mem >= PROACTIVE_MB:
            low_streak = 0

        if time.time() - last_save >= INTERVAL:
            if pid is not None:
                save_idle(last_sig)
            last_save = time.time()


PROTECTED = {
    "llama-server", "hermes", "lull_slot_persist", "vitriol-tui",
    "systemd", "python3",
}
# machine-specific extras (e.g. "firefox,opencode") without editing the file
PROTECTED.update(
    n.strip() for n in
    os.environ.get("VITRIOL_OOM_PROTECT_EXTRA", "").split(",") if n.strip()
)
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
