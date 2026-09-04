#!/usr/bin/env bash
# VITRIOL OOM hardening — 2026-09-04 (run as root: sudo bash scripts/vitriol-oom-hardening.sh)
#
# B. 16G disk swapfile — zram is RAM-backed; disk swap gives the kernel real
#    reclaim headroom so global OOM stops firing under bursts.
# C. Engine + persistence sidecar move to SYSTEM scope:
#      - vitriol-server.service  (User=randozart, OOMScoreAdjust=-500 REAL,
#                                 effective negative — the engine becomes the
#                                 LAST thing the OOM killer chooses)
#      - vitriol-autosave.service (User=randozart, system bus; the sidecar's
#                                 restart calls target the system unit via
#                                 VITRIOL_SYSTEMCTL=systemctl)
#      - polkit rule letting randozart manage exactly those two units
#      - the USER units are STOPPED, DISABLED, and their files moved aside
#        (vitriol-server.service -> .disabled) so they can never load again.
#        NOTE: this script runs as ROOT, so the user manager must be reached
#        via runuser + XDG_RUNTIME_DIR — a bare `systemctl --user` as root
#        silently talks to a nonexistent root user bus and is a no-op.
#
# Idempotent: safe to re-run. Verifies at the end and FAILS if the engine
# is not under the system unit with oom_score_adj -500.

set -euo pipefail

USER_HOME="$(getent passwd randozart | cut -d: -f6)"
USER_UID="$(getent passwd randozart | cut -d: -f3)"
UNIT=vitriol-server.service
AUTO=vitriol-autosave.service

# Reach the USER systemd manager from a root context.
user_sysctl() {
  runuser -u randozart -- env XDG_RUNTIME_DIR="/run/user/${USER_UID}" systemctl --user "$@"
}

echo "==> [B] disk swapfile"
if ! swapon --show=NAME | grep -q '^/swapfile'; then
  rm -f /swapfile  # drop any failed/partial sparse file
  truncate -s 0 /swapfile
  # btrfs requires NOCOW on a fresh empty file BEFORE writing, and the file
  # must not be sparse (fallocate holes -> swapon "Invalid argument").
  FS="$(findmnt -no FSTYPE -T "$USER_HOME" 2>/dev/null || true)"
  if [ "$FS" = "btrfs" ]; then
    chattr +C /swapfile
  fi
  dd if=/dev/zero of=/swapfile bs=1M count=16384 status=progress conv=fsync
  chmod 600 /swapfile
  mkswap /swapfile >/dev/null
  swapon /swapfile
  grep -q '^/swapfile' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab
  echo "    /swapfile on ($FS)"
else
  echo "    /swapfile already active"
fi

echo "==> [C] retire the USER units (stop + disable + move files aside)"
user_sysctl stop  "$UNIT" "$AUTO" 2>/dev/null || true
user_sysctl disable "$UNIT" "$AUTO" 2>/dev/null || true
for f in "$UNIT" "$AUTO"; do
  if [ -f "$USER_HOME/.config/systemd/user/$f" ]; then
    mv "$USER_HOME/.config/systemd/user/$f" "$USER_HOME/.config/systemd/user/$f.disabled"
    echo "    moved $USER_HOME/.config/systemd/user/$f -> .disabled"
  fi
done
user_sysctl daemon-reload 2>/dev/null || true

echo "==> [C] system-scope units"
cat > /etc/systemd/system/$UNIT <<EOF
[Unit]
Description=VITRIOL llama-server (Lapis Occultus, dual-slot) — SYSTEM scope
Wants=$AUTO

[Service]
Type=forking
User=randozart
Group=randozart
Environment=HOME=$USER_HOME
WorkingDirectory=$USER_HOME/Desktop/Projects/VITRIOL
ExecStartPre=-$USER_HOME/Desktop/Projects/VITRIOL/scripts/vitriol stop
ExecStart=$USER_HOME/Desktop/Projects/VITRIOL/scripts/vitriol serve --detach
Restart=always
RestartSec=5
# SYSTEM scope: real negative OOMScoreAdjust applies (CAP_SYS_RESOURCE).
# The engine is now the LAST process the OOM killer selects.
OOMScoreAdjust=-500
TimeoutStopSec=30

[Install]
WantedBy=multi-user.target
EOF

cat > /etc/systemd/system/$AUTO <<EOF
[Unit]
Description=VITRIOL slot persistence (system scope; hang watchdog + oom-shield + proactive bounce)
After=$UNIT

[Service]
Type=simple
User=randozart
Group=randozart
Environment=HOME=$USER_HOME
Environment=VITRIOL_SYSTEMCTL=systemctl
Environment=VITRIOL_PORT=8279
ExecStart=/usr/bin/python3 $USER_HOME/Desktop/Projects/VITRIOL/scripts/lull_slot_persist.py
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF

echo "==> [C] polkit rule (randozart may manage only the two VITRIOL units)"
cat > /etc/polkit-1/rules.d/10-vitriol.rules <<EOF
polkit.addRule(function (action, subject) {
  if (action.id === "org.freedesktop.systemd1.manage-units" &&
      subject.user === "randozart" &&
      (action.lookup("unit") === "vitriol-server.service" ||
       action.lookup("unit") === "vitriol-autosave.service")) {
    return polkit.Result.YES;
  }
});
EOF

systemctl daemon-reload
systemctl enable --now $AUTO $UNIT

echo "==> verify"
sleep 6
echo "system engine: $(systemctl is-active $UNIT)"
echo "system sidecar: $(systemctl is-active $AUTO)"
echo "user units: $(user_sysctl is-active $UNIT $AUTO 2>/dev/null | tr '\n' ' ')"
EP=$(pgrep -f "llama-server -m" | head -1)
if [ -n "$EP" ]; then
  ADJ="$(cat /proc/$EP/oom_score_adj)"
  CG="$(cat /proc/$EP/cgroup)"
  echo "engine pid $EP adj=$ADJ cgroup=$CG"
  if [ "$ADJ" != "-500" ] || ! echo "$CG" | grep -q "system.slice/vitriol-server.service"; then
    echo "!!! FAIL: engine is not under the system unit with adj -500"
    exit 1
  fi
  echo "    OK: engine protected (last OOM victim)"
else
  echo "!!! FAIL: no engine process found"
  exit 1
fi
swapon --show=NAME,SIZE,USED