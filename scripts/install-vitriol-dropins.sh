#!/usr/bin/env bash
# Install VITRIOL systemd drop-ins (run with sudo; idempotent).
#   vitriol-autosave: sidecar writes to the engine mirror dir (/scratch/kvtier)
#   vitriol-server:   pin VITRIOL_KV_DISK_PRUNE=0 (never wipe the mirror at boot)
# Does NOT restart any unit - activation is a separate owner decision.
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "run with sudo: sudo $0" >&2
    exit 1
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

install -d -m 0755 \
    /etc/systemd/system/vitriol-autosave.service.d \
    /etc/systemd/system/vitriol-server.service.d

install -m 0644 "$HERE/systemd/vitriol-autosave.service.d/dir.conf" \
    /etc/systemd/system/vitriol-autosave.service.d/dir.conf
install -m 0644 "$HERE/systemd/vitriol-server.service.d/prune.conf" \
    /etc/systemd/system/vitriol-server.service.d/prune.conf

systemctl daemon-reload

echo "drop-ins installed:"
echo "  /etc/systemd/system/vitriol-autosave.service.d/dir.conf  (VITRIOL_SLOT_SAVE_DIR=/scratch/kvtier)"
echo "  /etc/systemd/system/vitriol-server.service.d/prune.conf  (VITRIOL_KV_DISK_PRUNE=0)"
echo "no units were restarted; enable/start when ready (restart-day checklist)."
