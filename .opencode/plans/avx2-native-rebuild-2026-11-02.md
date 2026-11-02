# AVX2 native rebuild — plan (2026-11-02)

## Context

The PC upgrade replaced the CPU (i7-3770 → **Xeon E5-1620 v4**, Broadwell-EP)
while the engine binary was built 2026-10-23 with `-march=native` on the old
CPU. Disassembly of the installed `libggml-cpu.so.0.23.0`:

- AVX/F16C instructions present (`vpmaddubsw`, `vcvtph2ps`)
- AVX2/FMA instructions absent (`vpbroadcastd`, `vpblendd`, `vperm2i128`,
  `vgather*`, `vfmadd*` all = 0)

So the CPU backend still runs pre-AVX2 kernels even though the CPU now
supports AVX2 + FMA + BMI2 (no AVX-512).

The daily driver is GPU-resident (`-ngl 99`), so decode is expected to be
largely unchanged. Wins are expected on CPU-side paths: quant matmul / repack
/ llamafile SGEMM, state save/restore, tokenization/sampling, CPU-only
inference, embeddings, and `--n-cpu-moe`.

## Build trap

`-march=native` is an unchanged flag string, so an in-place reconfigure will
NOT recompile existing objects. Build into a fresh directory instead of
forcing a destructive clean.

## Decisions (owner-approved)

- Target: rebuild engine CPU backend native, A/B only; no experiment detour.
- ISA: `-march=native` (binary tied to AVX2-class CPUs; not portable to the
  old i7-3770).
- Location: new build dir, A/B then swap. Working build stays untouched.

## Paths

| item | path |
|---|---|
| current build | `/scratch/builds/build-ku-cu12` |
| new build | `/scratch/builds/build-ku-cu12-native` |
| symlink | `engine/build -> build-ku-cu12` (to be repointed only on a win) |

## Steps

1. **Baseline capture (read-only).**
   - `llama-bench` CPU-only: `-m qwen2.5-0.5b-instruct-q4_k_m.gguf -ngl 0
     -t 4 -p 512 -n 128 -r 3`
   - record current engine shallow bench (post-upgrade: median 29.0 t/s) and
     slot restore (39,312 tok / 615 ms)
   - capture old `system_info` feature line.

2. **Configure** the new build dir with the same cache settings as the
   current build:

   ```
   cmake -S engine -B /scratch/builds/build-ku-cu12-native \
     -DCMAKE_BUILD_TYPE=Release \
     -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
     -DGGML_CUDA=ON \
     -DCMAKE_CUDA_ARCHITECTURES=86 \
     -DCMAKE_CUDA_COMPILER=$HOME/toolkits/cuda-12.9/bin/nvcc \
     -DCMAKE_CUDA_HOST_COMPILER=/usr/bin/g++-14 \
     -DGGML_NATIVE=ON \
     -DGGML_VITRIOL_MKFS=ON \
     -DVITRIOL_RUST_LIB=$PWD/vitriol-stream/target/release/libvitriol_stream.a \
     -DVITRIOL_URING_LIB=/usr/lib/liburing.so
   ```

   `sm_61` dropped: both GPUs are sm_86. NCCL/FA/graphs/compression defaults
   match the existing cache.

3. **Build:** `cmake --build ... -j8 --target llama-server llama-bench`

4. **Verify ISA:** disassemble new `libggml-cpu.so` for AVX2/FMA mnemonics;
   `llama-bench` must report `AVX2 = 1`, `FMA = 1`, `F16C = 1`.

5. **A/B:**
   - CPU-only `llama-bench`, old vs new (primary signal)
   - if CPU gain is real: engine-level A/B (stop unit, temporarily point
     `engine/build` at the new dir, start, shallow bench + slot restore,
     restore symlink immediately)

6. **Decision gate:** adopt only if CPU paths improve and engine decode/VRAM/
   fingerprint do not regress. Adopt = repoint symlink, restart via unit,
   re-verify health + fingerprint + slot restore.

7. **Record:** report with A/B numbers, cmake cache, portability caveat;
   commit the report only (build artifacts stay untracked).

## Execution results (2026-11-02, completed)

### Build

`/scratch/builds/build-ku-cu12-native` configured (CUDA 12.9.86, host g++ 14.3.1,
`CMAKE_CUDA_ARCHITECTURES=86`, `GGML_NATIVE=ON`, NCCL not found same as before)
and built successfully (`llama-server`, `llama-bench`). Symlink shim created:
`engine/build-ku-cu12-native -> /scratch/builds/build-ku-cu12-native`.

### ISA verification

`libggml-cpu.so` disassembly, old vs new:

| mnemonic | old | new |
|---|---:|---:|
| `vpbroadcastd` | 0 | 283 |
| `vpblendd` | 0 | 156 |
| `vperm2i128` | 0 | 192 |
| `vgatherdps` | 0 | 44 |
| `vfmadd231ps` | 0 | 411 |

Runtime `ggml_cpu_has_*`: old `{avx:1, avx2:0, fma:0, f16c:1, bmi2:0}`;
new `{avx:1, avx2:1, fma:1, f16c:1, bmi2:1}`.

### CPU-only A/B (`llama-bench -dev none`, qwen2.5-0.5b Q4_K_M, `-t 4`)

| build | pp512 | tg128 |
|---|---|---|
| old (i7-3770 native) | 58.50 ± 0.64 | 18.59 ± 1.16 |
| new (Xeon native) | **101.75 ± 1.74** | **33.31 ± 0.69** |

≈ +74% prefill, +79% token-gen on the CPU backend.

### Engine-level A/B (completed 22:38–22:40 CET)

First attempt at 20:05 OOM'd on GPU0 (`cudaMalloc failed: out of memory`, 1311 MiB
compute buffer) because Firefox took ~1.5 GB of GPU0 VRAM; only 115 MiB was
free. That was desktop pressure, not a build regression. After the owner closed
Pathfinder/Firefox (GPU0 free: 11.3 GiB), the A/B was repeated cleanly:

| build | bench shallow median | runs | prompt t/s |
|---|---|---|---|
| old `build-ku-cu12` | 27.387 | 25.909 / 30.147 / 27.387 | 24.0–24.5 |
| new `build-ku-cu12-native` | 27.338 | 27.338 / 28.380 / 30.923 | 24.8–25.4 |

Decode is statistically identical (GPU-bound, as expected). New build VRAM is
slightly lower or equal (GPU0 10,820 vs 10,834 MiB after bench). Fingerprint
identical; slot restore 17 tokens / 119 ms; no OOM.

**Decision: adopt the native build.**
- `engine/build -> build-ku-cu12-native`
- engine restarted via the unit, `/health` ok, fingerprint unchanged
- old build remains as instantaneous rollback at
  `/scratch/builds/build-ku-cu12`

### Follow-up

- Re-bless `fingerprint.blessed` at leisure (still 2026-09-17).
- Firefox/Pathfinder need ~1.5 GB GPU0; 262K + MTP n=3 fits only with that
  closed (or switch to `qwen38-buffedmod-208k`).

## Rollback

`engine/build -> build-ku-cu12` remains valid at all times; repoint and
restart. No config or fingerprint changes are part of this work.

## Risks

- Full C++ recompile (~10–20 min); ccache misses on changed ISA.
- nvcc 12.9 + g++-14 host pairing is already proven by the current cache.
- `-march=native` binary SIGILLs on pre-AVX2 hosts — do not copy it there.
