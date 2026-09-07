#!/bin/bash
# VITRIOL TUI launcher for the PTL/Arc laptop (Overlord-x8664).
#
# The laptop runs Flash-Next under `~/.config/systemd/user/vitriol-flashnext.service`
# which writes its stdout/stderr to /tmp/opencode/vitriol_gen.log (the path the
# TUI's gen-log tail reads). GPU telemetry comes from the Intel sysfs fallback
# in `intel.rs` (Arc B390). This wrapper just sets the env the desktop launcher
# would normally provide and execs the release binary.
set -euo pipefail

export VITRIOL_GEN_PORT="${VITRIOL_GEN_PORT:-8080}"
export COPULA_LOG_DIR="${COPULA_LOG_DIR:-/tmp/opencode}"
export VITRIOL_REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

exec "${VITRIOL_REPO}/vitriol-tui/target/release/vitriol-tui" "$@"