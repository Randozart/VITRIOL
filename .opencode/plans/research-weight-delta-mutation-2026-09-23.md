# Research: Weight-Delta Paging, Mutation Kernels, and Magic-Number Expert Derivation

Date: 2026-09-23. Status: falsified hypotheses documented; 2 testable ideas remain.

PROVENANCE: analysis and measurements performed against VITRIOL's own store
(`/dev/nvme0n1p3`, Qwen3.8-Flash-Next-UD-Q3_K_XL, 336 expert tensors).
Paper references below are cited for the *concepts discussed*; nothing was
copied into code.

## 1. Goal (restated precisely)

Reduce bytes crossing the PCIe bus (host/Optane -> GPU, ~13 GB/s) while
producing the **same observable result** (identical sampled tokens under a
fixed RNG). NOT storage compression - blowing up Optane usage is acceptable.
This is a bus-bandwidth problem, not a disk-space problem.

## 2. Hardware asymmetry (the real motivation)

| path | bandwidth | role |
|---|---|---|
| PCIe 3.0 x16 (3060) | ~13 GB/s | the slow straw |
| GDDR6 VRAM (3060) | ~360 GB/s | the fast firehose |
| SM compute | ~13-25 TFLOPS | effectively free |

Streaming a 0.9-1.7 MB expert per cold miss across PCIe is the bottleneck.
Mutating resident VRAM in place, or deriving weights from a tiny recipe, would
shift the cost onto the 360 GB/s internal bus - if the data allowed it.

## 3. Measured ground truth (from the live store, 2026-09-23)

All measurements: byte-level comparisons of expert slices (IQ3/IQ4 quantized,
921600 B for ffn_down, 627200-870400 B for ffn_up/gate), read directly from
the raw store zone. "identical" = same byte at same index.

| structure tested | result | conclusion |
|---|---|---|
| element-wise identity (exp0 vs exp1-4, ffn_down) | 1.0-1.3% identical | delta ~= full expert |
| element-wise (ffn_up, ffn_gate) | 0.7-0.9% | same |
| high-nibble coarse match (top ~6 bits) | 9-11% | no coarse redundancy either |
| shift/offset symmetry (E1[i] == E0[i+s]) | best s: 0.98% (floor 0.4%) | no superstring overlap |
| longest identical run | 2 bytes | no patch/dedup granularity |
| 512 B block sharing | 0.00% | no block dedup / codebook |
| cross-layer (same expert idx, layer N vs N+1) | 0.4-1.1% | no shared base across layers |
| global XOR constant | best C=0x00: 1.0% | no single XOR key |
| global multiply mod 256 | best M=129: 1.15% | no multiplier |
| per-256B-block XOR keys | 7 distinct, non-transferring | no position key generalizes |
| per-position XOR key across pairs | 3.0% shared | same |

**Verdict: expert weights are near-information-theoretically independent.**
The delta between any two experts is ~99% of the expert. Shannon entropy of a
target expert ~= its size. No lossless code can transmit E1 "given E0 + tiny
key" cheaper than transmitting E1 directly.

## 4. Hypotheses falsified (with why)

### 4.1 Delta paging / mutation kernel (send a delta, mutate resident state)
Requires delta(E0->E1) to be small. Measured: delta ~= full expert (1%
overlap). Dead for this model.

### 4.2 Overlapping superstring / De Bruijn ribbon
Requires experts to share substrings/blocks. Measured: 0% block sharing, 2-byte
max run. De Bruijn guarantees every length-n string exists, but the expert is
not composed of a small recurring motif set. Dead.

### 4.3 Hashed/coordinate mapping (offset = hash(state, expert))
Works only if the hashed space is densely populated by real experts. It is a
collision-resolution scheme, not a compression scheme. Does not reduce bus
bytes for a cold expert. Not applicable.

### 4.4 LCG / affine procedural generator (seed, a, c -> weights)
An LCG generates pseudo-random *numbers*, not trained weight *structure*. The
magic-number trick works because 1/sqrt(x) is a smooth function; expert weights
are arbitrary data. An LCG cannot reproduce arbitrary learned matrices. Dead.

