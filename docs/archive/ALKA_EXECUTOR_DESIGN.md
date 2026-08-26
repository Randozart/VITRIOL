# VITRIOL Alka Executor — Design & Usage

## Overview

The Alka Executor bridges compiled Alka streams (`.alkas` files) to the VITRIOL kernel module, enabling hardware-level DMA orchestration for MoE expert streaming.

```
Alka Source (.alka) + Vial (.alkavl)
         │
         ▼
  ┌──────────────┐
  │   Alka       │  Compile → .alkas (Drop packets) + .azoth (rollback)
  │  Compiler    │
  └──────┬───────┘
         │
         ▼
  ┌──────────────┐
  │   Executor   │  Validate Drops against Vial → IOCTL → /dev/vitriol
  │  (userspace) │
  └──────┬───────┘
         │
         ▼
  ┌──────────────┐
  │   vitriol.ko │  Execute: CLAIM, SHIFT, FLOW, FENCE, SIGNAL, etc.
  │  (kernel)    │
  └──────┬───────┘
         │
         ▼
  GPU Hardware (PCIe BAR1, DMA, VRAM)
```

## Components

### 1. Kernel Module (`vitriol-daemon/vitriol.c`)

**Version 0.2** — Adds Alka ABI support (0xA1 magic) alongside legacy 'V' magic.

#### New IOCTLs (0xA1 magic)

| IOCTL | Direction | Purpose |
|-------|-----------|---------|
| `VITRIOL_IOC_SET_VIAL` | Write | Load vial constraints (aperture, thermal, DMA) |
| `VITRIOL_IOC_EXECUTE` | Write | Execute a single Drop packet |
| `VITRIOL_IOC_VALIDATE` | Read/Write | Validate Drop without executing |
| `VITRIOL_IOC_GET_RESULT` | Read | Get last execution result |
| `VITRIOL_IOC_STREAM` | Write | Execute entire stream with auto-rollback |

#### Supported Opcodes

| Opcode | Name | Kernel Handler | Status |
|--------|------|----------------|--------|
| 0x01 | CLAIM | `handle_claim()` | ✅ Implemented |
| 0x03 | FLOW | `handle_flow()` | ✅ Staged (DMA buffer) |
| 0x04 | SHIFT | `handle_shift()` | ✅ Implemented |
| 0x05 | FENCE | `handle_fence()` | ✅ Polling |
| 0x06 | SYNC | `handle_sync()` | ✅ `wmb()` |
| 0x07 | SENSE | `handle_sense()` | ✅ Stub |
| 0x09 | SIGNAL | `handle_signal()` | ✅ Stub |
| 0x0E | LIMIT | `handle_limit()` | ✅ Implemented |
| 0x2F | WATCH | `handle_watch()` | ✅ Stub |
| 0x2C | DRY_RUN | `handle_dry_run()` | ✅ No-op |
| 0x3B | REFRACT | `handle_refract()` | ✅ Stub |

#### Rollback

The kernel maintains a rollback stack (max 64 entries). On stream failure, `.azoth` packets are executed in reverse order to undo state changes.

### 2. Userspace Executor (`alka-executor/executor.c`)

#### Features

- **Vial parser**: Reads `.alkavl` files and extracts vessel constraints
- **CRC validation**: Alka ROL-XOR CRC32 (matches compiler)
- **Drop validation**: Per-opcode checks against vial limits
- **Dry-run mode**: Validate without executing (`--dry-run`)
- **Rollback support**: Execute `.azoth` packets on failure (`--rollback`)

#### Usage

```bash
# Dry-run validation (no kernel module needed)
./alka-executor/alka-executor stream.alkas vial.alkavl --dry-run

# Full execution (requires /dev/vitriol)
sudo ./alka-executor/alka-executor stream.alkas vial.alkavl

# With rollback file
sudo ./alka-executor/alka-executor stream.alkas vial.alkavl --rollback stream.azoth

# Verbose output
./alka-executor/alka-executor stream.alkas vial.alkavl --dry-run --verbose
```

#### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Stream executed successfully |
| 1 | Validation failure or I/O error |

### 3. ABI Header

Three versions exist, all with identical struct layout:

