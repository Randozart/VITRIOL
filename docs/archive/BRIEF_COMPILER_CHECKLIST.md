# VITRIOL Compiler Target: Checklist for Brief

To implement the VITRIOL "Short Path" using Brief, the compiler must support these specific features for Ring 0 (Kernel) targets.

## 1. Native C/LLVM Backend
- [ ] **No-Standard Library Mode:** Can the compiler generate C code that doesn't rely on `libc` (e.g., no `printf`, `malloc`, `stdlib.h`)? Kernel modules must use `printk` and `kmalloc`.
- [ ] **Section Annotations:** Does Brief support `@section` or similar attributes? We need to mark functions for `.init.text` and `.exit.text`.
- [ ] **Header Injection:** Can Brief include external headers like `<linux/module.h>`?

## 2. Spatial Isomorphism (Memory Mapping)
- [ ] **Absolute Addressing:** Can Brief define registers at specific physical addresses (BARs)? 
    - *Example:* `let GPU_BAR: UInt64 @ 0xFB000000;`
- [ ] **Volatile Access:** Does the compiler ensure that reads/writes to these addresses aren't optimized away by the LLVM/C compiler?

## 3. Contract-Based Safety (The "No-Panic" Guarantee)
- [ ] **Bounds Provenance:** Can Brief prove that a DMA transfer size `N` never exceeds the mapped BAR size `M`?
- [ ] **FFI Safety:** If we call `pci_p2pdma_map_sg` from Brief, can we wrap it in a contract that prevents null pointers from entering the kernel?

## 4. Concurrency (Spatial Logic)
- [ ] **Async/Parallel Pipelining:** Can Brief describe the "Double Buffering" logic (Streaming Layer N while calculating Layer N-1) without a thread-scheduler? (This must be hardware-native logic).

## 5. Build Pipeline
- [ ] **Kernel-Object (.ko) Target:** Is there a way to tell the Brief CLI to use the Linux Kbuild system instead of a standard linker?

---
**Status:** Use this to audit `https://github.com/Randozart/brief-lang`. If these are missing, the "C-Lijm" (C-Glue) will have to be written manually while Brief handles the high-level Transformer math.
