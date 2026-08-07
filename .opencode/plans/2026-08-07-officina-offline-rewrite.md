# Officina — P3b offline GGUF rewrite (DISSOLVE/COAGULATE)

Date: 2026-08-07.

## 1. Goal

Make weight surgery real: `DISSOLVE` (prune by mask) and `COAGULATE` (fold a
normalizer into adjacent weights) write a **new `.gguf`** on disk, then `TEST`
launches a server on it and verifies logit parity vs baseline. This is the
LARQL payoff — weights as an editable database, edits as composable ops.

## 2. Rewrite mechanism (size-preserving)

The GGUF layout is fixed: `[header + metadata + tensor-info] [tensor payloads]`
with payloads at recorded offsets. **Key constraint**: keep every payload the
same byte size, so all offsets stay valid and the header needs no re-serialization.

- Copy the source file to the target byte-for-byte.
- For each edit, **overwrite the payload in place at its offset** with a
  same-size re-encoded payload (masked values re-encoded in the same quant
  format = same block size).

Masking never changes size: zeroing f16/f32 elements keeps the format; zeroing
quantized values keeps the fixed block layout. Coagulate (fold norm scale into
f16 weights) also preserves size.

## 3. Module: `libvitriol/src/rewrite.rs`

- `plan(path) -> Result<RewritePlan>`: parse header → `header_end`, tensor
  offset/size table (reuses gguf.rs `TensorEntry`).
- `copy_and_edit(src, dst, edits) -> Result<u64>`: byte-copy src→dst, then
  write each same-size `edit` (tensor index, replacement bytes) at its offset.
  Returns bytes written.
- `mask_f16(src, ratio, rng) -> Vec<u8>` / `mask_f32(...)`: zero a random
  `ratio` fraction of elements (same-size).
- Quantized masking: decode→zero→re-encode per format (iq4_nl / iq2_s) is a
  follow-up; for now quantized tensors are byte-copied and reported untouched.

## 4. Officina wiring (P3b-3)

- `DISSOLVE > layer.N.mlp magnitude 0.35` — **probe** (exists) shows the impact
  table; **`COMMIT as "name" >`** copies the model, masks matching tensors, and
  writes `<name>.gguf` (or `overwrite` targets the active path). Journals the
  op.
- `TEST > "prompt"` then runs against the rewritten file (a flag selects the
  rewritten model), and the probe reports logit drift vs baseline from a
  reference run.

## 5. Parity

Verify the rewritten model loads in the server and the output is coherent. Logit
drift measured by running the same prompt on base vs rewritten and diffing
logits at a print boundary (tolerance, never `==`).

## 6. Tests

`plan`/`copy_and_edit` roundtrip byte-identical; in-place edit changes only the
target tensor; `mask_f16` zeroes exactly `ratio`; a tiny synthetic gguf (built
in-test) survives an edit and re-reads.

## 7. Provenance

`docs/provenance/officina-rewrite.md`: weight-mutation via size-preserving GGUF
rewrite — standard GGUF spec (public format) re-implemented; no third-party code.

## 8. Results

- **P3b-1/P3b-2/P3b-3 landed**: `libvitriol/src/rewrite.rs` — `plan()`
  (header/tensor-index parse), `copy_and_edit()` (byte-copy + same-size in-place
  payload edits), `mask_f16`/`mask_f32` (exact unique-index zeroing, SplitMix64
  seeded). `DISSOLVE > layer.N[.group] magnitude <ratio>` now probes the impact
  table and commits via `COMMIT as "name" >` → writes
  `~/.vitriol/rewrites/<name>.gguf` (f16/f32 tensors masked; quantized
  iq2_s/iq4_nl byte-copied, reported). Header stays byte-identical; rewritten
  file is same-size. 9 libvitriol + 123 tui tests green, clippy/fmt/praetor
  clean (also fixed pre-existing libvitriol clippy nits: clamp, div_ceil,
  OR→range).
- Remaining: quantized-block masking (iq2_s/iq4_nl decode→zero→re-encode),
  COAGULATE norm-fold, R4 (`DISSOLVE > model <mask>` drop-dross), live
  logit-parity (server restart on the rewritten file).