| File | Audience | Types |
|------|----------|-------|
| `alka-handoff/vitriol_alka.h` | Alka compiler (Zig) | Zig `packed struct` |
| `vitriol-daemon/vitriol_alka_kernel.h` | Kernel module | `__u8`, `__u64`, etc. |
| `alka-executor/vitriol_alka_user.h` | Userspace executor | `uint8_t`, `uint64_t`, etc. |

#### Drop Packet (32 bytes)

```
┌─────────┬───────┬───────────┬──────────────────┬──────────────────┬─────────┬──────────┬─────────┐
│ op_code │ flags │ vessel_id │    src_addr      │    dst_addr      │  size   │ reserved │   crc   │
│  1 byte │1 byte │  2 bytes  │    8 bytes       │    8 bytes       │ 4 bytes │ 4 bytes  │ 4 bytes │
└─────────┴───────┴───────────┴──────────────────┴──────────────────┴─────────┴──────────┴─────────┘
```

#### CRC Algorithm

Alka uses a simple ROL-XOR CRC (not standard CRC32):

```c
uint32_t crc = 0;
for (size_t i = 0; i < crc_offset; i++) {
    crc = (crc << 1) | (crc >> 31);  /* Rotate left 1 */
    crc ^= bytes[i];
}
```

## Vial Contract

The `.alkavl` file defines hardware constraints that every Drop must respect:

```
Vessel GPU_MAIN {
    PCI_ID: 10de:1b82;
    Aperture DATA_PLANE {
        BAR: 1;
        SIZE: 256MB;
    }
    Thermal SENSOR_0 {
        HALT_AT: 85C;
        THROTTLE_AT: 80C;
    }
    Memory VRAM {
        TOTAL: 8GB;
        RESERVED: 256MB;
    }
}
```

### Validation Rules

| Opcode | Rule |
|--------|------|
| CLAIM | Vessel must exist in vial |
| FLOW | Size ≤ aperture_max, vessel must be DMA-capable |
| SHIFT | Offset + aperture_size ≤ BAR1 size |
| FENCE | No validation (metapage check at runtime) |
| SIGNAL | Signal ID must be non-zero |
| LIMIT | Thermal ≤ halt temperature |
| REFRACT | Total range ≤ BAR1 size |

## Stream Patterns

### GTX 960 Setup (7 packets)

```
CLAIM → LIMIT → REFRACT → SYNC → FENCE → WATCH → SIGNAL
```

Purpose: Initialize GPU, set thermal limits, configure sub-tensor slicing, monitor state.

### NVMe→GPU DMA (17 packets)

```
CLAIM×2 → LIMIT → [SHIFT → FLOW → FENCE]×4 → SYNC → SIGNAL
```

Purpose: Load 896MB of model weights through 256MB BAR1 aperture using sliding window.

```
Window 0: SHIFT @ 0     → FLOW 256MB → FENCE (metapage=1)
Window 1: SHIFT @ 256MB → FLOW 256MB → FENCE (metapage=2)
Window 2: SHIFT @ 512MB → FLOW 256MB → FENCE (metapage=3)
Window 3: SHIFT @ 768MB → FLOW 144MB → FENCE (metapage=4)
```

## Building

### Kernel Module

```bash
cd vitriol-daemon
make clean && make
sudo insmod vitriol.ko
dmesg | tail
```

### Executor

```bash
cd alka-executor
make clean && make
make test    # Dry-run both streams
```

## Current Limitations

1. **FLOW is staged**: Uses DMA buffer as staging area, not direct NVMe→GPU DMA
2. **FENCE polls**: Busy-waits on BAR0 register, no interrupt-driven completion
3. **SIGNAL is stub**: Doesn't trigger actual GPU kernel launch
4. **Vessel IDs**: Alka compiler encodes PCI ID in src_addr, not vessel_id field
5. **Placeholder offsets**: Compiled streams have zero src/dst for FLOW/SHIFT — real GGUF offsets need to be filled in at runtime

## Next Steps

1. **GGUF offset resolver**: Parse GGUF header to compute real expert tensor offsets
2. **Stream generator**: Generate `.alka` source from model file, compile to `.alkas`
3. **Direct NVMe DMA**: Replace staged FLOW with `blkdev_direct_read()` → GPU BAR1
4. **Interrupt-driven FENCE**: Use NVMe completion queue interrupts instead of polling
5. **GPU kernel launch**: Implement SIGNAL to submit work to GPU command ring
