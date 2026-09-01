# SHELVED SIDE TRACK — Briev-Native Inference Engine + Custom Format

Date: 2026-09-01 (evening discussion)
Status: SHELVED by user decision — "write this down, forget about it, resume
main line." Resume only on explicit user request. Open questions in §6 are
UNANSWERED and must be settled before any work starts.

---

## 1. Origin

During E2c/E-KV-0 work, user asked about EXL2 and TensorRT-LLM. Verdict
recorded in-session: both rejected (Pascal 1070 Ti unsupported by TRT-LLM,
ExL2 second-class on sm_61; qwen35 hybrid arch exists only in the VITRIOL
fork; switching engines abandons the entire VITRIOL stack).

User then asked "what about our own format?" which grew into: **a custom
hyperoptimized weight/container format drawn from safetensors, and an
inference engine written almost fully in Briev** (Projects/briev-lang —
user's compiler project, actively being optimized). User framing: "This
would be our own attempt at AI inference."

## 2. Why the idea is strong (analysis preserved)

- **Weight-format math for Qwen3.8-27B (27.32 B params)** — our TQ formats
  as WEIGHT formats (kernels already exist: vecdotq/mmvq for tq1_0/tq2_0,
  mmq-instances for tq3_*):

  | format | size | fit | theoretical decode* |
  |---|---|---|---|
  | Q3_K_M (today) | 12.86 GB | dual-GPU split, sequential | 12.88 t/s shallow |
  | TQ3_0 weights | 11.95 GB | dual-GPU, roomier | ~+10-15% |
  | TQ2_0 | 7.04 GB | fits the 1070 Ti ALONE | ~24-30 t/s |
  | TQ1_0 | 5.77 GB | 1070 Ti + giant KV | ~30+ t/s |

  *bandwidth-bound: bytes / (256 GB/s x efficiency 0.66-0.81). TQ2_0
  packing verified: block = ggml_half + QK_K/4 = 66 B / 256 = 2.0625 bpw.

- Single-GPU fit eliminates the sequential dual-GPU layer split entirely.
- SPIR-V via Briev = ONE binary on both GPUs (sm_61 + sm_86), kills the
  CUDA 13.3-vs-12.9 dual-toolkit dance permanently.
- VITRIOL stack (TurboQuant, sparse KV, lull, predictor) is not abandoned —
  this is a THIRD project that consumes the same ideas, not an engine swap.
- TQ1_0 at 1.69 bpw is bitnet-class and normally wants training-time
  quantization; TQ2_0 (2.06 bpw) is the plausible-but-unproven band;
  TQ3_0-for-weights (3.5 bpw) is the safe on-ramp. libvitriol surgery
  functions + the lift-plan "census -> rectify -> validate -> bench"
  pipeline were built for exactly this (connects to E10 in the master
  plan, section 8).

## 3. Format design sketch (safetensors core + hardware layer)

From safetensors: 8-byte LE header size + strict JSON header (name ->
dtype/shape/offsets), single mmap, zero-copy, no parser code-exec surface.

Hardware layer (the honest delta vs GGUF — cf. the BMTS "already
equivalent" verdict, which this must beat to justify itself):

| feature | why for OUR box |
|---|---|
| device-sectioned physical layout | GPU0/GPU1/CPU sections per declared split (22,14): two big sequential streams |
| 2 MiB section alignment | cuMemCreate/VMM granularity + DirectIO; one mmap per device section |
| pre-repacked payloads | device-final layouts (u32/128-bit padded rows, TQ block order) - load-time repack disappears (OurobourOS measured 3x on repack alone) |
| native TQ metadata | tq1_0/tq2_0/tq3_* first-class per-tensor |
| fingerprint/provenance block | generator, git SHA, split plan, profile - evidence culture in the artifact |
| per-section checksums | silent-corruption detection safetensors lacks |

Kill-criteria to be written INTO the spec before work starts (BMTS rule:
if it does not beat GGUF on measured load path + qualitative wins, it is
a VERDICT: not maintained).

## 4. Program ladder (each rung ships a measured artifact)

Doctrine (from briev-lang, locked there): .abv = pure GPU (SPIR-V/Vulkan),
.bv = CPU lane with verified offload. VITRIOL evidence rules adopted.

| phase | artifact | gate |
|---|---|---|
| B0 | format spec (placeholder name token; name deferred until "emergent properties", per user) | spec review + kill-criteria |
| B0.5 | SPIR-V-on-Pascal verification (1070 Ti proprietary driver + emitted SPIR-V version) | day-one go/no-go |
| B1 | packer + reader in Briev .bv (mmap, JSON header parse) | round-trip byte-exact vs GGUF source |
| B2 | .abv kernels: M1 GEMV -> M3 quant-dequant-dot (feeds existing vitriol-gemm-comparison ledger, docs/plans/2026-08-31-vitriol-gemm-comparison.md) | M1 vs VITRIOL mmvq on both GPUs |
| B3 | end-to-end greedy decode, one tiny model (proposed: bitnet-2b-tq1_0 - ternary = simplest kernels, own format family, ~1.7 GB) | greedy token equality vs VITRIOL, identical prompt+seed |
| B4 | KV cache + softmax attention + sampler in .abv | depth decode at 27B-context shapes on proxy |
| B5 | scale 9B -> 27B, multi-GPU Vulkan split | certified table vs VITRIOL on same box |

Strategic note: B2/B3 are the forcing function for briev-lang's kernel
surface growth (foreach in kernels etc.) - the engine IS the compiler
roadmap. Engine scope grows as the compiler grows; the ledger protects
from sunk cost at every rung.

## 5. Risks (stated, preserved)

1. Briev compiler gaps are the critical path (M1 GEMV blocked on foreach
   in kernel surface as of 2026-08-31 handoff).
2. Non-Briev surface must be defined (proposal: tokenizer = Python shim
   initially, shrink over rungs; everything else Briev).
3. SPIR-V-on-Pascal compatibility = B0.5 gate.
4. Multi-session arc, not a side quest; "small side track" was actually
   "an inference engine" - hence this document.

## 6. Open questions (user must answer before unshelving)

1. Where does it live? (new sibling repo vs briev-lang/examples vs
   VITRIOL formats/) - my instinct was new sibling repo.
2. First end-to-end target: bitnet-2b-tq1_0 (proposed) or tinyllama-class?
3. Confirm non-Briev surface = tokenizer shim only?
4. Format + repo naming: deferred by user ("figure out as we become aware
   of emergent properties") - placeholder token needed in spec regardless.

## 7. Cross-references

- briev-lang: docs/HANDOFF-2026-08-31-gpu.md (GPU backend state, doctrine)
- briev-lang: docs/plans/2026-08-31-vitriol-gemm-comparison.md (M0-M4
  ladder + ledger, targets VITRIOL's certified numbers on this box)
- briev-lang: docs/plans/2026-08-31-abv-gpu-by-default.md (route fixes)
- VITRIOL: mining-experiment-master-plan-2026-09-01.md section 8 (E10 =
  TQ-for-weights ladder, E8 KV-PQ, E9 MLA) - E10a/E10b are the VITRIOL-side
  mirror of the weight-format idea and DO NOT require this engine.
- VITRIOL main line at shelving time: tq3_0 type-registration port bug
  (COUNT/type_traits/whitelists) restored but server init crash unresolved;
  E-KV-0 pending on it; E2c mmvq audit mid-flight; H1c blocked (no shared
  MoE model; user has Models/ on an unmounted external drive).
