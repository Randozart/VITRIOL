# Installed optimization lab acceptance test

Created: 2026-11-15T11:15:01+01:00 (Europe/Amsterdam).
Tested source: `35423b22a1195b5fac6cb8465ef13842547646fd`.
Installed binary SHA256: `820dc773e1b803722d249482734989cd90149c0e256a13d94c098c26d4e2b654`.

The user asked to test the new layer and the CPU decision-model proposal. The
installed CLI was tested without issuing inference requests, loading GPU models,
changing routing/configuration or restarting services. The source tree matched
the completed implementation checkpoint before these documentation changes.

## Executed acceptance checks

- All four installed operator example commands (`compare`, `governor`,
  `kernel-check`, `validate`) exited0 with valid JSON.
- Fixed seed `610086` generated60 synthetic workloads of48 requests each,
  each compared across four slot/eviction/dispatch configurations:240 candidate
  reports and11,520 checked request outcomes. Each of60 input files was executed
  twice; corresponding complete JSON output was byte-identical.
- Independent checks compared request arrivals/completions against reported
  starts, per-slot non-overlap, queue durations, token totals, prefill/decode/restore
  accounting, model/layout/rewind compatibility and the maximum prefix available
  from genuinely completed requests. No unavailable future state was reused.
- Reported RAM/disk high-water marks stayed within candidate budgets. Scenarios
  with every entry protected produced zero evictions. Unknown GPU capacity never
  became a feasible-estimate winner. The operator fixture separately excluded a
  two-slot candidate exceeding its smaller GPU's estimate.
- Thirty-six randomized dot/matvec/stable-softmax cases matched independent
  Python `math.fsum`/stable-exponential calculations at absolute/relative1e-10
  tolerance, rather than merely comparing the CPU oracle with itself.
- Negative controls returned the required non-success statuses: wrong numerical
  answer → exit1/failed; malformed response → exit1/failed;20ms timeout →
  exit1/failed with worker timed_out; nonexistent backend → exit2/unavailable.

All assertions passed; no implementation defect was exposed by this bounded run.
The stress script completed in about1.71s on this host. This
is acceptance-run host elapsed time, not a GPU timing estimate or engine speedup.
The preceding implementation evidence separately contains30 Rust lab tests and
seven governor-policy tests; these checks do not turn coverage into a formal proof.

## Synthetic example result

| Candidate | Processed tokens | Predicted prefill microseconds | Supplied capacity estimate |
|---|---:|---:|---|
| baseline |24|240|fits estimate|
| RAM cache |15|150|fits estimate|
| disk tier |11|110|fits estimate|
| two slots |24|240|exceeds smaller GPU estimate|

The fixture selects disk-tier on its supplied p95-latency/makespan objective.
This says nothing measured about actual VITRIOL throughput, whether a second
slot physically fits, or how well the linear model predicts real sessions.
Governor replay records relieve → resize → yield → restore, with zero executed
actions. Numerical `kernel-check` is a CPU-reference protocol check.

## Decision-model and emulator boundary

The default Python has no Numba, PyTorch, Transformers or ONNX Runtime except
NumPy. A separate Python3.12 has PyTorch/Transformers/NumPy but no Laya, AnyJev,
Numba or ONNX. The local ModernBERT directory contains only metadata and an empty
ONNX directory; no Laya weights were found. A local DeBERTa base checkpoint is
not a validated Laya replacement and was not presented as one.

Both PyPI and Hugging Face metadata probes failed with temporary DNS resolution
failure. No decision-model quality/latency benchmark, model download, CUDA-Python
emulation or actual GPU inference was performed. Laya's compact-state
shadow-evaluation proposal remains unqualified. Its next prerequisite is a
compatible CPU runtime plus pinned model weights in a network-enabled environment.

## Evidence and reproducibility

Complete reports, negative fixtures and the fixed-seed external acceptance script
are under `/tmp/vitriol-lab-acceptance`, copied to the private durable directory
`~/.local/state/vitriol-lab/acceptance/2026-11-15`. The script uses the installed
binary and the source fixture from the tested commit. Its SHA256 is
`2597f63d201dc895f0addef314be85385c562383b7fe424112bec45f8132ffc4`.
The script and detailed local capability evidence stay outside reusable source
packages. The test plan is
[lab-acceptance-tests](2026-11-15-111144-lab-acceptance-tests.md); the operator
contract is [optimization-lab](../../docs/optimization-lab.md).
