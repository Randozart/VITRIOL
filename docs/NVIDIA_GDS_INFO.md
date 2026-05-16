# NVIDIA GPUDirect Storage (GDS) Information for VITRIOL Project

## Key Insights from System Reminder

### 1. Source Code Location
- Repository: [NVIDIA/gds-nvidia-fs](https://github.com/NVIDIA/gds-nvidia-fs)
- Critical file: `src/nvfs-core.c`
- This contains the kernel-side implementation of GPUDirect Storage

### 2. Completion Handling Mechanism
NVIDIA uses a two-stage completion ritual:

**Stage A: Kernel Callback (`nvfs_io_complete`)**
- Hooks into Linux Asynchronous I/O (AIO) system
- When NVMe finishes DMA burst, triggers hardware interrupt
- Kernel calls: `static void nvfs_io_complete(struct kiocb *kiocb, long res)`
- Uses `kiocb` completion callback instead of custom interrupt handler

**Stage B: Shared "Metapage"**
- 4KB shared memory page between kernel and AI app (llama.cpp)
- Kernel writes fence value: `mpage_ptr->end_fence_val = nvfsio->end_fence_value;`
- AI app watches memory address for changes
- Bypasses context switch latency - app sees truth instantly

### 3. Implementation Plan for VITRIOL

**Step 1: Update sentinel.bv**
```brief
let dma_fence_addr: UInt = 0; // Physical address of shared metapage
let dma_complete: Bool = false;

// Sentinel: Watch the Fence
rct txn watch_fence [dma_active == true] [dma_complete == true] {
    let current_fence = read_phys(dma_fence_addr);
    [current_fence == 1] {
        &dma_complete = true;
        &dma_active = false;
        printk("VITRIOL: Moore Stream Complete. Weights are settled.\n");
    };
    term;
};
```

**Step 2: llama.cpp Surgery (ggml-cuda.cu)**
```cpp
// Instead of cudaMemcpy, use VITRIOL DMA
vitriol_trigger_dma(tensor_offset, file_offset);

// Wait for Metapage Fence (fastest path)
while (*vitriol_metapage_fence == 0) {
    __builtin_ia32_pause(); 
}
```

### 4. GPU Compatibility "Heist"
- GTX 1070 Ti (Pascal, device ID 1b82) is physically capable
- NVIDIA GDS officially supports "Tesla or Quadro class GPUs" 
- Difference is often just a PCIe config space bit check
- Can spoof identity to bypass enterprise-only restriction

### 5. Critical Implementation Details
- **Memory Barriers**: Need `wmb()` (write memory barrier) before triggering DMA
- **Sliding Window**: Handle segmentation for layers > 256MB BAR window
- **BAR 0 Dashboard Registers**: Must poke correct registers to prepare GPU for reception

### 6. Next Steps
1. Clone and examine NVIDIA GDS source
2. Implement device ID spoofing for GTX 1070 Ti (0x1b82)
3. Test GDS with our model to verify baseline functionality
4. Create Brief implementation of GDS core logic
5. Integrate with VITRIOL kernel module

## References
- NVIDIA GDS GitHub: https://github.com/NVIDIA/gds-nvidia-fs
- Specific file: src/nvfs-core.c
- Key functions to examine: nvfs_io_complete, nvfs_transit_state