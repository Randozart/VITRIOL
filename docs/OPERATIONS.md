# VITRIOL Operations — the self-healing runtime

> How VITRIOL stays up on a 16 GiB host that is permanently red-lined.
> Implementation: `scripts/lull_slot_persist.py`, `systemd/user/*.service`,
> `scripts/vitriol`.

## Unit topology

```
vitriol-server.service          vitriol-autosave.service
  Restart=always (5 s)            Restart=always (10 s)
  Wants= ──────────────────────▶ started with server
  ◀────────────────────────────── PartOf= stopped with server
```

- `ExecStartPre` runs `scripts/vitriol stop` (kills stale processes; skipped
  from systemctl path when invoked *by* the unit via `INVOCATION_ID` guard).
- `vitriol stop` outside the unit goes through `systemctl --user stop` —
  sticky, `Restart=always` does not resurrect explicit stops.
- The sidecar (`lull_slot_persist.py`) polls every 5 s and owns everything
  below.

## Failure-mode → response

| # | failure | detection | response | bound |
|---|---|---|---|---|
| 1 | server exits (OOM kill, crash) | PID disappears | systemd restarts; sidecar detects generation change, replays `slot{N}.bin` | ~40 s |
| 2 | server hangs (health-deaf, same PID) | `/health` fails ×12 consecutive polls | sidecar forces `systemctl --user restart` | ~90 s |
| 3 | host memory exhaustion building | `MemAvailable` < `VITRIOL_PROACTIVE_MB` for N polls | **proactive bounce**: pre-bounce checkpoint save + clean restart, then cooldown | ~2 min detect, no data loss |
| 4 | swap-thrash stall in progress | `/slots` listing slow/times out | save tick aborted (`SERVER SLOW`), never pile writes onto a struggling server | immediate |
| 5 | operator wants it down | `vitriol stop` | systemctl-based sticky stop of both units | stays down |

## Checkpoint integrity rules

1. **Churn guard** — skip a tick entirely when
   `llamacpp:{prompt,predicted}_tokens_total` are unchanged since the last
   successful pass. A 48k-token slot checkpoints at ~1 GiB; rewriting it
   while nothing happened was measured as swap-thrash fuel.
2. **Clobber guard** — saves stage through `slot{N}.tmp.bin`; an empty result
   (`n_saved == 0`) never replaces an existing checkpoint > 10 MiB.
   Rationale: `--cache-idle-slots` clears occupied slots into host RAM,
   making them *look* empty. Observed loss: 1.26 GB checkpoint overwritten
   by a 1 KiB stub (2026-08-26 07:00), leaving nothing to restore after the
   next crash.
3. **Never delete on refusal** — an HTTPError from the save endpoint keeps
   any existing file: stale-but-warm beats gone.
4. **Restore replay** — on every new server instance, before serving traffic:
   `POST /slots/{id}?action=restore`. Warm GDN state ≈ 150 MiB base + KV;
   replay of a 63k-token slot takes ~5 s.

## OOM economics (why the shield works backwards)

- systemd `--user` has no `CAP_SYS_RESOURCE`: negative `OOMScoreAdjust` is
  silently clamped. You cannot protect the server directly.
- Same-uid processes *can raise* each other's `oom_score_adj` (ptrace-mode
  write). Therefore the shield raises big non-essential consumers
  (browsers, opencode) to +300 so the kernel's oom-killer eats them first.
- Protected names: `llama-server`, `hermes`, `lull_slot_persist`,
  `vitriol-tui`, `systemd`, `python3`; extendable via
  `VITRIOL_OOM_PROTECT_EXTRA="name1,name2"`.

## Knobs (all env, all optional)

| variable | default | meaning |
|---|---|---|
| `VITRIOL_PORT` | 8279 | server port |
| `VITRIOL_AUTOSAVE_SECS` | 300 | autosave interval |
| `VITRIOL_SLOT_SAVE_DIR` | `~/.vitriol/checkpoints` | checkpoint dir |
| `VITRIOL_HANG_STRIKES` | 12 | health-deaf polls before forced restart (0 = off) |
| `VITRIOL_PROACTIVE_MB` | 250 | MemAvailable floor for proactive bounce (0 = off) |
| `VITRIOL_PROACTIVE_TICKS` | 24 | sustained-low polls before bounce (~2 min) |
| `VITRIOL_BOUNCE_COOLDOWN_SECS` | 600 | min seconds between bounces |
| `VITRIOL_OOM_SHIELD_ADJ` | 300 | oom_score_adj applied to shielded consumers (0 = off) |
| `VITRIOL_OOM_SHIELD_MB` | 300 | min RSS MiB to be considered a shield target |
| `VITRIOL_OOM_PROTECT_EXTRA` | — | comma-separated extra protected process names |

Server-side memory knobs that matter here:

- `--cache-ram N` — DRAM prompt-cache cap in MiB. **Default entitlement is
  8192 MiB**, which on a shared 16 GiB host fills zram and wedges the box.
  Production profiles pin `1024`.
- Never pass `--ctx-checkpoints 0` (heap corruption) or `--cache-ram 0`
  (no readiness).

## Daily-driver commands

```bash
trismegistus            # fish function: ontic profile → restart → wait → hermes
vitriol stop            # sticky stop (no auto-restart)
vitriol serve --detach  # manual start (systemd wraps this)
journalctl --user -u vitriol-autosave.service -f     # watch the watchdogs
```

## Pymander semantic recall stack (2026-08-26)

Cross-domain semantic lookup over the curated doctrine, built as
augmentation — every layer degrades to silence, never to caller errors.

```
docs/pymander/*.md (canonical)  ──pymander_sync.py──▶  mongod :27018
        ▲                                               │ vectors
        │ markdown stays source of truth                ▼
copula.ts ◀──/search (local cosine, bge-small) ── vitriol_rag.py :8282
```

| unit | standing RAM | caps |
|---|---|---|
| `mongod-vitriol.service` | ~200 MiB | cache 0.25G, `MemoryMax=300M`, port **27018** + private socket dir |
| `vitriol-rag.service` | ~195 MiB | fastembed bge-small int8 lazy-loaded, `MemoryMax=400M` |

Degradation ladder (all verified live):
- mongo down, warm cache → serves stale, retries in 30 s
- mongo down, cold cache → empty results; ascensus proceeds without context
- rag service down → copula.ts primary path unchanged (Hermetis domain search)

Ops notes:
- A *system* mongod (`mongodb:mongodb`, port 27017) exists on this host,
  enabled+failed — it died unnoticed 2026-08-25 night. Recommended cleanup:
  `sudo systemctl disable --now mongod`. Our instance deliberately uses
  27018 so the two never contend.
- Sync: `~/.vitriol/venvs/rag/bin/python scripts/pymander_sync.py`
  (`--dry-run`, `--full`). Hash-gated: only changed nodes re-embed.
  Or `POST /sync` on :8282.

## Known anomaly (watch-list)

One episode (2026-08-26 ~07:43) of the server task queue jamming:
`/metrics` and `/health` instant, `/slots` and admin tasks timing out;
resolved by server restart, not yet reproduced. If it recurs: capture
llama-server logs *before* restarting.
