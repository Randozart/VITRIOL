# VITRIOL Project - Change Log

**Project:** linux-pipe-module (VITRIOL - Visita Interiora Terrae Rectificando Invenies Occultum Lapidem)

---

## 2026-04-29 - VITRIOL Implementation

### 2026-04-29 18:46 - Kernel Module Built
- **File:** `vitriol.ko` (175K)
- **Action:** Successfully compiled VITRIOL kernel module for Infinite VRAM project
- **Build command:** `brief-compiler c --target linux_kernel vitriol.bv && make`
- **Result:** Kernel module ready for `insmod` (requires sudo)

### 2026-04-29 18:30 - Brief Source Created
- **File:** `vitriol.bv`
- **Action:** Created VITRIOL kernel module in Brief language
- **Features:**
  - `#![target(linux_kernel)]` file-level attribute
  - State declarations for GPU mapping and DMA tracking
  - `rct txn init` for module initialization
  - `rct async txn calc_layer` and `stream_layer` for double-buffered pipeline
  - Contract-based safety (`[pre][post]` conditions)

### 2026-04-29 18:00 - Architecture Document Created
- **File:** `VITRIOL_ARCHITECTURE.md`
- **Content:** Complete architectural blueprint for Infinite VRAM via PCIe P2P DMA
- **Sections:**
  - Alchemical mapping (V.I.T.R.I.O.L. acronym)
  - Core transmutations (Multiplicatio, Mercurial Bridge, Calcinatio, Coagula)
  - Technical implementation gaps for full 400B model support

### 2026-04-29 17:45 - Compiler Checklist Created
- **File:** `BRIEF_COMPILER_CHECKLIST.md`
- **Purpose:** Verify brief-compiler kernel target readiness
- **Status:** All 5 requirements marked ✅ COMPLETE:
  1. No-stdlib mode (`--target linux_kernel`)
  2. Section annotations (`#[c, section(".init.text")]`)
  3. Header injection (auto + `#[c, include(...)]`)
  4. Absolute addressing (`let x @ 0xADDR: Type`)
  5. Async pipelining (`rct async`)

---

## 2026-04-29 - Brief Compiler Bug Fixes

### 2026-04-29 19:25 - C Backend Fixes Applied
- **File:** `brief-compiler/src/backend/c.rs`
- **Bugs Fixed:**
  1. **Duplicate `brief_init`** — Renamed wrapper to `init_wrapper()` (line 162)
  2. **NULL state pointer** — Kernel mode now uses `static State state_instance;` (lines 123-126)
  3. **`find_entry_point` logic** — Now prioritizes transaction named "init" (lines 224-236)
  4. **Makefile circular dependency** — Removed `-objs` line (lines 262-272)
  5. **Missing `MODULE_DESCRIPTION`** — Added to kernel output (line 202)

### 2026-04-29 19:00 - BRIEF.md Updated
- **File:** `/home/randozart/Desktop/Projects/BRIEF.md`
- **Added to "Known Behaviors & Quirks":**
  - Transactions (`txn`/`rct`) REQUIRE semicolon after `};` (visual finality)
  - Only `//` comments supported (by design, not a bug)
  - Change Log updated with all compiler fixes

---

## Build Commands

### Compile Brief to Kernel Module
```bash
brief-compiler c --target linux_kernel vitriol.bv
make
```

### Insert Module (requires sudo)
```bash
sudo insmod vitriol.ko
dmesg | tail -5
```

### Remove Module
```bash
sudo rmmod vitriol
```

---

## Next Steps
1. Add PCIe device discovery (GPU: `10de:1b82` - GTX 1070 Ti)
2. Implement `pci_iomap()` and `pci_p2pdma_map_sg()` FFI bindings
3. Add NVMe streaming logic for 400B model weights
4. Test `sudo insmod vitriol.ko` on physical hardware

---

**Last Updated:** 2026-04-29 19:30
**Author:** Randy - Omo Sanza Lettera
