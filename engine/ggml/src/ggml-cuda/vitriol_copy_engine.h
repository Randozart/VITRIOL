/*
 * vitriol_copy_engine.h — NV_C0B5 Copy Engine DMA hijack
 *
 * Direct NVMe→VRAM DMA via GPU's internal Copy Engine,
 * bypassing CPU-mediated cudaMemcpyAsync.
 *
 * Based on NVIDIA open-gpu-kernel-modules:
 *   - src/common/sdk/nvidia/inc/class/clc0b5.h (Pascal DMA Copy Engine)
 *   - src/common/sdk/nvidia/inc/class/clb06f.h (Maxwell Chan GPFIFO)
 *   - src/common/unix/nvidia-push/interface/nvidia-push-types.h
 *
 * Copyright 2026
 * Licensed under Apache 2.0 with Runtime Exception.
 */

#ifndef VITRIOL_COPY_ENGINE_H
#define VITRIOL_COPY_ENGINE_H

#include <stdint.h>
#include <stddef.h>
#include <cuda_runtime.h>
#include <cuda.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ══════════════════════════════════════════════════════════════════
   NV_C0B5 — Pascal DMA Copy Engine (class 0xC0B5)
   From clc0b5.h: #define PASCAL_DMA_COPY_A (0x0000C0B5)
   ══════════════════════════════════════════════════════════════════ */

/* Pushbuffer method header format
 * Bits 31:29 = sec_op (1=INCR, 3=NONINCR)
 * Bits 28:16 = method count
 * Bits 15:0  = method offset
 */
#define NV_METHOD_INCR(count, method)   (((1) << 29) | ((count) << 16) | (method))
#define NV_METHOD_NONINCR(count, method) (((3) << 29) | ((count) << 16) | (method))

/* ── NV_C0B5 Register Offsets ─────────────────────────────────── */

#define NVC0B5_SET_SEMAPHORE_A       0x00000240
#define NVC0B5_SET_SEMAPHORE_A_UPPER 16:0
#define NVC0B5_SET_SEMAPHORE_B       0x00000244
#define NVC0B5_SET_SEMAPHORE_B_LOWER 31:0
#define NVC0B5_SET_SEMAPHORE_PAYLOAD 0x00000248
#define NVC0B5_SET_SEMAPHORE_PAYLOAD_PAYLOAD 31:0

#define NVC0B5_LAUNCH_DMA            0x00000300

/* LAUNCH_DMA bitfields (from clc0b5.h lines 67-125) */
#define NVC0B5_LAUNCH_DMA_DATA_TRANSFER_TYPE           1:0
#define NVC0B5_LAUNCH_DMA_DATA_TRANSFER_TYPE_NONE      0x00000000
#define NVC0B5_LAUNCH_DMA_DATA_TRANSFER_TYPE_PIPELINED 0x00000001

#define NVC0B5_LAUNCH_DMA_FLUSH_ENABLE         2:2
#define NVC0B5_LAUNCH_DMA_FLUSH_ENABLE_FALSE   0x00000000
#define NVC0B5_LAUNCH_DMA_FLUSH_ENABLE_TRUE    0x00000001

#define NVC0B5_LAUNCH_DMA_SEMAPHORE_TYPE             4:3
#define NVC0B5_LAUNCH_DMA_SEMAPHORE_TYPE_NONE        0x00000000
#define NVC0B5_LAUNCH_DMA_SEMAPHORE_TYPE_RELEASE_1W  0x00000001

#define NVC0B5_LAUNCH_DMA_INTERRUPT_TYPE             6:5
#define NVC0B5_LAUNCH_DMA_INTERRUPT_TYPE_NONE        0x00000000

#define NVC0B5_LAUNCH_DMA_SRC_MEMORY_LAYOUT         7:7
#define NVC0B5_LAUNCH_DMA_SRC_MEMORY_LAYOUT_PITCH   0x00000001

#define NVC0B5_LAUNCH_DMA_DST_MEMORY_LAYOUT         8:8
#define NVC0B5_LAUNCH_DMA_DST_MEMORY_LAYOUT_PITCH   0x00000001

#define NVC0B5_LAUNCH_DMA_SRC_TYPE                  12:12
#define NVC0B5_LAUNCH_DMA_SRC_TYPE_VIRTUAL          0x00000000
#define NVC0B5_LAUNCH_DMA_SRC_TYPE_PHYSICAL         0x00000001

#define NVC0B5_LAUNCH_DMA_DST_TYPE                  13:13
#define NVC0B5_LAUNCH_DMA_DST_TYPE_VIRTUAL          0x00000000
#define NVC0B5_LAUNCH_DMA_DST_TYPE_PHYSICAL         0x00000001

#define NVC0B5_LAUNCH_DMA_SRC_BYPASS_L2             20:20
#define NVC0B5_LAUNCH_DMA_SRC_BYPASS_L2_DEFAULT     0x00000000
#define NVC0B5_LAUNCH_DMA_DST_BYPASS_L2             21:21
#define NVC0B5_LAUNCH_DMA_DST_BYPASS_L2_DEFAULT     0x00000000

