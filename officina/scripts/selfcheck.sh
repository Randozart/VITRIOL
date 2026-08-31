#!/usr/bin/env bash
# selfcheck.sh — the self-sufficiency gate (docs/SELF-SUFFICIENCY-2026-08-31.md §SS5).
#
# Asserts VITRIOL + Officina stand alone: no external project paths in the
# live tree, officina test suites green, CLI tests green, config valid.
# Run before every release tag. Opt-in live checks (engine, PTY) run with
# --live. Exit 1 on any failure.

set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"   # officina/
VITRIOL="$(cd "$HERE/.." && pwd)"
FAIL=0

ok()   { echo "  [PASS] $1"; }
bad()  { echo "  [FAIL] $1"; FAIL=1; }
skip() { echo "  [SKIP] $1"; }

echo "── 1. external path grep (self-sufficiency) ──"
HITS=$(grep -rn "Projects/little-coder\|Projects/hermes-agent\|Projects/trismegistus" \
    "$VITRIOL/officina/.pi" "$VITRIOL/officina/cli" "$VITRIOL/officina/officina.mjs" \
    --exclude-dir node_modules --exclude-dir .git \
    --include="*.ts" --include="*.mjs" --include="*.py" --include="*.sh" \
    2>/dev/null | grep -v "^\s*#" | grep -v "// " | grep -v "legacy little-coder" || true)
if [[ -z "$HITS" ]]; then ok "no live external project paths"; else bad "external paths in live code:"; echo "$HITS" | head -5; fi

echo "── 2. officina tests + typecheck ──"
if (cd "$HERE" && npx vitest --run .pi/extensions >/dev/null 2>&1); then ok "vitest extensions"; else bad "vitest extensions"; fi
if (cd "$HERE" && npm run typecheck >/dev/null 2>&1); then ok "typecheck"; else bad "typecheck"; fi

echo "── 2b. Vitriolum palette parity (vitriol-tui/src/theme.rs ↔ theme/officina.json ↔ .pi/extensions/_shared/vitriolum.ts) ──"
PAL_OK=1
for hex in 0d1117 ffd700 00ffff 39ff14 ff4444 ff5f1f 8b949e e0e0e0 2e5fa3; do
  if grep -qi "$hex" "$VITRIOL/officina/theme/officina.json" \
    && grep -qi "$hex" "$VITRIOL/officina/.pi/extensions/_shared/vitriolum.ts"; then
    ok "palette: #$hex (json + extensions)"
  else
    bad "palette: #$hex drifted (check officina.json / _shared/vitriolum.ts)"
    PAL_OK=0
  fi
done

echo "── 3. cli tests ──"
if (cd "$HERE/cli" && python3 -m pytest tools/tests -q >/dev/null 2>&1) \
    || (cd "$HERE/cli" && "${HOME}/venvs/tris/bin/python" -m pytest tools/tests -q >/dev/null 2>&1); then
  ok "cli pytest"
else
  bad "cli pytest"
fi

echo "── 3b. provenance headers (docs/PROVENANCE.md registry) ──"
  PROV_OK=1
  check_prov() {
    local f="$1" marker="$2"
    if [[ -f "$VITRIOL/officina/.pi/extensions/$f" ]] && grep -q "$marker" "$VITRIOL/officina/.pi/extensions/$f"; then
      ok "provenance: $f"
    else
      bad "provenance missing in $f (expect '$marker')"
      PROV_OK=0
    fi
  }
  check_prov "llama-cpp-provider/index.ts" "Vendored 2026-08-31"
  check_prov "caveman/compress.ts" "Provenance: ported"
  check_prov "injection-guard/index.ts" "Provenance: ported"
  check_prov "memory-extractor/index.ts" "Provenance: ported"
  check_prov "vitriol-decode/braille.ts" "ported from VITRIOL"
  check_prov "memory/index.ts" "SS2 gateway fold-in"

  echo "── 4. config + keybindings ──"
if [[ -f "${HOME}/.config/trismegistus/config.yaml" ]]; then ok "unified config present"; else bad "unified config missing"; fi
KB="${HOME}/.pi/agent/keybindings.json"
if [[ -f "$KB" ]] && grep -q '"tui.input.tab"' "$KB"; then ok "TAB unbound (no conflict banner)"; else skip "keybindings.json not written yet (first officina run writes it)"; fi

if [[ "${1:-}" == "--live" ]]; then
  echo "── 5. live engine checks ──"
  if curl -sf http://127.0.0.1:8279/health >/dev/null 2>&1; then
    ok "engine healthy on 8279"
    R="$(LLAMACPP_API_KEY=none timeout 300 node "$HERE/officina.mjs" -p 'Reply with exactly: SELFCHECK-OK' 2>&1 | tail -1)"
    if [[ "$R" == *SELFCHECK-OK* ]]; then ok "one-shot round trip"; else bad "one-shot: $R"; fi
  else
    skip "engine down — start it (vitriol serve) for live checks"
  fi
fi

echo ""
if [[ $FAIL -eq 0 ]]; then echo "SELFCHECK: PASS"; else echo "SELFCHECK: FAIL"; exit 1; fi
