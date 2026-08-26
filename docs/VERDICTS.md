# VERDICTS — tombstones of abandoned ideas

> Every line below was built, measured, and killed *by this project*, with
> the reason attached. A fork accumulates hacks; VITRIOL accumulates verdicts.
> Raw lab notes: `.opencode/plans/`, `docs/archive/`.

---

### NVMe→VRAM DMA / copy engine / kernel module (May 2026)
**Hypothesis**: bypass page cache and staging copies with a kernel module
(`vitriol.ko`), BAR1 windows, cooperative `nvidia_p2p`; stream experts
directly from SSD/RAM.
**Measurement**: DDR3 + narrow PCIe made streamed expert fetches starve both
GPUs; for models that *fit* VRAM, resident execution beat every streaming
configuration.
**Verdict**: ❌ superseded by the **residency rule** (`VITRIOL_MODE=off`
default). The buffer-type infrastructure survives in the tree; the streaming
data path does not. Artifacts: `docs/archive/{COOPERATIVE_DMA,COPY_ENGINE_PLAN,
EMULATED_MEMORY_ARCHITECTURE}.md`.

### RAM Shot (35B era)
**Hypothesis**: page-locked host memory can serve MoE expert weights that do
not fit a single GPU.
**Measurement**: worked for Qwen3.5-35B-class on one GPU; rendered obsolete
for current workloads by adding a second GPU (ts splits) where combined VRAM
covers the model.
**Verdict**: ⚰️ not wrong — outlived by hardware. Correct answer for its era;
would be the answer again on a single-8GiB box. See residency rule for when
streaming strategies are (not) worth it.

### MTP speculative decoding
**Hypothesis**: embedded MTP draft head accelerates decode.
**Measurement**: full 5×5 sweep (pin × draft-n): all configs 9.6–9.98 t/s —
zero measurable end-to-end benefit on this hardware; draft-n ≥ 2 regresses
(acceptance decay, ~8 ms/chained-draft cost).
**Verdict**: ❌ omit unless A/B-ing. Keep `n_max = 1` when enabled.

### tq3_0 KV everywhere
**Hypothesis**: 3.5 bpw TurboQuant KV is strictly better than q4_0.
**Measurement**: −40% decode penalty in non-stream mode (missing MMQ kernels)
at some configs; at depth with IQ3_S it certified fine (96,836 tok @ 11.32).
**Verdict**: ⚠️ profile-dependent. Production speed profiles use q4_0 KV;
master deep-context profile uses tq3_0. Measure per profile, don't assume.

### Protecting the server via OOMScoreAdjust
**Hypothesis**: `OOMScoreAdjust=-500` in the user unit pushes llama-server to
the back of the oom-killer queue.
**Measurement**: systemd `--user` has no `CAP_SYS_RESOURCE`; negative values
are silently dropped (server ran at adj=100 regardless). Same-uid processes
*can raise* each other's score, never lower.
**Verdict**: ❌ inverted into the **oom-shield**: raise big consumers to +300,
let the kernel eat browsers first.

### Trusting idle-slot emptiness
**Hypothesis**: an idle slot with 0 tokens is safe to re-checkpoint.
**Measurement**: `--cache-idle-slots` clears occupied slots into host RAM;
the next autosave tick overwrote a 1.26 GB warm checkpoint with a 1 KiB stub
(2026-08-26), leaving nothing for the next crash-restart.
**Verdict**: ❌ empty saves now stage through `slot{N}.tmp.bin` and never
replace a rich checkpoint.

### id_task-based autosave churn detection
**Hypothesis**: skip saves when a slot's `id_task` is unchanged.
**Measurement**: restored-idle slots carry no `id_task` field at all, so the
guard never engaged for exactly the case it was built for.
**Verdict**: ❌ replaced by global `/metrics` counter signature (frozen
counters ⇒ nothing happened anywhere ⇒ skip).

### LRU tie-break `<=` in slot selection
**Hypothesis**: upstream's `<=` tie-break is harmless.
**Measurement**: restored slots keep `t_last_used == -1`; ties then always
picked the *last* slot — every unpinned prompt landed on ontic's 8k window
and died with `exceed_context_size_error` while the 73k slot idled.
**Verdict**: ❌ capacity-fit skip + strict `<` (lowest slot id wins).
Submodule `441ccd871`.

### Forbidden zeros
`--ctx-checkpoints 0` → heap corruption. `--cache-ram 0` → no readiness.
Not tuned values: *do not pass them*.

### systemd trivia that cost real debugging time
- `Wants=` under `[Install]` is silently ignored — relationships belong in
  `[Unit]`.
- `set -u` + `$INVOCATION_ID` unbound → launcher died mid-stop; `${VAR:-}` is
  mandatory in that script.
- bare `clear` under `errexit` without TERM killed `serve` under systemd.

### 262K context + MTP
Does not fit (Pascal compute buffers). Drop MTP for max-context profiles.
