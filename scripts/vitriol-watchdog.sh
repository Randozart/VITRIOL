#!/usr/bin/env bash
# vitriol-watchdog.sh — fallback restarter for hosts without systemd user units.
# Polls /health; on failure kills stale servers and relaunches via the launcher.
# Prefer the canonical systemd units (systemd/user/*.service) where available.
set -u

PORT="${VITRIOL_PORT:-8279}"
INTERVAL="${VITRIOL_WATCHDOG_SECS:-30}"
PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LAUNCHER="${PROJECT_DIR}/scripts/vitriol"

echo "[watchdog] guarding http://127.0.0.1:${PORT} every ${INTERVAL}s"
while true; do
    if ! curl -sf -m 5 "http://127.0.0.1:${PORT}/health" | grep -q '"ok"'; then
        echo "[watchdog $(date '+%F %T')] unhealthy — restarting server"
        killall -9 llama-server 2>/dev/null
        sleep 2
        export VITRIOL_KV_SCORE="${VITRIOL_KV_SCORE:-probe}"
        export VITRIOL_POOL_RESET="${VITRIOL_POOL_RESET:-1}"
        "$LAUNCHER" serve --detach \
            && echo "[watchdog] relaunched" \
            || echo "[watchdog] ERROR: relaunch failed (see gen log)"
    fi
    sleep "$INTERVAL"
done
