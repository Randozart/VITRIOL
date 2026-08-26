# VITRIOL Probe — Hardware Discovery & Pipeline Generator

## Overview

`vitriol probe` scans the PCIe bus, classifies every device into a capability category, and optionally generates a Vial (`.alkavl`) + Recipe (`.alka`) for targeting a specific inference or DMA task.

The user never needs to manually write a Vial. They select hardware, and VITRIOL builds the pipeline.

## Philosophy

Alka's Vial describes *physical affordances*, not firmware protocols. Probe discovers what exists on the bus; it does not know how to compute on it. The split:

| Component | Role |
|-----------|------|
| **Probe** | "There is a GPU at 0000:02:00.0 with 2GB VRAM and a 256MB BAR1 window" |
| **Vial** | Formal encoding of the above into `.alkavl` syntax |
| **VITRIOL** | Knows this GPU is Maxwell gen; uses BAR0+0x1000 for PUSH_BUFFER |
| **Recipe** | SHIFT/FLOW/FENCE sequence tailored to the BAR1 window size |

## Usage

### Basic discovery

```bash
vitriol probe
```

Scans all PCI devices and prints a human-readable table.

### Output to file

```bash
vitriol probe --output /tmp/hardware.json
```

### Select devices for pipeline

```bash
vitriol probe --select 0000:02:00.0,0000:03:00.0 --output /tmp/vitriol_rig.alkavl
```

Generates a Vial file containing only the selected devices.

### Full auto-pipeline

```bash
vitriol probe \
  --select 0000:02:00.0,0000:03:00.0 \
  --recipe inference_pipeline \
  --model model.gguf
```

Generates:
- `inference_pipeline.alkavl` — Vial for selected GPU + NVMe
- `inference_pipeline_base.alka` — Base tensors recipe
- `inference_pipeline_full.alka` — All tensors recipe

## Output: Human-readable

```
VITRIOL Probe — 2026-05-13 18:30:00
====================================
PCIe bus scan complete: 12 devices found, 3 usable

=== GPU (2 found) ===
  0000:01:00.0  GTX 1070 Ti     driver=nvidia     VRAM=8GB   BAR1=256MB  PCIe 3.0 x16
  0000:02:00.0  GTX 960         driver=nvidia     VRAM=2GB   BAR1=256MB  PCIe 3.0 x16

=== Storage (1 found) ===
  0000:03:00.0  NVMe SSD 1TB    driver=nvme       Size=1TB   DMA=yes

=== Other (9 skipped) ===
  USB controllers, audio, SATA, bridge chips

Selected: 0000:02:00.0, 0000:03:00.0
Vial generated: /tmp/vitriol_rig.alkavl
```

## Device Classification

Every PCI device is sorted into one of these categories based on its class code:

| Category | PCI Class | Examples | Probe collects |
|----------|-----------|----------|----------------|
| **GPU** | 0x03 (VGA/Display) | GTX 960, GTX 1070 Ti, RTX 4090 | VRAM size, BAR0/BAR1 base+size, thermal sensors, current driver, PCIe link speed/width |
| **Storage** | 0x01 (Mass storage) | NVMe SSD, SATA AHCI | Capacity, DMA capability, block device path, namespace count |
| **Network** | 0x02 | Ethernet, Wi-Fi | MAC, link speed, DMA ring support |
| **USB** | 0x0C (Serial bus) | xHCI controller | Number of ports, USB generation |
| **Bridge** | 0x06 | PCIe bridges, switches | Topology info (upstream/downstream ports) |
| **Other** | Everything else | Audio, SMBus, LPC | Skips unless `--all` is passed |

### GPU-specific probing

For GPUs, probe reads:

```
/sys/bus/pci/devices/0000:02:00.0/resource             → BAR0, BAR1 base addresses
/sys/bus/pci/devices/0000:02:00.0/resource2_multiple    → BAR1 size
/sys/bus/pci/devices/0000:02:00.0/driver                → Current driver
/sys/class/drm/card*/device/vendor                      → Vendor ID
/sys/class/drm/card*/device/device                      → Device ID
```

VRAM size is determined from the BAR1 aperture:
- If `resizable BAR` is active, read the actual BAR size from `resource2_multiple`
- Otherwise, use GPU model table lookup (since BAR1 window is fixed at 256MB regardless of total VRAM)

