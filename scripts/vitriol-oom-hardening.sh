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
#      - the old USER units are stopped + disabled (files retained as
#        reference; re-enabling them would fight the system units)
#
# Idempotent: safe to re-run. Verifies at the end.

set -euo pipefail

USER_HOME="$(getent passwd randozart | cut -d: -f6)"
UNIT=vitriol-server.service
AUTO=vitriol-autosave.service

echo "==> [B] disk swapfile"
if ! swapon --show=NAME | grep -q '^/swapfile'; then
  if [ ! -f /swapfile ]; then
    fallocate -l 16G /swapfile
    chmod 600 /swapfile
    mkswap /swapfile >/dev/null
  fi
  swapon /swapfile
  grep -q '^/swapfile' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab
  echo "    /swapfile on"
else
  echo "    /swapfile already active"
fi

echo "==> [C] system-scope units"
# Stop + disable the legacy USER units first (engine + sidecar).
systemctl --user stop  "$UNIT" "$AUTO" 2>/dev/null || true
systemctl --user disable "$UNIT" "$AUTO" 2>/dev/null || true

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
echo "engine: $(systemctl is-active $UNIT)"
echo "sidecar: $(systemctl is-active $AUTO)"
EP=$(pgrep -f "llama-server -m" | head -1)
if [ -n "$EP" ]; then
  echo "engine oom_score_adj: $(cat /proc/$EP/oom_score_adj)  (expect -500)"
fi
swapon --show=NAME,SIZE,USED