/* ── Address registers ─────────────────────────────────────────── */

#define NVC0B5_OFFSET_IN_UPPER   0x00000400
#define NVC0B5_OFFSET_IN_LOWER   0x00000404
#define NVC0B5_OFFSET_OUT_UPPER  0x00000408
#define NVC0B5_OFFSET_OUT_LOWER  0x0000040C
#define NVC0B5_PITCH_IN          0x00000410
#define NVC0B5_PITCH_OUT          0x00000414
#define NVC0B5_LINE_LENGTH_IN    0x00000418
#define NVC0B5_LINE_COUNT        0x0000041C

/* ── SEMAPHORE flags (for NVC0B5_SET_SEMAPHORE_PAYLOAD) ────────── */

#define NVC0B5_SEMAPHORE_OP_RELEASE  0x00000001
#define NVC0B5_SEMAPHORE_RELEASE_WFI 0x00100000  /* Wait For Idle */

/* Build the LAUNCH_DMA register value for a 1D pitch transfer
 * with virtual addresses and 1-word release semaphore */
#define NVC0B5_LAUNCH_DMA_VIRT_1D \
    (NVC0B5_LAUNCH_DMA_DATA_TRANSFER_TYPE_PIPELINED | \
     NVC0B5_LAUNCH_DMA_FLUSH_ENABLE_FALSE |           \
     NVC0B5_LAUNCH_DMA_SEMAPHORE_TYPE_RELEASE_1W |    \
     NVC0B5_LAUNCH_DMA_INTERRUPT_TYPE_NONE |           \
     NVC0B5_LAUNCH_DMA_SRC_MEMORY_LAYOUT_PITCH |      \
     NVC0B5_LAUNCH_DMA_DST_MEMORY_LAYOUT_PITCH |      \
     NVC0B5_LAUNCH_DMA_SRC_TYPE_VIRTUAL |              \
     NVC0B5_LAUNCH_DMA_DST_TYPE_VIRTUAL |              \
     NVC0B5_LAUNCH_DMA_SRC_BYPASS_L2_DEFAULT |         \
     NVC0B5_LAUNCH_DMA_DST_BYPASS_L2_DEFAULT)


/* ══════════════════════════════════════════════════════════════════
   NVB06F — Maxwell Channel GPFIFO (class 0xB06F)
   From clb06f.h: #define MAXWELL_CHANNEL_GPFIFO_A (0x0000B06F)
   ══════════════════════════════════════════════════════════════════ */

/* UserD control page structure
 * From clb06f.h lines 47-63:
 *   typedef volatile struct _clb06f_tag0 { ... } Nvb06FControl;
 *
 * GPU writes: Get, GPGet, TopLevelGet
 * CPU writes: Put, GPPut, DOORBELL (at 0x90)
 */
typedef volatile struct {
    uint32_t Ignored00[0x010];   /* 0x0000-0x003F */
    uint32_t Put;                /* 0x0040 */
    uint32_t Get;                /* 0x0044 — GPU updates */
    uint32_t Reference;          /* 0x0048 */
    uint32_t PutHi;              /* 0x004C */
    uint32_t Ignored01[0x002];   /* 0x0050-0x0057 */
    uint32_t TopLevelGet;        /* 0x0058 — GPU updates */
    uint32_t TopLevelGetHi;      /* 0x005C — GPU updates */
    uint32_t GetHi;              /* 0x0060 */
    uint32_t Ignored02[0x007];   /* 0x0064-0x007F */
    uint32_t Ignored03;          /* 0x0080 */
    uint32_t Ignored04[0x001];   /* 0x0084-0x0087 */
    uint32_t GPGet;              /* 0x0088 — GP get offset, GPU updates */
    uint32_t GPPut;              /* 0x008C — GP put offset, CPU writes */
    uint32_t Ignored05[0x5C];    /* padding */
} nvc0_userd_t;

/* Doorbell register is at UserD offset 0x90.
 * Writing the channel token here wakes the GPU's PBDMA.
 * From clb06f.h: No explicit DOORBELL field exists in the struct,
 * but the next word after Ignored05 (0x008F) is at 0x0090.
 * The nvidia-push code references it as the "kickoff" register.
 */
#define USERD_DOORBELL_OFFSET 0x90

/* GPFIFO entry format (8 bytes each)
 * From clb06f.h lines 156-177:
 *   NVB06F_GP_ENTRY__SIZE = 8
 *   NVB06F_GP_ENTRY0_FETCH       0:0  (0=unconditional)
 *   NVB06F_GP_ENTRY0_GET        31:2  (PB address >> 2)
 *   NVB06F_GP_ENTRY1_GET_HI      7:0
 *   NVB06F_GP_ENTRY1_PRIV        8:8  (0=user)
 *   NVB06F_GP_ENTRY1_LEVEL       9:9  (0=main)
 *   NVB06F_GP_ENTRY1_LENGTH    30:10  (length in dwords)
 *   NVB06F_GP_ENTRY1_SYNC       31:31 (0=proceed)
 */
