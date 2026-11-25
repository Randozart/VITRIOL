# What actually makes the Kintsugi engine fast — complete inventory (2026-11-26)

Owner framing: "three rodeos — find out what about the Kintsugi version of llama works for it, and
how we get the same or better on vitriol-engine." This note is the complete, evidence-backed answer.
Predecessors: `kintsugi-parity-research-2026-11-26.md` (HEAD2 finding), `gdn-*-2026-11-25/26.md`.

Method: their **launcher** (`/scratch/kintsugi-ref/Kintsugi-27B/start-server.sh`) read as the runtime
manifest, then every feature it names traced into their source; then the remaining unknown files in
the diff inventory (`mirai-s.cu`, `wddm.cu`) identified.

## The answer: their speed is FOUR stacked features, not one

| # | feature | what it does | evidence | in ours |
|---|---|---|---|---|
| 1 | **HEAD2 two-stage output head** (`LLAMA_HEAD2=512`, default ON) | exact Q8_0 head (6144×205k, ~1.2 GB) in pinned RAM; Q4_0 copy in VRAM scores every token; radix-select top-512 per row; exact rows gathered only for those. ~½ the head bandwidth per token, ~606 MiB VRAM freed | `start-server.sh:172,277`; `llama-model-loader.cpp:1326-1337`; `llama-model.cpp:2024-2060`; `ggml-cuda.cu:3013-3060`; `head2.cu` 643 L | **absent** |
| 2 | **GDN raw gates** (fold gate activations into the op) | per-layer: sigmoid+softplus+mul computed inside the GDN kernel over the whole block instead of 4 elementwise ops × 65 layers in the graph | `ggml.h` "fold the per-head gate activations" comment; `gated_delta_net.cu:36-46,196-219`; graph sites `qwen35.cpp:532-542`, `delta-net-base.cpp:404-409` | ported once, garbage output, reverted |
| 3 | **DFlash2 + replay rollback** | DFlash2 draft 7 (we have the drafter); on PARTIAL rejection the next forward reuses the drafter's pending-token inputs from a reserved "pending plane" instead of rewinding the recurrent state and re-forwarding every position — rejected tokens cost ~one fused op, not a re-prefill | `server-context.cpp:4156-4199` (spec_is_replay + checkpoint partial restore); `llama-memory-recurrent.cpp:162,223-225,798-799`; `ggml_gated_delta_net_replay` (`ggml.h:2641`); `delta-net-base.cpp:566-574` | absent — our `seq_rm` (`llama-memory-recurrent.cpp:181`) drops positions and the next decode redoes the work |
| 4 | **Launcher-side sizing** | UB=1024 (vs our 128), `--kq-mask-packed`, `--fit off`, `--kv-vram-cells` (ported, `kvc=`), cache-ram 4096 | `start-server.sh:317,241-352` | partial (tiered KV yes; the other flags missing from arg table) |

Not relevant on this host: `gated-delta-net-mma.cu` (Blackwell-only, verified dead on sm_86),
`head2.cu`'s 2-bit h2 layout (only for GGML_TYPE_COUNT stage-1, we'd use q4_0),
`mirai-s.cu` (Mirai-quantized models only), `wddm.cu` (Windows only), `dspark.cpp` (DSpark arch).

## Why previous rounds missed this

1. **We diffed kernel files, not the runtime manifest.** The launcher names every feature in one
   place; we found GDN by profiling and HEAD2 only last round — because the I7 comparisons ran their
   binary with its launcher defaults, HEAD2 was silently inside every "theirs 14.55 t/s" number.
2. **The gap was measured as one number and assumed to have one cause.** It is at least three
   multiplicative effects (head bandwidth, gate-fold ops, rejection rework).
3. **Feature 3 (replay) is invisible in any kernel diff** — it spans server → memory → graph → op.

## Ordering by expected value (rough, per decode step at c 8192, n_tokens 1)

- Head matmul at batch 1 is the largest single matmul (6144 × 205k). HEAD2 halves its effective read:
  plausibly the largest single win, and it also frees ~606 MiB (single-card DFlash2 blocker headroom).
- Replay: with draft 7 at ~0.8 acceptance, ~1-2 rejections per verify round are the NORM
  (51 accept-handling sites); each rejection today costs us a rewind + re-forward of up to 7 positions.
- GDN raw gates: removes 4 elementwise ops/layer ≈ 260 small kernels per token — real but smaller,
  and it FAILED parity once; must land behind a unit test.
- UB 1024: affects prefill more than decode; cheap to test on their binary first (ablation).

## The plan shape this implies (for PLAN)

- **Stage 0 — ablate their binary** (no code): `LLAMA_HEAD2=0` vs default; UB 1024 vs 256.
  Attributes the 14.55 among the four features in minutes. Everything after is sized by these numbers.
- **Stage 1 — port HEAD2** (bounded: head2.cu/.cuh + 19 hunks in ggml-cuda.cu + loader/model/impl.h;
  no graph change per their comment). Test: two-stage output == exact MUL_MAT output (tolerance for
  the Q4 stage-1 selection), plus golden corpus ≥ 6/8.
- **Stage 2 — port replay rollback** (server + memory + graph + op; the largest surface, but each
  piece has an existing counterpart in our tree to diff against). Test: force a rejection mid-draft
  and require the next forward to produce byte-identical output to a cold re-prefill.
- **Stage 3 — GDN raw gates** (the reconciliation plan stands: op-level unit test first, merge into
  OUR file keeping the chunk kernel). Lands last, gated by Stage 0's numbers.
- **Stage 4 — launcher parity**: kq-mask-packed / fit off / ub 1024 as A/Bs on OUR binary.

Better-than-theirs levers we already own and they don't: tiered KV on one card (`kvc=`), VITRIOL
graph dump, the chunked GDN kernel for n_tokens ∈ {2,3,4,6,8} (their file lacks it — our MTP verify
path can beat their replay on small verify batches).
