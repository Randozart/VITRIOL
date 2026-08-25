# Dual-Slot Plan — Lapis Occultus (OOM-safe) + Ontic Reserved Slot

> **Date:** 2026-08-25
> **Trigger:** OOM kill of production llama-server at 17:52:01 (anon-rss 8.4 GiB,
> c=131072 window); user request to split Ontic forge traffic onto a reserved
> tiny slot instead of sharing Hermes' KV space.

## 1. Objective

Two profiles on one server binary:

| profile | purpose | context layout |
|---|---|---|
| `qwen38-master` ("Lapis Occultus") | hermes-agent daily driver, **OOM-safe** | `-np 1`, `c 98304` |
| `qwen38-ontic` | dual-slot: hermes + ontic forge coexist | `-np 2 -kvu --slot-context "0=90112,1=8192"` |

Ontic pins itself to slot 1 via `"id_slot": 1` in its `/completion` body.
Hermes is untouched and keeps landing on slot 0.

## 2. Background facts (verified this session)

- OOM: dmesg `oom-kill task=llama-server pid=1502952 anon-rss=8790872kB`,
  17:52:01, after ~8 min of hermes "memory pressure elevated" warnings.
  c=131072 + q4_0 KV (~2.37 GiB) + weights + buffers exceeded what 16 GiB
  DDR3 tolerates under agent load. Certified safe filled depth remains
  **92,642 tok** within a 98304 window (LAPIS report §5).
- Upstream llama.cpp **PR #23340** (`--slot-context SLOT_CONTEXTS`) provides
  per-slot software caps over a unified KV pool:
  `--slot-context "0=4000"` with `-np 4 -c 10000` → slot0 4k, rest 2k each.
  Requires `--kv-unified` for slots to exceed the equal split.
  **Not present in our fork** (grep confirmed: no `slot_ctx_sizes` anywhere).
- Slot lifecycle: `release()` leaves KV cells resident; only
  `prompt_clear()` erases (`server-context.cpp:147`). No per-request
  cache-clear parameter exists upstream or in fork. With `--slot-context`
  we don't need one: slot 1's cap bounds Ontic's footprint by construction.
- Contamination caveat (upstream issue #27148): RAM prompt-cache +
  idle-slot publishing can leak unrelated conversation content across
  requests, even sequentially. Our launcher does not pass `--cache-ram`;
  defaults may still enable it — **verify empirically after port** and
  consider `--no-cache-idle-slots` if any leakage appears (never
  `--cache-ram 0`: readiness rule, AGENTS.md protocol §2).

## 3. Implementation steps

1. **Port PR #23340** into `llama.cpp` submodule:
   - `common/common.h`: `std::map<int,int32_t> slot_ctx_sizes` in common_params
   - `common/arg.cpp`: `--slot-context` parser (`<id>=<size>` pairs)
   - `tools/server/server-context.cpp`: apply per-slot sizes at slot init;
     warn when a size exceeds physical per-slot KV without unified;
     validate id range / sum ≤ n_ctx / positive sizes.
   Adapt to fork drift where the patch doesn't apply cleanly.
2. **Rebuild** llama-server with `CMAKE_CUDA_ARCHITECTURES="61;86"`.
3. **Profiles** (repo + sync to `~/.vitriol/profiles/`):
   - `qwen38-master/config`: `context = 131072` → `98304` (both top-level
     and `[model]`); meta updated (OOM note, 92k certified depth).
   - NEW `qwen38-ontic/`: copy of master plus `[server] parallel = 2`,
     new key `[server] slot_context = 0=90112,1=8192`.
4. **Launcher** (`scripts/vitriol`): wire `CFG_SLOT_CONTEXT` →
   `SLOT_CTX_ARGS=(--slot-context "$CFG_SLOT_CONTEXT")` in both launch
   paths (dry-run + real), fingerprint line extended with `slots=` field.
5. **Ontic** (`~/Desktop/Projects/ontic/src/forge.rs`): add
   `"id_slot": 1` to the llama-backend JSON body. Cargo build check.
6. **Hermes** (`~/.hermes/config.yaml` line 22): VITRIOL provider
   `context_length: 131072 → 98304`. Gateway restart.
7. **Verification matrix**:
   - master relaunch: fingerprint shows `c=98304`; smoke completion;
     heartbeat > 0.
   - ontic profile launch (test port or stop/start): log shows
     `n_slots = 2`, per-slot ctx 90112/8192; POST `/completion` with
     `id_slot:1` + long prompt (>8k) → capped/rejected at 8k boundary;
     normal short sample routes to slot 1; hermes-shaped chat request
     lands slot 0. `/slots` endpoint reports distinct `n_ctx` per slot.
   - reuse audit baseline re-run.

## 4. Risks / notes

- Fork drift may require manual adaptation of #23340 hunks (file is from
  May; our tree has VITRIOL modifications in server-context.cpp).
- `parallel = 2` halves nothing under unified pool; total KV budget stays
  98304 tokens — Ontic's 8192 cap comes out of the shared pool only while
  its cells are live; they die with the CLI process's requests completing…
  actually cells persist post-release; acceptable because capped at 8k and
  sequential usage means hermes' next prefill simply LCP-truncates around
  them. If pollution observed → add `--no-cache-idle-slots` (NOT
  `--cache-ram 0`).
- MTP/speculative stays off in both profiles (no benefit on this HW).
- Alias "Lapis Occultus" preserved in both profiles so hermes config needs
  no model-name change when switching profiles.

## 5. Rollback

Single-file revert each: profiles are independent; launcher flag is inert
unless `slot_context` key present; server falls back to equal-split
without `--slot-context`. Master profile rollback = restore `context =
131072` line (OOM risk returns — do not).
