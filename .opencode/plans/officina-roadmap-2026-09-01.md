# Officina Roadmap — 2026-09-01

**Status anchor:** context pipeline proven (~20k live / ~200k offloaded, 10:1),
engine dual-slot (`--parallel 2`, VRAM flat thanks to `--kv-unified`),
`background-lane` v1 shipped, 454 extension tests green.

## 1. Dual-slot A/B — MEASURED, ADOPTED ✅

`officina/scripts/bench-dual-slot.py` against the live engine
(Lapis Occultus, ts 22,14, -c 81920, MTP n1):

| phase | result | criterion | verdict |
|---|---|---|---|
| A serial (4 jobs, one after another) | 107.1s wall, 9.6 t/s aggregate | — | baseline |
| B parallel (4 jobs, continuous batching) | 71.1s wall, **14.4 t/s aggregate** | ≥20% wall-clock win | **1.51× — PASS** |
| C foreground stall (8k prefill admitted mid-decode) | decode **11.3 t/s before AND during** | tail within noise | **zero stall — PASS** (chunked prefill at ubatch 64 behaves as Sarathi-Serve predicts) |

Per-job rates during parallel: 9.9 / 10.2 / 10.3 / 7.9 t/s — each slot a bit
slower than solo (12.5), aggregate well above. Depth criterion: KV pool is
unified and VRAM was flat, so no window was sacrificed.

**Consequence:** background-lane is cleared for daily use. Phase C's zero-stall
result also means the idle-gate could be relaxed later (jobs during foreground
decode are tolerable) — but keep the gate until card quality is trusted.

## 2. Next builds (ranked)

1. **Read-ahead digests** — background-lane job type 2: repo-map PageRank
   neighbors of the actively-edited module, pre-read while idle, digest cards
   into knowledge-inject. Compounds with the 10:1 context offload.
2. **Churn investigator** — on `edit-churn` loop detection, race an
   alternative fix on the fast lane (Large Language Monkeys: coverage grows
   with samples when verification is cheap — tests).
3. **Structured-output verdicts** — vendor upstream `structured-output` so
   lane cards are machine-parseable (needed for 2 to auto-apply).
4. **Lane-armed indicator** in session-panel (one gauge line: gate state +
   pending jobs).
5. **Glyph fallbacks** — ASCII substitutes when the terminal font lacks
   U+1F70x.

## 3. Small debts

- vitriol-tui: 2 pre-existing test failures (`tab_all_matches_labels`,
  `service_rows_reflect_port_trio`) from the HERMETIS/EMBED retirement
  refactor — clear while the dashboard is fresh.
- HTTPS push: credential helper points at a missing `/usr/bin/gh`; either
  reinstall gh or move remotes to SSH permanently.
- `SESSION_LOG_*` addendum for the dashboard elemental-glyph work.

## 4. Deferred (triggers recorded in the mining audit)

dynamic-tools (if schema tokens matter), questionnaire (multi-question),
github-issue-autocomplete (gh issue flow), modal-editor/bookmark (owner
preference), file-trigger (read-ahead prerequisite).
