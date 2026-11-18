# Several sessions in one project: what already holds, what doesn't

**Date:** 2026-11-18 21:00
**Mode:** PLAN (research only — nothing changed)
**Owner:** "Do we need to account better for several sessions running inside of the same project at
once?"

Yes — but *not* where it is usually assumed. Cross-session routing was designed and shipped; the
unguarded part is **capacity** and **file claims**. Measured evidence below.

---

## 1. Live state right now

| fact | value |
|---|---|
| `officina` processes | **4** (I am one of them) |
| peer-routing shards | **3** live (`routing-registry.d/{3911,3997,4237}.json`) — peer arbitration **is active** |
| engine `parallel` | **1** slot, `n_ctx 262144` |
| KV state size | **1.63 GB / 1.74 GB** (two slot snapshots on disk, `turn-110/120`) |
| `cache_ram` | **8192 MB** → holds ≈ 4.7 states of that size |
| re-prefill since boot | **212,522 prompt tokens in 1,014.9 s** (~209 tok/s) |

So: 4 sessions × ~1.7 GB ≈ **6.7 GB against an 8 GB RAM cache** — we are one session (or one big
context) away from evicting a state and paying a **full re-prefill at ~209 tok/s** (≈ 18 min for a
225k context). That is the dominant multi-session cost, and at depth decode is already ~1.2 tok/s.

## 2. What already accounts for multiple sessions (verified, don't rebuild)

- **Peer-aware routing** (P1/P3, 2026-11-10): every session publishes `{ts, tuiPid, sessionId, tier,
  model, ctxTokens, role}` to a per-pid shard; `readLivePeers()` merges shards, TTL 60 s, GC ≥30 s,
  self excluded by pid. `decideTier` demotes the newest/smallest session when `freeSlots == 0`, and
  `OFFICINA_SESSION_ROLE=subagent` ranks spawned children first. Pinned by `overflow-peer.test.ts`.
- **Per-session files**: task lists, scratchpad, telemetry, ledger rows (`sessionId` + `tuiPid`).
- **Engine prompt cache** (RAM → Optane mirror, LRU cap) so a returning session swaps state instead
  of re-prefilling.

## 3. The three real gaps

### A. Capacity — the arithmetic is undocumented and undefended
`states_held ≈ cache_ram / state_size`. Nothing knows N, and nothing warns when `N × state_gb`
approaches `cache_ram`. The failure mode is silent and expensive: the 5th session (or a subagent)
evicts someone, and *that* session's next turn is a multi-minute prefill.

Proposal:
- **A1** raise `cache_ram` 8 → **16 GB** (host has 39 GB; the engine is already `oom_score_adj -500`).
  Measure before/after with `/cache` + `prompt_tokens_total` growth over a fixed 4-session window.
- **A2** surface the arithmetic: `/cache` gains `states ≈ cap/state` and a live `sessions: 4` count
  (the shard count is already there); a one-line warning when `N × state ≈ cap`.
- **A3** verify the demotion fires for real (a live 2-session probe: both `local`, `freeSlots == 0`,
  assert one is demoted with reason naming the rank) — the machinery exists but has only ever been
  unit-tested.
- **A4 record the alternative and reject it**: `parallel = 2` halves each slot to 131k. At 262k
  native context that trades away the thing we bought the hardware for. Keep 1 slot, more RAM.

### B. File claims — two sessions can edit one file with zero signal
Today the only backstop is `git` (an `index.lock` at commit time) and the operator's attention.
For a project that routinely runs 3-4 sessions, that is the largest *correctness* risk after the slot
cost — and it bites silently (one session's edit overwrites the other's intent).

Proposal (advisory only, never blocking — the owner stays in control):
- **B1** promote `_shared/active-files.ts` into a **claim registry**:
  `~/.vitriol/officina/claims/<pid>.json` `{pid, sessionId, paths[], at}`, same shard/TTL/GC pattern
  as the routing registry (which already solves exactly this class of problem). Written on
  `tool_call` for edit/write, TTL ~10 min.
- **B2** on `edit`/`write` of a path claimed by another **live** pid: a one-line warning notice
  ("`tui/src/tui/mod.rs` is open in session 3911"), and the same row in `/claims`. Never a refusal.
- **B3 prerequisite — a genuine bug**: `active-files.ts` is a **module-level** Map with the comment
  "one per process, shared across all extensions". Under pi's jiti isolation that is false: each
  extension gets its own instance, so the tool-result-clearer's protection is unreliable today.
  Make it `globalThis`-backed and register it in `jiti-isolation.test.ts` (the standing rule).

### C. Shared-state hygiene (small, but each is a real cross-session bug)
- **C1** `~/.vitriol/officina/routing-registry.json` (the pre-shard aggregate) still holds **two
  phantom peers from 2026-11-11** (`tuiPid 3469621`, `ctxTokens 50000`). Nothing reads it today
  (readers use shards only) — so delete it and let a test assert shards are the only source.
- **C2** `_shared/compact-on-finish.ts` stamps **one global file**
  (`~/.vitriol/officina/compact-on-finish.json`), not session-keyed: with 4 sessions, one session's
  compaction stamp **silences another's** for the cooldown window. Key it by session id
  (`compact-on-finish/<sessionId>.json`) so the "once per finished run" guarantee stays per session.
- **C3** an inventory test: enumerate `~/.vitriol/officina/*`, classify each file **session-keyed** or
  **last-writer-wins**, and fail when a *new* shared file appears without a classification. New
  cross-session state then becomes a conscious decision instead of an accident.

## 4. Invariants

- **I1 — no collision:** two live sessions never share a state path (test spawns two registry writers
  with different pids and asserts disjoint files).
- **I2 — single-session parity:** with N=1, every decision, notice and stamp is byte-identical to
  today (golden test on `decideTier` and the stamp writer).
- **I3 — claims are advisory:** no tool call ever fails or blocks because of a claim; a claim read
  failure degrades to silence.
- **I4 — stamps are per session:** session A's compaction stamp cannot suppress session B's
  (two-writer test).
- **I5 — capacity is visible:** `/cache` states `states_held` and the warning threshold, and a test
  pins the arithmetic against a synthetic cap.

## 5. Order and cost

| stage | work | risk |
|---|---|---|
| **S0** | C1 (delete the phantom aggregate) + C2 (session-keyed stamp) + C3 (inventory test) | low — pure hygiene, two small files |
| **S1** | A1 (`cache_ram` 16 GB) + A2 (`/cache` states) + A3 (live demotion probe) | low — one config key, one row, one test |
| **S2** | B3 (fix the jiti bug in `active-files`) → B1 (claim registry) → B2 (advisory warning) | medium — touches every edit/write hook |

S0 and S1 are an afternoon and remove three silent cross-session failure modes. S2 is the one that
changes what the operator sees, and it is worth doing *after* the de-spaghetti S0 (M2 band registry),
because the claim warning wants a band row — and that is exactly the registry M2 builds.
