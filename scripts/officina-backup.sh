#!/bin/bash
# officina-backup — nightly snapshots of the pi session store.
#
# Why: the session store is the operator's working memory (100+ sessions,
# three corruption events in Nov 2026). Quarantine/patch bugs are single
# points of failure; this is the cheap insurance.
#
# Usage:
#   officina-backup.sh              # one snapshot run (what the timer calls)
#   officina-backup.sh --dry-run    # show what would be copied, change nothing
#   officina-backup.sh --install    # write + enable the user-scope systemd units
#
# Restore (manual by design, I2):
#   rsync -a ~/.local/share/officina-backups/sessions/<YYYY-MM-DD>/ \
#         ~/.pi/agent/sessions/
#
# Retention: 14 daily snapshots. Lock: flock (overlapping runs no-op).
# Source is NEVER written; the engine and TUIs are untouched.
set -euo pipefail

SRC="$HOME/.pi/agent/sessions"
DST_ROOT="$HOME/.local/share/officina-backups/sessions"
KEEP_DAYS=14
DATE="$(date +%F)"

case "${1:-}" in
--install)
  mkdir -p "$HOME/.config/systemd/user"
  cat >"$HOME/.config/systemd/user/officina-backup.service" <<UNIT
[Unit]
Description=officina — snapshot the pi session store (14-day retention)
After=local-fs.target

[Service]
Type=oneshot
ExecStart=$HOME/Desktop/Projects/VITRIOL/scripts/officina-backup.sh
Nice=10
IOSchedulingClass=idle
UNIT
  cat >"$HOME/.config/systemd/user/officina-backup.timer" <<UNIT
[Unit]
Description=officina — nightly session-store backup

[Timer]
OnCalendar=*-*-* 04:00:00
Persistent=true
RandomizedDelaySec=300

[Install]
WantedBy=timers.target
UNIT
  systemctl --user daemon-reload
  systemctl --user enable --now officina-backup.timer
  systemctl --user --no-pager --lines=0 list-timers officina-backup.timer
  echo "installed: $(systemctl --user is-enabled officina-backup.timer)"
  ;;
--dry-run)
  exec rsync -a --dry-run --stats "$SRC/" "$DST_ROOT/$DATE/"
  ;;
"")
  mkdir -p "$DST_ROOT"
  exec 9>"$DST_ROOT.lock"
  flock -n 9 || { echo "officina-backup: another run holds the lock"; exit 0; }
  mkdir -p "$DST_ROOT"
  rsync -a "$SRC/" "$DST_ROOT/$DATE/"
  # retention: prune snapshot dirs (and nothing else) older than KEEP_DAYS
  find "$DST_ROOT" -maxdepth 1 -mindepth 1 -type d -name '20*' -mtime +"$KEEP_DAYS" -exec rm -rf {} +
  count=$(find "$DST_ROOT" -maxdepth 1 -mindepth 1 -type d -name '20*' | wc -l)
  size=$(du -sh "$DST_ROOT" | cut -f1)
  echo "officina-backup: $DATE done · $count snapshots · $size"
  ;;
*)
  echo "usage: officina-backup.sh [--dry-run|--install]" >&2
  exit 2
  ;;
esac
