# Copy Engine Hijack — Implementation Plan

**Goal**: Direct NVMe→VRAM DMA via GPU's internal Copy Engine, bypassing CUDA/cpu-mediated `cudaMemcpyAsync`. Single-command `vitriol run` on an active, in-use GPU.

**Hardware**: GTX 1070 Ti (Pascal GP104, SM 6.1) — active display GPU.
**Model**: Qwen3.6-35B-A3B (MoE, 8/256 active experts, ~42 MB/expert).
**Target**: ~17 tok/s with VRAM expert cache, 0 CPU copy overhead.

---

## Architecture

```
llama.cpp ──→ vitriol_cuda_set_tensor_hook()
                  │
                  ├─ VITRIOL_MODE_DISABLED → cudaMemcpyAsync (original path)
                  │
                  └─ VITRIOL_MODE_STREAM ──→ copy_engine_dma()
                                                 │
                    ┌──────────────────────────────────────────────┐
                    │ 1. Allocate pinned bounce buffer (cudaHostAlloc) │
                    │ 2. Read expert from GGUF via io_uring+O_DIRECT  │
                    │ 3. Build NV_C0B5 pushbuffer opcodes            │
                    │ 4. Write entries-to-stream GPFIFO ring buffer   │
                    │ 5. Increment GPPUT, ring doorbell in UserD      │
                    │ 6. Spin on metapage fence (volatile uint32_t*)  │
                    │ 7. Return true (skip cudaMemcpyAsync)           │
                    └──────────────────────────────────────────────┘
```

### Key insight

NVIDIA's CUDA UMD (User Mode Driver) creates a `MaxwellChannelGPFIFO` (class `0xB06F`) for every `cudaStream_t`. This includes a `UserD` control page (mapped into userspace) containing `GPGET`, `GPPUT`, and `DOORBELL` registers. The Copy Engine (class `0xC0B5` on Pascal) accepts pushbuffer commands via this channel.

We do NOT need kernel code. Everything happens in userspace:
- Extract `UserD` pointer, `GPFIFO` pointer, `Doorbell token` from a dedicated DMA CUDA stream
- Write `NV_C0B5` opcodes to a host-memory pushbuffer
- Write a GPFIFO entry pointing to our pushbuffer
- Increment `GPPUT` and write `channel_token` to `UserD + 0x90`

---

## Files to create/modify

| File | Action | Purpose |
|------|--------|---------|
| `vitriol-daemon/vitriol_copy_engine.h` | **CREATE** | C structs and macros for NV_C0B5, GPFIFO, UserD, doorbell |
| `vitriol-daemon/vitriol_copy_engine.c` | **CREATE** | Pushbuffer builder, GPFIFO submitter, stream scanner |
| `vitriol-daemon/Makefile` | MODIFY | Add `vitriol_copy_engine.o` to kernel module (empty, userspace-only for now) |
| `alka-executor/executor.c` | MODIFY | Add `--copy-engine` flag for standalone CE test |
| `llama.cpp-patches/source/vitriol-cuda-integration.h` | MODIFY | Add `vitriol_ce_init()`, `vitriol_ce_dma()` decls |
| `llama.cpp-patches/source/vitriol-cuda-integration.cpp` | MODIFY | Implement Copy Engine DMA path |

---

## Step 1: Define hardware constants and structures

File: `alka-executor/vitriol_copy_engine.h`

From NVIDIA `open-gpu-kernel-modules` headers:

### NV_C0B5 (Pascal DMA Copy Engine) — class 0xC0B5