Example model table:

| Device ID | Model | VRAM | Architecture |
|-----------|-------|------|-------------|
| 0x1401 | GTX 960 | 2GB | Maxwell 2.0 |
| 0x1b82 | GTX 1070 Ti | 8GB | Pascal |
| 0x1b06 | GTX 1080 Ti | 11GB | Pascal |
| 0x2484 | RTX 4070 | 12GB | Ada Lovelace |
| 0x2684 | RTX 4090 | 24GB | Ada Lovelace |

### Storage-specific probing

```
/sys/block/nvme0n1/size          → Capacity
/sys/block/nvme0n1/queue/dma     → DMA support
```

## Vial Generation (`--output`)

When `--output` is a `.alkavl` path, probe generates a formal Vial:

```alkavl
Vessel GPU_960 {
    PCI_ID: 10de:1401;

    Aperture DATA_PLANE {
        BAR: 1;
        MAX_WINDOW: 256MB;
        TYPE: Prefetchable;
    }

    Aperture CTRL_PLANE {
        BAR: 0;
        SIZE: 16MB;
    }

    Thermal SENSOR_0 {
        HALT_AT: 85C;
        THROTTLE_AT: 80C;
    }

    Memory VRAM {
        TOTAL: 2GB;
        RESERVED: 256MB;
    }
}

Vessel NVME_BOOT {
    BLOCK_DEVICE: /dev/nvme0n1;
    DMA_CAPABLE: true;
}
```

Each selected device becomes a Vessel. Device names are auto-generated from model names (e.g., `GPU_960`, `NVME_BOOT`, `SSD_SECONDARY`). The user can pass `--rename` to override:

```bash
vitriol probe --select 0000:02:00.0 --rename 0000:02:00.0=DRAFT_GPU
```

## Recipe Generation (`--recipe`)

When `--recipe` is provided along with `--model`, probe generates Alka source recipes for the selected GPU + storage device, exactly as the existing `generate-alka-recipe.sh` does — but with correct window sizes auto-detected from the hardware.

The generated recipe uses the correct `MAX_WINDOW` from the Vial:

```alka
CLAIM GPU_960;
LIMIT GPU_960 THERMAL 85C;
SLICE NVME_BOOT 0x0 <model_size> 256MB;  // Auto-chunked based on BAR1 window
```

## Integration with Executor

The generated Vial + Recipe feeds directly into the existing pipeline:

```bash
vitriol probe --select 0000:02:00.0,0000:03:00.0 \
              --recipe pipeline \
              --model model.gguf

alka build pipeline.alka pipeline.alkavl    # Compile
alka-executor pipeline.alka.alkas pipeline.alkavl \
  --bind 0000:02:00.0 \
  --source model.gguf                        # Execute
```

## Implementation Plan

### Phase 1: Device scan + human output
- Single C file: `vitriol-probe.c`
- Reads `/sys/bus/pci/devices/*/class`, matches to categories
- Reads vendor/device/name from PCI ID tables
- Prints formatted table

### Phase 2: Vial generation
- Extends probe to emit `.alkavl` from `--output`
- Maps device properties to Vial syntax
- Handles multiple devices, naming, affordance inference

### Phase 3: Recipe generation
- Integrates with existing `gguf-offset-resolver` and `generate-alka-recipe.sh`
- Auto-detects window sizes from probed BAR1
- Generates SHIFT/FLOW/FENCE sequences

### Phase 4: Full automation (`vitriol run`)
- Single command: `vitriol run --gpu 0000:02:00.0 --model model.gguf`
- Runs probe → generate Vial → generate Recipe → compile → bind → execute → collect results

## Relation to Existing Tools

| Existing | Role in new pipeline |
|----------|---------------------|
| `gguf-offset-resolver` | Phase 3 — tensor offset extraction |
| `generate-alka-recipe.sh` | Phase 3 — recipe generation |
| `alka` (compiler) | Unchanged — compiles `.alka` → `.alkas` |
| `alka-executor` | Phase 4 — execution with `--bind` and `--source` |
| `benchmark_alka.sh` | Phase 4 — automated benchmark runs |
