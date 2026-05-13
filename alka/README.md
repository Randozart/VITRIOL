# VITRIOL + Alka Integration

> *"VITRIOL = Engine. Alka = ECU."*

## Directory Structure

```
alka/
  vials/
    vitriol_rig.alkavl          # Hardware topology: GTX 1070 Ti + GTX 960 + NVMe
  recipes/
    benchmark_35b.alka          # Benchmark workflow with WATCH/TRACE profiling
    moore_stream.alka           # Expert streaming via NVMe->GPU DMA
    speculative_decoding.alka   # Dual-GPU speculative decoding
  scripts/
    run_benchmark.sh            # Runs llama.cpp benchmark + Alka mock
    run_mock.sh                 # Runs Alka mock executor only
  results/                      # Generated outputs (.alkas, .azoth, logs)
```

## Workflow

### 1. Compile a Recipe

```bash
alka recipes/benchmark_35b.alka vials/vitriol_rig.alkavl
```

Produces:
- `benchmark_35b.alka.alkas` — Metrod packets (executable)
- `benchmark_35b.alka.azoth` — Rollback binary (antidote)

### 2. Mock Execution (no kernel/hardware)

```bash
alka --mock recipes/benchmark_35b.alka.alkas
```

Simulates DMA transfers, thermal monitoring, BAR window shifts, and FENCE polling against a mock 8GB GPU.

### 3. Run Full Benchmark (llama.cpp + Alka)

```bash
./scripts/run_benchmark.sh
```

Runs both:
- Alka recipe compilation + mock execution
- llama.cpp inference benchmark (35B MoE vs 9B baseline)

### 4. Kernel Execution (when Athanor is ready)

```bash
alka --safe recipes/moore_stream.alka.alkas recipes/moore_stream.alka.azoth
```

Executes via `/dev/vitriol` with Azoth rollback safety.

## Recipes

### benchmark_35b.alka

Instruments the baseline inference loop with `WATCH` (throughput monitoring) and `TRACE` (execution profiling). Describes the expert streaming pattern:

```
CLAIM GPU_1070TI -> FLOW NVMe->VRAM -> FENCE -> SIGNAL -> WATCH -> TRACE
```

### moore_stream.alka

The target architecture: stream only active MoE experts from NVMe to GPU via DMA. Uses `PIPE` for continuous ring buffer operation and `SHIFT`/`FLOW`/`FENCE` triplets for BAR1 sliding window management.

### speculative_decoding.alka

Dual-GPU coordination: GTX 960 runs a 0.5B draft model, GTX 1070 Ti verifies via P2P DMA. Uses `SPECULATE` instruction and `PIPE` between GPUs.

## Integration Phases

| Phase | Status | Description |
|-------|--------|-------------|
| 1. Recipe authoring | Done | Alka recipes describe the benchmark workflow |
| 2. Mock execution | Done | Recipes validated via mock executor |
| 3. llama.cpp benchmark | Done | Actual tok/s measured with `-ot` flag |
| 4. Kernel module testing | Pending | Load `vitriol.ko`, test BAR mapping |
| 5. Alka -> Athanor | Pending | Wire compiled Metrod packets to kernel IOCTLs |
| 6. DMA streaming | Pending | Replace `cudaMemcpyAsync` with `FLOW` |
| 7. Speculative decoding | Pending | Wire `SPECULATE` between GPUs |

## Vial Constraints

The `vitriol_rig.alkavl` encodes the physical limits:

| Constraint | Value | Impact |
|------------|-------|--------|
| BAR1 MAX_WINDOW | 256MB | All transfers >256MB require SHIFT loop |
| GPU_1070TI VRAM | 8GB (775MB reserved) | 7.2GB available for experts |
| GPU_960 VRAM | 2GB (256MB reserved) | Only fits draft models (<1.7GB) |
| Thermal halt | 85C (1070Ti), 98C (960) | LIMIT enforces hard abort |
| PCIe Gen | 3.0 x16 | ~12 GB/s theoretical DMA bandwidth |
