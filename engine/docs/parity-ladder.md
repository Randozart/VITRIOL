# VITRIOL Parity Ladder

Adopted 2026-09-01 from OurobourOS `docs/CONTRACTS.md` (own work) via the
E3 oracle implementation (`.opencode/plans/e3-cb-eval-oracle-2026-09-01.md`).

Every kernel change, backend port, quant-type addition, or merge that
touches arithmetic must re-pass the applicable rungs before its results are
trusted. A rung either PASSES with numbers attached or FAILS with a
reproduction; "should be fine" is not a state.

| rung | claim | tool | gate |
|---|---|---|---|
| L0 | node byte-exact (same backend, before/after) | `llama-vitriol-oracle` captures + `diff.py` | 100% byte-exact |
| L1 | node statistically equal (cross-backend or fp-order change) | same + cos >= 0.999 per node | 100% of matched nodes; quantized-weight models see §calibration |
| L2 | behavioral equality | sampled token / greedy stream equal | token match over probe prompts |
| L3 | placement independence | same run, different `-ts` / device pin | L2 holds + t/s sane |
| L4 | performance contract | llama-bench + fingerprinted argv | no regression beyond noise gate (±2%) vs frozen baseline |
| L5 | energy contract | future: RAPL/nvidia-smi delta sampling | W/token within budget (not yet instrumented) |

## Calibration

- L1 cos >= 0.999 applies to f32/f16 graphs and same-backend A/Bs.
- Quantized-weight cross-backend: per-node cos can legitimately sit at
  0.997-0.9999 (q6_k matmul rounding amplified over depth, see E3 report).
  Gate = L2 (greedy) + review of per-node trend; no structural mismatch
  (node counts align by name, no ONLY-A/ONLY-B on compute nodes).
- CPU is thread-count bit-stable (L0 holds t=1 vs t=4) - use CPU captures as
  the reference oracle.

## Usage recipes

    # capture (same flags both sides except what you are testing)
    ORACLE_OUT=/tmp/o-a ./build-ku2/bin/llama-vitriol-oracle -m model.gguf -ngl 99
    ORACLE_OUT=/tmp/o-b <changed build>/bin/llama-vitriol-oracle -m model.gguf -ngl 99

    # L0/L1 compare + injected-fault sanity
    python3 tools/vitriol-oracle/diff.py /tmp/o-a.idx /tmp/o-a.bin /tmp/o-b.idx /tmp/o-b.bin
    python3 tools/vitriol-oracle/diff.py ... --perturb=A:500:17   # must be flagged

Keep captures out of git (transient, model-derived).
