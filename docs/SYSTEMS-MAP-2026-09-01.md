# Systems map — who consumes what — 2026-09-01

**Purpose:** one page answering "what does each VITRIOL-side system exist
for, and who uses it?" after the Officina fold-in. Dispositions follow
docs/DEPRECATION-AUDIT-2026-09-01.md. This is a LIVE document: new systems
get a row here (with consumer + status) in the same commit that builds
them.

## The two gauges (owner clarification — not duplication)

- **`vitriol tui`** — the BACKGROUND ops console. You open it when you
  want machine state while working elsewhere: VRAM per GPU, service
  health, bounce/oom-shield activity, checkpoint liveness.
- **Officina's decode panel** — the IN-SESSION readout. Exists so you
  never swap to the TUI to see how far along decoding is: slot gauge,
  tok/s, boot decode totals, rendered under the editor from /metrics +
  /slots.

Different moments, same truth. Both stay.

## Inventory

| System | Consumed by | Status |
|---|---|---|
| `llama.cpp` fork, DMA executor, KV/cache managers | engine | CORE |
| `vitriol-server.service` (llama-server + endpoints) | Officina, Hermes-era tools, curl | CORE |
| `/health` `/props` `/slots` `/metrics` | officina (coupling probe, decode widget, KV gate), validate | CORE |
| checkpoint/restore endpoints + `vitriol-checkpoint` ext + `/rewind` | Officina continuity | CORE |
| cert suite, profiles, fingerprint emission | engine gating, provenance | CORE |
| `vitriol serve` / `stop` / `run` / `bench` / `calibrate` / `config` / `setup` | operator | CORE |
| `vitriol-tui` (`vitriol tui`) | operator, background ops | KEEP (see gauges note) |
| oom-shield, sidecar/watchdog, proactive bounce | engine survival | CORE |
| `officina/` workshop + `vitriol officina` | owner, daily | CORE |
| `tris` CLI | folded — `validate`/`lanes`/`budget`/`ledger-ingest`/`perms-sync`/`status` now exec via `vitriol` | RETIRED 2026-09-01 (symlink removed; implementation at `officina/cli`) |
| `tris chat` / hermes-bridge ext | — | RETIRED (SS2a) |
| memory: `~/.vitriol/officina/memory/` + per-project `.officina/MEMORY.md` | Officina memory ext | CORE (SS2a-b) |
| injection-guard (TS) | Officina ingested-content screening | CORE (SS2a) |
| caveman compressor | Officina, DARK (`TRIS_CAVEMAN=1`) | ARMED-OFF (SS2b) |
| memory-extractor → curator-queue.jsonl | Officina, DARK (`TRIS_MEMORY_AUTO=1` to auto-apply ≥0.85) | ARMED-OFF (SS2b) |
| couplings (`/coupling`, `couplings.example.json`) | Officina hot-swap; future ascensus | CORE |
| **pymander** (libhermes Rust binary) | deprecated — Officina memory replaced recall | RETIRING (notice in launcher; direct access one release) |
| **`vitriol-rag.service`** (:8282 semantic recall) | deprecated — no consumer | RETIRING — owner: `systemctl --user disable --now vitriol-rag mongod-vitriol` |
| **`mongod-vitriol.service`** | pymander mirror only | RETIRING (with rag; data kept one release) |
| **`hermes-gateway.service`** | messaging platforms (Telegram etc.) via hermes-agent | OWNER DECISION: retire if messaging unused; otherwise the one sanctioned hermes runtime |
| repo-map MCP clone | repo-map ext | OPTIONALIZED (SS3): ext off unless `OFFICINA_REPO_MAP_DIR` points at a real checkout |
| little-coder repo | none (mining source; fallback removed SS1) | RETIRED from runtime |
| trismegistus repo | none (private archive) | FROZEN (bfd9a7b final entry) |

## Self-sufficiency gate

`officina/scripts/selfcheck.sh [--live]` asserts this map stays true: no
external project paths in live code, tests green, config present,
keybindings written, provenance headers intact. Run before release tags
and after touching the systems above.
