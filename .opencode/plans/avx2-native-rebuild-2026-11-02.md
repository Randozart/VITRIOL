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

## Execution results (2026-11-02 20:00–20:13 CET, partial)

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

### Engine-level A/B (INCOMPLETE)

New build loaded the 262K profile and reached `/health` ok with the same
fingerprint. The first bench warmup OOM'd on GPU0 (`cudaMalloc failed: out of
memory`, 1311 MiB compute buffer) because **Firefox started at 20:05 and took
~1.5 GB of GPU0 VRAM**; only 115 MiB was free. This is desktop VRAM pressure,
not a build regression (the 19:38 baseline ran without Firefox).

Owner paused the session at 20:13. State left:
- `engine/build` reverted to the known-good `build-ku-cu12` (old build).
- `vitriol-server` **inactive**; autosave + governor remain active.
- No engine process; GPU0 2.3 GB desktop, GPU1 35 MiB.
- Config/fingerprint untouched (262K profile).

### Next session

1. Close GPU-heavy desktop apps (Firefox/Pathfinder + Discord/Code) or accept
   the 208k profile.
2. Repeat engine-level A/B at 262K, then adopt/revert per the decision gate.
3. Record final numbers here and commit the report.

## Rollback

`engine/build -> build-ku-cu12` remains valid at all times; repoint and
restart. No config or fingerprint changes are part of this work.

## Risks

- Full C++ recompile (~10–20 min); ccache misses on changed ISA.
- nvcc 12.9 + g++-14 host pairing is already proven by the current cache.
- `-march=native` binary SIGILLs on pre-AVX2 hosts — do not copy it there.