typedef struct {
    uint32_t addr_lo;       /* [31:2] = PB phys addr >> 2, [1:0] = fetch */
    uint32_t addr_hi_priv_level_len_sync;
} __attribute__((packed)) nvc0_gpfifo_entry_t;

#define GPFIFO_ENTRY_FETCH_UNCONDITIONAL  0x00000000
#define GPFIFO_ENTRY_PRIV_USER            0x00000000
#define GPFIFO_ENTRY_LEVEL_MAIN           0x00000000
#define GPFIFO_ENTRY_SYNC_PROCEED         0x00000000

static inline void gpfifo_entry_set(nvc0_gpfifo_entry_t *e,
    uint64_t pb_phys_addr, uint32_t pb_dwords)
{
    e->addr_lo = (uint32_t)((pb_phys_addr >> 2) & 0x3FFFFFFF)
               | GPFIFO_ENTRY_FETCH_UNCONDITIONAL;
    e->addr_hi_priv_level_len_sync =
        (uint32_t)((pb_phys_addr >> 32) & 0xFF) << 0 |
        GPFIFO_ENTRY_PRIV_USER              << 8  |
        GPFIFO_ENTRY_LEVEL_MAIN             << 9  |
        ((pb_dwords & 0x7FFFFF) << 10)            |
        GPFIFO_ENTRY_SYNC_PROCEED           << 31;
}


/* ══════════════════════════════════════════════════════════════════
   VITRIOL Copy Engine Channel State
   ══════════════════════════════════════════════════════════════════ */

/* Max pushbuffer dwords for a single DMA command */
#define CE_PUSHBUFFER_MAX_DWORDS 64

/* Max bounce buffer size (2 × expert size) */
#define CE_BOUNCE_SIZE (256UL * 1024 * 1024)

/* Default GPFIFO ring size (NVIDIA default for CUDA streams) */
#define CE_GPFIFO_ENTRIES 512

typedef struct {
    /* CUDA stream we're piggybacking on */
    cudaStream_t stream;

    /* UserD control page (mapped by CUDA UMD) */
    nvc0_userd_t *userD;

    /* GPFIFO ring buffer (allocated by CUDA UMD) */
    nvc0_gpfifo_entry_t *gpfifo;
    uint32_t gpfifo_entries;

    /* Channel doorbell token */
    uint32_t channel_token;

    /* Pinned host bounce buffer (cudaHostAlloc) */
    void *bounce_cpu;
    CUdeviceptr bounce_gpu;   /* GPU VA of bounce buffer */
    size_t bounce_size;

    /* Metapage fence in pinned host memory */
    volatile uint32_t *fence_cpu;
    CUdeviceptr fence_gpu;    /* GPU VA of metapage */

    /* Pushbuffer in host memory */
    uint32_t pushbuffer[CE_PUSHBUFFER_MAX_DWORDS];
} vitriol_ce_channel_t;


/* ══════════════════════════════════════════════════════════════════
   API
   ══════════════════════════════════════════════════════════════════ */

/* Initialize Copy Engine channel:
 *   - Creates dedicated CUDA stream
 *   - Scans /proc/self/maps for UserD page
 *   - Locates GPFIFO ring buffer
 *   - Extracts channel token
 *   - Allocates bounce buffer + metapage
 */
int vitriol_ce_init(vitriol_ce_channel_t *chan);

/* Perform a single DMA transfer via Copy Engine:
 *   - src_gpu_va: GPU virtual address of bounce buffer data
 *   - dst_gpu_va: GPU virtual address of VRAM destination
 *   - size: bytes to transfer
 *   - Returns 0 on success, -1 on timeout/failure
 */
int vitriol_ce_dma(vitriol_ce_channel_t *chan,
                   CUdeviceptr src_gpu_va,
                   CUdeviceptr dst_gpu_va,
                   uint32_t size);

/* Build NV_C0B5 pushbuffer (write into pb, return dword count) */
uint32_t vitriol_ce_build_pushbuffer(uint32_t *pb,
    CUdeviceptr src_gpu_va, CUdeviceptr dst_gpu_va,
    uint32_t size, CUdeviceptr fence_gpu_va);

/* Submit pushbuffer via GPFIFO and ring doorbell */
void vitriol_ce_submit(vitriol_ce_channel_t *chan,
    const uint32_t *pb, uint32_t pb_dwords);

/* Wait for DMA completion (spin on metapage) */
int vitriol_ce_wait(vitriol_ce_channel_t *chan, uint32_t timeout_ms);

/* Cleanup */
void vitriol_ce_destroy(vitriol_ce_channel_t *chan);

#ifdef __cplusplus
}
#endif

#endif /* VITRIOL_COPY_ENGINE_H */