### 4.5 Rank-1 outer product / base + low-rank delta (IA3-style)
Requires W_k = B * m_k for a common resident B (or W_k ~= U_k V_k^T small).
Measured: cross-layer identity 0.4-1.1% - there is NO shared base to scale.
IA3 works for *fine-tuning one frozen base*; it does not imply trained MoE
experts are expressible as base x scale. Dead.

### 4.6 XOR / multiply symmetry (one or two transformations)
Measured exhaustively (XOR constant, multiply, per-block keys). Best = 1.15%,
at the random floor. Dead.

### 4.7 "Blow up the Optane side" as a general trick
A larger lookup table is a codebook. Exact reproduction requires sending
H(E) bits; the codebook index for a near-random 921 KB expert needs ~921 KB
(the table would exceed the universe). Storage size does not change the
entropy of the transfer. Dead for *exact* transfer.

## 5. Ideas worth attempting (the survivors)

### 5.1 SVD / subspace rank probe (the last exact-generator test)
The one structural property never measured: do experts lie in a shared
low-dimensional subspace? If effective rank k is small, a shared resident
basis B (n x k, loaded once, zero steady-state bus cost) + per-expert
coefficients c_k (k values across the bus) + GPU rebuild W_k = B c_k in VRAM
is a *legitimate* exact scheme. Blowing up Optane (store B redundantly) does
not change k - rank is a property of the weights.
- Method: dequantize ffn_down experts (ggml dequant, C++ probe), SVD, report
  effective rank / energy curve.
- Prediction: full rank (trained experts are diverse) - would falsify the
  exact scheme. Run anyway; it is decisive and cheap.

### 5.2 Output-equivalence A/B (the Unreal-style lossy test)
The real contract is "same sampled tokens", not "same weight bytes". The
engine samples deterministically given a seed. So: can a cheaper
approximation (IQ1 hard-quantize, rank-k truncation, or a magic-constant
perturbation) produce identical tokens?

Why MoE gives this a real chance: routing averages top-2/3 expert outputs,
an ensemble denoiser. Perturbing one expert shifts its output, but the
average with 1-2 others shrinks the net change. The router only needs
relative expert ordering, far more robust than absolute GEMM values. Unreal's
fast inverse sqrt (0x5f3759df) shipped precisely because it accepted lossy
approximation where the downstream was tolerant.
- Method: perturb a subset of experts (IQ1 / k=32 SVD truncation), run the
  model at a fixed seed, diff tokens vs the exact run. Measure how much loss
  the ensemble absorbs before tokens diverge.
- If tokens survive aggressive loss: the real scheme - store full weights on
  Optane, transfer only the cheap-derivation recipe, GPU rebuilds lossy-but-
  output-identical experts. Bus bytes drop by the loss factor.

### 5.3 Bus-traffic counter (measure the goal directly)
Independent of the above: add a store->GPU bytes-per-token counter (via
/metrics or the store read path) so the ladder reports actual PCIe traffic
at depth. Prediction: residency + LRU already keeps hot-expert bus bytes low;
the number will show whether we are at the floor.

## 6. Paper / concept references (for the ideas discussed; none used in code)

- Schmidhuber, J. (1992). "Learning to control fast-weight memories: An
  alternative to dynamic architectures." Fast-weight programming - router
  emits dynamic synaptic updates. (concept anchor for mutation kernels)
- Liu, H. et al. (2022). "Few-Shot Parameter-Efficient Fine-Tuning is Better
  and Cheaper than In-Context Learning" (IA3 - Infused Adapter by Inhibiting
  and Amplifying Inner Activations). Scaling-vector adapters over a frozen
  base. (concept anchor for multiplier/offset; NOT evidence experts are
  base-scaled)
- Hu, E. et al. (2021). "LoRA: Low-Rank Adaptation of Large Language Models."
  Low-rank delta updates. (concept anchor for rank-reduced deltas)
- Chen, W. et al. (2015). "HashedNets: Scaling Deep Learning to Billion-scale
  Neural Networks." Hash-based weight sharing. (coordinate-mapping concept)
