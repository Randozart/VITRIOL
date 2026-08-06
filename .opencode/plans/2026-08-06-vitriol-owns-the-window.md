# VITRIOL-Owns-the-Window: ctx-shift drain + selective Hermetis re-inject

Date: 2026-08-06.

## 1. Goal

Stop opencode from compacting. VITRIOL owns and constrains the context window; opencode
is told it is always within budget so its compaction never fires. Drain = server-side
context shifting; required context is selectively re-injected from Hermetis.

## 2. Problem

- The session context is split by `--parallel 2` → per-slot 16384 → opencode compacts at
  ~13K (constant compaction). Root cause of "compacts extremely often."
- Even with 32768, opencode's compaction is opencode-owned and lossy-feeling.

## 3. Architecture

```
opencode (limit.context = 131072 -> compaction threshold never reached)
   |  sends the full growing session each request
   v
VITRIOL server (--ctx 32768 --ctx-shift on --parallel 1 --cache-reuse)
   |  ctx-shift drains the OLDEST turns when the prompt exceeds 32768,
   |  keeps the system prompt + recent turns (incl. injected [Hermetis context])
   v
model sees a rolling ~32K window; everything older is in Hermetis (lossless)
plugin selectively re-injects required context as a recent message (survives the shift)
```

- **Drain** = llama.cpp context shifting (`--ctx-shift on`, default off; verified in the
  fork: server-context.cpp `params_base.ctx_shift`, common.h:544 default false).
- **Never compact** = opencode `limit.context: 131072` (Mellum native) — its compaction
  threshold (~80% of limit) is never reached.
- **Selective re-inject** = Hermetis gates: relevance floor (`min_score`), topic-change
  detection (`is_new_topic` via embedding distance), reduced budget (1500). Injected as a
  recent message so it survives the shift; stale injected blocks drain naturally.

## 4. Changes

1. `scripts/launch_vitriol_full.sh`: `--ctx 32768 --ctx-shift on --parallel 1`
   (keep `--reasoning off`; add `--cache-reuse 256`).
2. `~/.config/opencode/opencode.jsonc`: `limit.context: 32768 -> 131072` (Lapis Occultus).
3. Hermetis selective injection:
   - server `context_block`: return `top_score`, accept `min_score`, compute `is_new_topic`
     (query embedding vs recent episode embeddings via `hermetis/embed`).
   - plugin `injectContext`: skip if `top_score < min_score`; skip if `!is_new_topic`;
     hash dedupe; `COPULA_CONTEXT_BUDGET` default 1500.

## 5. Decisive risk

Whether opencode honors config `limit.context` or `/v1/models` `n_ctx` for its compaction
threshold. If it trusts `/v1/models` (reports 16384/32768), the config trick fails —
fallback is a context-reporting shim (proxy) or re-verify what opencode reads.

## 6. Verify

- `/v1/models` `n_ctx` with parallel=1 = 32768.
- Long session: no compaction fires; model sees the rolling window; injected context
  survives shifts; Hermetis captures rolled-away turns (lossless).

### Acceptance criteria (the open verification — blocked on GPU)

- [ ] `/v1/models` reports `n_ctx` = 32768 (parallel=1 gives the full window to the slot).
- [ ] **Decisive risk resolved**: does opencode honor config `limit.context` (131072) for
      its compaction threshold, or `/v1/models` `n_ctx`? If it trusts `/v1/models`,
      fallback = context-reporting shim (proxy).
- [ ] Long session: no compaction fires.
- [ ] Injected `[Hermetis context]` survives ctx-shift (injected as a recent message).
- [ ] Hermetis captures rolled-away turns (lossless).

## 7. Cross-repo

Plan + code mirrored in bitshaper-ai (canonical) and VITRIOL.

## 8. Status

- Code committed: `62401a9` (launch parallel=1 + context-shift + cache-reuse; Hermetis
  selective context; plugin gates). `opencode.jsonc` limit.context 131072 (user config).
- Verification BLOCKED: GPU held by avatar capture (PID 912348, ~2.1 GB); also needed for
  the reasoning-model swap (`2026-08-06-mellum-thinking-agent-swap.md`).