```c
// Pushbuffer method header format
#define NV_METHOD_INCR(count, method)  (((2) << 28) | ((count) << 16) | (method))
#define NV_METHOD_NONINCR(count, method) (((3) << 28) | ((count) << 16) | (method))
#define NV_METHOD_IMMD(method, data)   (((4) << 28) | ((method) << 16) | (data))

// NV_C0B5 register offsets (from clc0b5.h)
#define NVC0B5_SET_SEMAPHORE_A      0x00000240
#define NVC0B5_SET_SEMAPHORE_B      0x00000244
#define NVC0B5_SET_SEMAPHORE_PAYLOAD 0x00000248
#define NVC0B5_LAUNCH_DMA           0x00000300
#define NVC0B5_OFFSET_IN_UPPER      0x00000400
#define NVC0B5_OFFSET_IN_LOWER      0x00000404
#define NVC0B5_OFFSET_OUT_UPPER     0x00000408
#define NVC0B5_OFFSET_OUT_LOWER     0x0000040C
#define NVC0B5_PITCH_IN             0x00000410
#define NVC0B5_PITCH_OUT            0x00000414
#define NVC0B5_LINE_LENGTH_IN       0x00000418
#define NVC0B5_LINE_COUNT           0x0000041C

// LAUNCH_DMA flags
#define NVC0B5_LAUNCH_DMA_TRANSFER_TYPE_PIPELINED      (0x00000001)
#define NVC0B5_LAUNCH_DMA_SRC_MEMORY_LAYOUT_PITCH      (0x00000001 << 7)
#define NVC0B5_LAUNCH_DMA_DST_MEMORY_LAYOUT_PITCH      (0x00000001 << 8)
#define NVC0B5_LAUNCH_DMA_SRC_TYPE_VIRTUAL             (0x00000000 << 12)
#define NVC0B5_LAUNCH_DMA_DST_TYPE_VIRTUAL             (0x00000000 << 13)
#define NVC0B5_LAUNCH_DMA_SEMAPHORE_TYPE_RELEASE_1W    (0x00000001 << 3)
#define NVC0B5_LAUNCH_DMA_INTERRUPT_TYPE_NONE          (0x00000000 << 5)

// SEMAPHORE flags (from NVC0B5_SET_SEMAPHORE_*)
#define SEMAPHORE_OP_RELEASE   (1)
#define SEMAPHORE_RELEASE_WFI  (1 << 20)  // Wait For Idle
```

### Maxwell Channel GPFIFO — UserD structure (from clb06f.h)

```c
// UserD control page layout (MUST be volatile — GPU writes GPGET)
typedef volatile struct {
    uint32_t Ignored00[0x010];   // 0x0000-0x003F
    uint32_t Put;                // 0x0040 — write offset
    uint32_t Get;                // 0x0044 — read offset (GPU updates)
    uint32_t Reference;          // 0x0048
    uint32_t PutHi;              // 0x004C
    uint32_t Ignored01[0x002];   // 0x0050-0x0057
    uint32_t TopLevelGet;        // 0x0058
    uint32_t TopLevelGetHi;      // 0x005C
    uint32_t GetHi;              // 0x0060
    uint32_t Ignored02[0x007];   // 0x0064-0x007F
    uint32_t Ignored03;          // 0x0080
    uint32_t Ignored04;          // 0x0084-0x0087
    uint32_t GPGet;              // 0x0088 — GPFIFO get offset
    uint32_t GPPut;              // 0x008C — GPFIFO put offset (WE WRITE THIS)
    uint32_t Ignored05[0x5C];    // padding
} nvc0_userd_t;

// GPFIFO entry (8 bytes each)
typedef struct {
    uint32_t addr_lo;      // bits 31:2 = pushbuffer physical addr (lower)
    uint32_t addr_hi_len;  // bits 7:0 = addr upper, bits 30:10 = length in dwords
} __attribute__((packed)) gpfifo_entry_t;

// GPFIFO entry bitfields (from NVB06F_GP_ENTRY)
// addr_lo: [31:2] = PB base address >> 2, [1:0] = fetch type
// addr_hi_len: [7:0] = PB address upper, [8] = priv, [9] = level,
//              [30:10] = length in dwords, [31] = sync
```

### UserD base format

From `clb06f.h` (Maxwell Channel GPFIFO):
- Class: `0xB06F`
- UserD base offset in BAR0: varies per chip, typically found by scanning `rw-s` mappings to `/dev/nvidia*`
- GPFIFO entries: allocated by CUDA UMD at stream creation time
- Doorbell: `UserD + 0x90` — write the channel token to this register

---

## Step 2: Implement the stream hijack

File: `alka-executor/vitriol_copy_engine.h` + `.c`

### Finding the UserD page

The key challenge: CUDA's `cudaStream_t` is opaque. NVIDIA's `NvPushChannelRec` (from `nvidia-push-types.h`) contains:

```c
struct _NvPushChannelRec {
    ...
    void *control[NV_MAX_SUBDEVICES];    // UserD mapping pointer
    NvU32 *gpfifo;                       // GPFIFO ring buffer
    NvU32  gpPutOffset;                  // last GPFIFO put offset
    NvU32  numGpFifoEntries;             // ring buffer size
    ...
};
```

We cannot access this struct from userspace directly (it's in `libcuda.so`'s heap). Instead, we locate the UserD page via `/proc/self/maps`:

**Method**: Create a dedicated CUDA stream, then scan `/proc/self/maps` for `rw-s` mappings to `/dev/nvidia*` devices. The UserD page has a distinct signature: `GPGET == GPPUT` when idle, and both increment when the stream executes work.

```c
typedef struct {
    nvc0_userd_t *userD;        // the UserD control page
    gpfifo_entry_t *gpfifo;     // the GPFIFO ring buffer
    uint32_t num_entries;       // GPFIFO ring size
    uint32_t channel_token;     // doorbell token
    cudaStream_t stream;        // the CUDA stream we hijacked
    volatile uint32_t *fence;   // metapage semaphore
    void *bounce_buffer;        // pinned host memory (cudaHostAlloc)
    void *bounce_devptr;        // GPU virtual address of bounce buffer
} vitriol_ce_channel_t;
```

### Stream initialization

```c
bool vitriol_ce_init(vitriol_ce_channel_t *chan) {
    // 1. Create dedicated DMA CUDA stream
    cudaStreamCreate(&chan->stream);
    
    // 2. Scan /proc/self/maps for UserD pages
    //    - Find all "rw-s" mappings to /dev/nvidia*
    //    - Cast each to nvc0_userd_t*
    //    - Launch tiny CUDA kernel on stream, check GPGET/GPPUT change
    //    - The one that increments is our channel
    if (!scan_userd_page(chan)) return false;
    
    // 3. Locate GPFIFO buffer (adjacent in process heap)
    //    - GPFIFO entries contain addresses matching known PB patterns
    if (!locate_gpfifo(chan)) return false;
    
    // 4. Extract doorbell token
    //    - Stored in UserD page at offset 0x90
    //    - Written by UMD when cudaLaunchKernel rings the bell
    //    - We can read it from the UserD page after a kernel launch
    chan->channel_token = read_doorbell_token(chan);
    
    // 5. Allocate pinned bounce buffer
    cudaHostAlloc(&chan->bounce_buffer, BOUNCE_SIZE, cudaHostAllocMapped);
    cudaHostGetDevicePointer(&chan->bounce_devptr, chan->bounce_buffer, 0);
    
    // 6. Allocate metapage fence (pinned host memory)
    cudaHostAlloc(&chan->fence, 4096, cudaHostAllocDefault);
    *chan->fence = 0;
    
    return true;
}
```

---

## Step 3: Implement pushbuffer builder

File: `alka-executor/vitriol_copy_engine.c`

### DMA transfer pushbuffer