- De Bruijn, N.G. (1946). De Bruijn sequences - every length-n string over
  alphabet k appears once. (superstring/overlap concept)
- Hull, T.E. & Dobell, A.R. (1962). "Random Number Generators." Hull-Dobell
  theorem for LCG maximal period. (LCG concept)
- Lomont, C. (2003). "Fast Inverse Square Root" (0x5f3759df analysis).
  The "magic number, lossy by design" trick - Quake III / id, often
  misattributed to Unreal. (lossy-approximation concept)
- Shannon, C.E. (1948). "A Mathematical Theory of Communication." Entropy
  bound: exact transfer needs H(E) bits. (the wall we hit)

Provenance note: these are concept references for documentation and future
research. No third-party code was incorporated. Any future implementation
must carry its own PROVENANCE header per VITRIOL convention.

## 7. Test plan priority (build mode)

1. Unblock arm A (LRU eviction fix a8e44bece is committed, needs re-test).
2. Run the depth ladder with the bus-traffic counter (measures the real goal).
3. Run the SVD rank probe (settles the exact-generator question).
4. If rank is full (predicted), run the output-equivalence A/B (IQ1 / rank-k
   truncation, fixed-seed token diff) - the plausible real win.

## 8. Status

- 2026-09-23: all lossless-delta/mutation/generator hypotheses measured and
  falsified on live store data. Two testable ideas remain (SVD rank probe,
  output-equivalence A/B). Bus-traffic counter planned for the ladder.

## 9. Addendum: Predictive push + Cap'n Proto on-disk format (2026-09-23)

Proposal: predictively push expert data ahead of the compute wave (io_uring
fixed-buffer ring, lookahead scheduler), and store experts in a Cap'n Proto
on-disk arena so the disk layout == in-memory execution layout (zero-copy).

Assessment against the actual codebase:

| claim | reality |
|---|---|
| predictive push before consumer needs it | ALREADY BUILT: vitriol_predictor_prefetch (cross-layer + temporal union, prefetch_ahead=2, async_prefetch=true) at integration.cpp:300/352/647 |
| io_uring fixed-buffer push ring | ALREADY BUILT: store uses io_uring + O_DIRECT (vitriol-store.cpp:173); read_async path exists |
| NVMe -> VRAM P2P DMA | NOT POSSIBLE on this host: no DAX, no NVMe peer-DMA to GPU. Path is Optane -> pinned host RAM -> H2D copy (LRU cuMemcpyHtoDAsync). |
| Cap'n Proto zero-copy (disk == memory layout) | ALREADY TRUE without capnp: our store format IS the raw byte layout (super + fixed-slot descriptor table, O(1) base + slot*stride). No serialization/parse/allocation to eliminate. capnp would be an indirection regression. |
| tiny multiplier/IA3 push payload | FALSIFIED: delta ~= 99% of expert (section 3). A 2KB multiplier cannot reproduce a 921KB expert to identical output. |

Correct kernel: predictive push is right and is the existing design - the bus
traffic is hidden behind compute (overlap), which is the correct attack on
PCIe latency (not shrinking bytes - entropy-bound - but overlapping with work).

Cap'n Proto: NOT needed. Our raw fixed-layout store already achieves
zero-copy. Adding capnp would add an indirection layer for a problem we don't
have.

Actionable follow-up (build mode): wire store_read_expert_async into
vitriol_predictor_prefetch so the store-backed async read feeds the prefetch
slot - completes the "predictive push from Optane" loop. The ladder bus-
traffic counter measures whether prefetch hides PCIe latency (the real
deliverable).

## 10. Addendum: Wayland restart (2026-09-23)

Black screens after heavy VRAM churn (desktop shares the GPU we were
hammering; kwin/plasmashell stayed resident through repeated server crashes).
Driver clean (615.71.09), no GPU faults in dmesg - compositor-side issue.

Restart options:
- A (non-destructive): kwin_wayland --replace & - new compositor, apps survive.
- B (full session): loginctl terminate-session 1 - autologin restores in ~10s,
  kills GUI apps (terminal sessions survive).
- Reboot is the reliable reset if B fails; also clears lingering nvidia state.