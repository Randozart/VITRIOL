# Reference 2 — context window & KV cache

The flags that decide how much conversation fits, how the KV cache is
shaped, and what survives between requests.

PROVENANCE: arg.cpp semantics; REBIS measurements 2026-08-21/22.

## `-c, --ctx-size N`

Context window in tokens. `0` = from model metadata. REBIS: **65536** on both
heads (hermes floor). Overflow behavior depends on `--context-shift`: without
it, requests error; with it, oldest context rolls away silently.

## `--keep N`

Tokens kept from the *initial* prompt when shifting (`-1` = all). Protects
system prompts during shift-trims.

## `--context-shift`

Rolling-window overflow handling. Day-long sessions need this — without it
we hard-error at 64k. Removed once during bleed debugging and restored after
the H1 gate fixed the actual root cause. Lesson: fix causes, don't disable
features.

## `--cache-reuse N`

Minimum chunk size for KV-shift reuse: regions of cached tokens that match
the new prompt get *shifted* to new positions instead of recomputed. 256 is
our value. Requires prompt caching enabled. Pairs with `--context-shift`.

## KV dtype

`-ctk/--cache-type-k`, `-ctv/--cache-type-v` — quantize K/V vectors (q4_0,
q8_0, fp16…). q4_0 + `-fa on` shrinks KV ~4× with negligible code-task loss;
V-quant requires FA. Measured: Sol full 64k KV ≈ 0.9 GiB at q4_0.
Draft-model variants exist (`--spec-draft-type-k/v`).

## `--swa-full` / sliding-window models

Models with SWA (like Luna: 3:1 ratio, window 1024) keep only a sliding KV
window on most layers — that's why her 64k KV is ~0.7 GiB. `--swa-full`
forces all-global attention (huge memory cost for SWA models); leave off.

## VITRIOL semantic prompt cache

| flag | our value | meaning |
|---|---|---|
| `--cache-ram N` | 2048/1024 MiB | host-RAM cap for saved conversation states. Default 8192 contributed to real OOM kills; `-1` unbounded; `0` disables |
| `--slot-prompt-similarity F` | 0.1 | slot dispatch picks best-prefix slot above F. 0 disables affinity (kills reuse but also kills cross-session restore) |
| `--prompt-cache-min-lcp F` | 0.5 (**H1 addition**) | refuse restoring any state below this LCP fraction. Root-cause fix for unrelated prompts inheriting foreign conversations |
| `--slot-save-path DIR` | unset | persist states to disk |

Mechanism worth knowing: with `cache_prompt=true`, each request first saves
the slot's state into the cache, then tries to load the *best-matching*
state. The similarity metric was biased for short prompts
(lcp/tokens_new explodes when tokens_new is small) — hence H1's absolute-gate.

## Engine-owned disk snapshot retirement (opt-in)

Build with CMake `-DVITRIOL_CACHE_STORE=ON`, or set
`VITRIOL_BUILD_CACHE_STORE=1` for `scripts/build-llama-server.sh`. This requires
POSIX, GCC 16 and Cargo with the locked dependencies available offline. The Rust
library and C++26 contracts apply to server host code; CUDA compilation is unchanged.

On that rebuilt engine, `VITRIOL_KV_OWNED_CACHE=1` enables tracking in the private
`owned-kv-v1` subdirectory of the configured disk mirror. Officina's local provider
adds persisted session/branch tags; reload extensions to register them. Tree
navigation creates a new branch frontier. Metadata is stored as a non-message
session entry and never enters the model prompt. `OFFICINA_NO_CACHE_OWNERS=1`
disables this adapter independently of prefill progress reporting.

Tracking defaults to dry-run. `VITRIOL_KV_OWNED_RECLAIM=1` additionally enables
physical deletion after successful completion/publication: at most two eligible
files and at most the managed namespace's byte budget per collection. The byte
budget uses the existing disk-trim multiplier times the RAM cache cap and
includes pending reservations. A full budget skips new managed disk writes;
RAM inference can continue. Legacy and managed namespaces each retain their own
budget, so enabling managed storage can increase the combined disk footprint.

Inspect `/cache` → `managed`: `enabled`, `available`, `store.reclamationEnabled`,
`store.limits`, and `store.audit` distinguish actual mode from conditional
eligibility. The pure audit keeps `deletionAuthorized=false`; it never unlinks
files. Only the synchronized engine owner can do that after fresh identity and
epoch checks. Read leases, active requests, shared owners, explicit pins and two
acknowledged rollback generations per session/branch/model/layout retain files.
Cancelled/failed requests cannot advance retirement. A late writer is fenced
unless an attached borrower still needs its state.

Untagged requests, multi-completion/child tasks, multimodal and LoRA requests
continue on the existing path. Anonymous legacy files are never reclassified as
owned. Storage/ledger faults stop managed effects; they do not authorize deleting
unknown files. See the [runtime contract](../../vitriol-cache-store/README.md)
for POSIX durability assumptions and tests. Activating a rebuilt binary and these
environment variables requires a normal engine restart; merely reloading
Officina does not enable engine reclamation.

## Checkpoints (VITRIOL)

`-ctx-checkpoints N` (per-slot cap, default 32) · `-cpent N` (spacing during
prefill, default 2048). Each checkpoint ≈150 MB host RAM at large ctx.
REBIS: 12 / 8192 — day-long sessions otherwise creep into multi-GB.

## RoPE / YaRN scaling family

`--rope-scaling`, `--rope-scale`, `--rope-freq-base/scale`,
`--yarn-*` — context-extension math applied at load. Leave untouched unless
deliberately extending a model beyond training context; wrong values corrupt
positions subtly rather than failing loudly.

## Deprecated

`--defrag-thold` — removed upstream; defragmentation is automatic now.