```c
uint32_t build_dma_pushbuffer(
    uint32_t *pb,              // pushbuffer output array
    uint64_t src_gpu_addr,     // source: GPU VA of bounce buffer
    uint64_t dst_gpu_addr,     // dest: GPU VA of VRAM allocation
    uint32_t size,             // transfer size in bytes
    uint64_t fence_gpu_addr    // GPU VA of metapage fence
) {
    uint32_t i = 0;
    
    // 1. Source address (low + high)
    pb[i++] = NV_METHOD_INCR(2, NVC0B5_OFFSET_IN_UPPER);
    pb[i++] = (uint32_t)(src_gpu_addr >> 32);
    pb[i++] = (uint32_t)(src_gpu_addr & 0xFFFFFFFF);
    
    // 2. Destination address (low + high)
    pb[i++] = NV_METHOD_INCR(2, NVC0B5_OFFSET_OUT_UPPER);
    pb[i++] = (uint32_t)(dst_gpu_addr >> 32);
    pb[i++] = (uint32_t)(dst_gpu_addr & 0xFFFFFFFF);
    
    // 3. Pitch (linear 1D: pitch_in = pitch_out = line_length = size)
    pb[i++] = NV_METHOD_INCR(3, NVC0B5_PITCH_IN);
    pb[i++] = size;              // PITCH_IN
    pb[i++] = size;              // PITCH_OUT
    pb[i++] = size;              // LINE_LENGTH_IN
    
    // 4. Line count (1 for linear transfer)
    pb[i++] = NV_METHOD_INCR(1, NVC0B5_LINE_COUNT);
    pb[i++] = 1;
    
    // 5. LAUNCH_DMA with virtual addresses, pitch layout, release semaphore
    pb[i++] = NV_METHOD_INCR(1, NVC0B5_LAUNCH_DMA);
    pb[i++] = NVC0B5_LAUNCH_DMA_TRANSFER_TYPE_PIPELINED
            | NVC0B5_LAUNCH_DMA_SRC_MEMORY_LAYOUT_PITCH
            | NVC0B5_LAUNCH_DMA_DST_MEMORY_LAYOUT_PITCH
            | NVC0B5_LAUNCH_DMA_SRC_TYPE_VIRTUAL
            | NVC0B5_LAUNCH_DMA_DST_TYPE_VIRTUAL
            | NVC0B5_LAUNCH_DMA_SEMAPHORE_TYPE_RELEASE_1W;
    
    // 6. Semaphore (metapage fence) — release=1 after DMA completes
    pb[i++] = NV_METHOD_INCR(4, NVC0B5_SET_SEMAPHORE_A);
    pb[i++] = (uint32_t)(fence_gpu_addr >> 32);   // SEMAPHORE_A (upper)
    pb[i++] = (uint32_t)(fence_gpu_addr & 0xFFFFFFFF); // SEMAPHORE_B (lower)
    pb[i++] = 1;                                      // SEMAPHORE_PAYLOAD
    pb[i++] = SEMAPHORE_OP_RELEASE | SEMAPHORE_RELEASE_WFI; // SEMAPHORE_C (ctrl)
    
    return i;  // number of dwords written
}
```

### GPFIFO submitter

```c
void submit_pushbuffer(
    nvc0_userd_t *userD,
    gpfifo_entry_t *gpfifo,
    uint32_t num_entries,
    uint32_t pb_gpu_addr_low,   // physical address low (or GPU VA)
    uint32_t pb_dword_count,
    uint32_t channel_token
) {
    uint32_t put = userD->GPPut;
    uint32_t next = (put + 1) % num_entries;
    
    // Wait for GPGET to catch up if ring is full
    while (next == userD->GPGet) {
        _mm_pause();
    }
    
    // Write GPFIFO entry
    gpfifo[put].addr_lo = (pb_gpu_addr_low >> 2) & 0x3FFFFFFF;  // 30-bit addr
    gpfifo[put].addr_hi_len = (pb_dword_count & 0x7FFFFF) << 10; // length in dwords
    
    // Memory barrier — GPFIFO write must be visible before doorbell
    __sync_synchronize();
    
    // Advance GPPut
    userD->GPPut = next;
    
    // Memory barrier — GPPut must be visible before doorbell
    __sync_synchronize();
    
    // Ring the doorbell (UserD + 0x90)
    userD->DOORBELL = channel_token;
}
```

### Metapage fence wait

```c
void wait_for_fence(volatile uint32_t *fence) {
    while (*fence == 0) {
        _mm_pause();
    }
    *fence = 0;  // reset for next transfer
}
```

---

## Step 4: Integration with vitriol_cuda_set_tensor_hook

File: `llama.cpp-patches/source/vitriol-cuda-integration.cpp`

### Trigger path

When `vitriol_cuda_set_tensor_hook()` is called with `VITRIOL_MODE_STREAM`:

```c
bool vitriol_cuda_set_tensor_hook(ggml_tensor *tensor, const void *data,
                                   size_t size, uint64_t file_off) {
    if (g_vitriol_config.mode != VITRIOL_MODE_STREAM)
        return false;
    
    // 1. Get the tensor's GPU pointer (where it lives in VRAM)
    //    ggml_tensor->data is set by ggml_backend_cuda_buffer_alloc
    cudaPointerAttributes attr;
    cudaPointerGetAttributes(&attr, tensor->data);
    // attr.devicePointer = GPU virtual address
    
    // 2. Read stream from CUDA backend (cuda_ctx->stream())
    //    We use our dedicated DMA stream, not cudaStreamPerThread
    cudaStream_t dma_stream = get_dma_stream();
    
    // 3. io_uring read from GGUF into bounce buffer
    size_t aligned_off = file_off & ~(4095);
    size_t aligned_size = ((size + (file_off - aligned_off) + 4095) & ~(4095));
    io_uring_read(gguf_fd, bounce_buffer + (file_off - aligned_off),
                  aligned_off, aligned_size);
    
    // 4. Build pushbuffer command
    uint32_t pb[32];
    uint32_t nwords = build_dma_pushbuffer(
        pb,
        bounce_devptr + (file_off - aligned_off),  // src GPU VA
        (uint64_t)(uintptr_t)device_ptr,            // dst GPU VA (VRAM)
        size,
        fence_gpu_addr                              // metapage
    );
    
    // 5. Submit to GPFIFO and ring doorbell
    submit_pushbuffer(userD, gpfifo, num_entries, 
                      pb_phys_addr, nwords, channel_token);
    
    // 6. Wait for completion (spin on metapage)
    wait_for_fence(&fence);
    
    vitriol_stats.stream_loads++;
    return true;  // skip cudaMemcpyAsync
}
```

---

## Step 5: The io_uring + O_DIRECT + double-buffer optimization

File: `llama.cpp-patches/source/vitriol-cuda-integration.cpp`

```c
typedef struct {
    int gguf_fd;                      // GGUF file with O_DIRECT
    struct io_uring ring;             // io_uring instance
    uint8_t *bounce_a, *bounce_b;     // double buffers (cudaHostAlloc)
    uint64_t bounce_devptr_a;         // GPU VAs of bounce buffers
    uint64_t bounce_devptr_b;
    bool active_buffer;               // true = A, false = B
} vitriol_nvme_t;

// Full-pipeline hot loop (per token):
// 1. Submit io_uring read for expert N+1 into bounce_b
// 2. Wait for io_uring read of expert N (completed bounce_a)
// 3. Submit CE DMA for expert N from bounce_a to VRAM
// 4. While CE runs, expert N+1 is being read from NVMe
// 5. Swap buffers, repeat
```

---

## Step 6: Standalone test (before llama.cpp integration)

File: `alka-executor/executor.c`

Add `--copy-engine` flag that:
1. Creates a CUDA stream
2. Scans for UserD/GPFIFO/ChannelToken  
3. Allocates bounce buffer + VRAM test buffer
4. Reads GGUF header via io_uring into bounce
5. Builds and submits a 4KB NV_C0B5 DMA
6. Waits on metapage fence
7. Verifies VRAM content matches GGUF magic (`47 47 55 46`)

---

## Implementation order

| Step | What | Verification |
|------|------|-------------|
| 1 | Write header constants | Compiles |
| 2 | Write stream scanner | Prints UserD/GPFIFO pointers |
| 3 | Write pushbuffer builder | Compiles, correct opcodes printed |
| 4 | Write GPFIFO submitter | Doorbell rings, no GPU hang |
| 5 | Write metapage fence | Fence turns 1 after DMA |
| 6 | 4KB copy test | VRAM reads back `47 47 55 46` |
| 7 | Wire into llamacpp hook | `vitriol run` produces tokens |
| 8 | io_uring + O_DIRECT | Overlapped NVMe reads |
| 9 | Double-buffer pipeline | Peak throughput |
| 10 | Alka FENCE integration | FENCE drop polls metapage |

---

## Risks and mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| UserD scan fails to find page | CE can't initialize | Fall back to cudaMemcpyAsync |
| GPFIFO ring buffer corruption | GPU hang, system freeze | Use dedicated DMA stream, not compute stream |
| Channel token wrong | Doorbell ignored, DMA never starts | Read from /dev/nvidiactl debug interface |
| O_DIRECT alignment mismatch | io_uring returns EINVAL | Round reads to 4KB boundaries |
| Pascal CE concurrency limit | CE blocks on compute | Use non-blocking semantics, small transfers |
| metapage never written | Infinite spin | Add timeout + fallback to cudaMemcpyAsync |
| nvidia driver update changes ABI | Stream scanner breaks | Version check, fallback |
