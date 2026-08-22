#!/bin/bash
# REBIS Mercury gateway launcher (:8280) — supervised by callers.
cd "$(dirname "$0")/.."
exec python3 libvitriol/rebis_shim.py \
  --port "${REBIS_PORT:-8280}" \
  --luna-url "${REBIS_LUNA_URL:-http://127.0.0.1:8247}" \
  --sol-url "${REBIS_SOL_URL:-http://127.0.0.1:8279}" \
  ${REBIS_EXTRA:-}
