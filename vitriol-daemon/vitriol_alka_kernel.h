/*
 * vitriol_alka_kernel.h — VITRIOL Alka ABI (Kernel-Safe Version)
 *
 * Copyright 2026 Randy Smits-Schreuder Goedheijt
 * Licensed under Apache 2.0 with Runtime Exception.
 *
 * Kernel-compatible version of the Alka C ABI contract.
 * Mirrors alka-handoff/vitriol_alka.h but uses __kernel types.
 */

#ifndef VITRIOL_ALKA_KERNEL_H
#define VITRIOL_ALKA_KERNEL_H

#include <linux/types.h>
#include <linux/stddef.h>

/* ── Drop Packet (32 bytes) ────────────────────────────────────── */

struct vitriol_drop {
    __u8   op_code;      /* Instruction opcode (0x01–0x3F)    */
    __u8   flags;        /* Execution flags                   */
    __u16  vessel_id;    /* Target Vessel identifier           */
    __u64  src_addr;     /* Source physical address            */
    __u64  dst_addr;     /* Destination physical address       */
    __u32  size;         /* Transfer size in bytes             */
    __u32  reserved;     /* Reserved (must be zero)            */
    __u32  crc;          /* Packet CRC checksum                */
} __attribute__((packed));

/* ── Opcodes ───────────────────────────────────────────────────── */

#define OP_CLAIM      0x01  /* Stake hardware node              */
#define OP_STAKE      0x02  /* Claim memory region              */
#define OP_FLOW       0x03  /* DMA transfer (SPARK-verified)    */
#define OP_SHIFT      0x04  /* Remap BAR window (SPARK)         */
#define OP_FENCE      0x05  /* Wait for metapage (SPARK)        */
#define OP_SYNC       0x06  /* Memory barrier                   */
#define OP_SENSE      0x07  /* Read sensor                      */
#define OP_PULSE      0x08  /* Timing signal                    */
#define OP_SIGNAL     0x09  /* GPU compute trigger (SPARK)      */
#define OP_YIELD      0x0A  /* Cooperative yield                */
#define OP_RECAST     0x0B  /* FPGA reconfigure                 */
#define OP_SNAP       0x0C  /* Serialize state                  */
#define OP_REVERT     0x0D  /* Restore state                    */
#define OP_LIMIT      0x0E  /* Thermal limit                    */
#define OP_MOLT       0x14  /* Full state dump                  */
#define OP_ECHO       0x17  /* Non-intrusive introspection      */
#define OP_STASIS     0x18  /* Bus-level locking                */
#define OP_FLUX       0x2A  /* Cache invalidation               */
#define OP_AUDIT      0x2B  /* Post-instruction residue check   */
#define OP_DRY_RUN    0x2C  /* Simulate without executing       */
#define OP_MOCK       0x2D  /* Use mock hardware                */
#define OP_PROVE      0x2E  /* Formal verification              */
#define OP_WATCH      0x2F  /* Real-time monitoring             */
#define OP_TRACE      0x30  /* Execution trace                  */
#define OP_GUARD      0x31  /* Runtime safety sentinel          */
#define OP_ISOLATE    0x32  /* Complete hardware isolation      */
#define OP_VERIFY     0x33  /* Cryptographic state verification */
#define OP_OSSIFY     0x34  /* Pin CPU core to Alka             */
#define OP_BOND       0x35  /* RAM-to-GPU direct tunnel         */
#define OP_STILL      0x36  /* Manual DRAM refresh control      */
#define OP_RESONATE   0x37  /* Coordinate reset for pure window */
#define OP_OSCILLATE  0x38  /* Dual-bank refresh coordination   */
#define OP_IMC_HIJACK 0x39  /* Direct memory controller access  */
#define OP_OCCUPY     0x3A  /* Seize PCIe device                */
#define OP_REFRACT    0x3B  /* Sub-tensor slicer (SPARK)        */
#define OP_PIPE       0x3C  /* Continuous DMA ring buffer       */

/* ── Vial Constraints (passed via IOCTL) ───────────────────────── */

struct vitriol_vial {
    __u64  aperture_size;   /* BAR window size in bytes        */
    __u64  aperture_max;    /* Maximum transfer window         */
    __u8   thermal_halt;    /* Temperature to halt at (°C)     */
    __u8   thermal_throttle;/* Temperature to throttle at (°C) */
    __u8   dma_capable;     /* 1 if DMA is available           */
    __u8   cooperative;     /* 1 = use nvidia P2P cooperative  */
    __u64  p2p_token;       /* nvidia P2P token for this VA    */
    __u32  va_space_token;  /* nvidia VA space token           */
    __u32  _pad;            /* Alignment padding               */
    __u64  gpu_va;          /* GPU virtual address for coop DMA*/
} __attribute__((packed));

/* ── Execution Result ──────────────────────────────────────────── */

struct vitriol_result {
    int      success;          /* 1 = success, 0 = failure       */
    __u64    cycles_spent;     /* CPU cycles consumed            */
    __u64    bytes_transferred;/* Bytes moved by DMA             */
    char     error_message[256];/* Null-terminated error string   */
};

/* ── CRC Utility (Alka ROL-XOR algorithm) ──────────────────────── */

static inline __u32 vitriol_compute_drop_crc(const struct vitriol_drop *drop)
{
    __u32 crc = 0;
    const __u8 *bytes = (const __u8 *)drop;
    size_t crc_offset = offsetof(struct vitriol_drop, crc);

    for (size_t i = 0; i < crc_offset; i++) {
        crc = (crc << 1) | (crc >> 31);  /* ROL 1 */
        crc ^= bytes[i];
    }

    return crc;
}

struct vitriol_azoth {
    __u8   op_code;
    __u8   flags;
    __u16  vessel_id;
    __u64  src_addr;
    __u64  dst_addr;
    __u32  size;
    __u32  reserved;
    __u32  crc;
} __attribute__((packed));

/* ── Stream Execution Request ──────────────────────────────────── */

struct vitriol_stream_req {
    __u64  drops_ptr;       /* User pointer to Drop array       */
    __u32  drop_count;      /* Number of Drops                  */
    __u32  _pad0;
    __u64  vial_ptr;        /* User pointer to Vial struct      */
    __u64  result_ptr;      /* User pointer to Result struct    */
    __u64  azoth_ptr;       /* User pointer to Azoth array      */
    __u32  azoth_count;     /* Number of Azoth packets          */
    __u32  _pad1;
} __attribute__((packed));

/* ── Source File (GGUF/NVMe path) ──────────────────────────────── */

/* Pass an already-opened file descriptor from userspace */
struct vitriol_source {
    __s32  fd;              /* Opened file descriptor           */
    __u32  _pad;
} __attribute__((packed));

/* ── BIND Request ──────────────────────────────────────────────── */

#define VITRIOL_BDF_MAX_LEN 13

struct vitriol_bind_req {
    char bdf[VITRIOL_BDF_MAX_LEN];   /* "0000:01:00.0\0"        */
    __u8  force;                      /* 0 = normal, 1 = force   */
    __u8  _pad[3];
} __attribute__((packed));

/* ── BAR1 Readback Request ─────────────────────────────────────── */

struct vitriol_bar1_read {
    __u64  bar1_offset;    /* Offset into BAR1                  */
    __u64  size;           /* Bytes to read                     */
    __u64  buf;            /* Userspace buffer address           */
} __attribute__((packed));

/* ── IOCTL Commands (0xA1 magic) ───────────────────────────────── */

#define VITRIOL_IOC_MAGIC     0xA1
#define VITRIOL_IOC_EXECUTE   _IOW(VITRIOL_IOC_MAGIC, 1, struct vitriol_drop)
#define VITRIOL_IOC_VALIDATE  _IOWR(VITRIOL_IOC_MAGIC, 2, struct vitriol_drop)
#define VITRIOL_IOC_SET_VIAL  _IOW(VITRIOL_IOC_MAGIC, 3, struct vitriol_vial)
#define VITRIOL_IOC_GET_RESULT _IOR(VITRIOL_IOC_MAGIC, 4, struct vitriol_result)
#define VITRIOL_IOC_STREAM    _IOW(VITRIOL_IOC_MAGIC, 5, struct vitriol_stream_req)
#define VITRIOL_IOC_SET_SOURCE _IOW(VITRIOL_IOC_MAGIC, 6, struct vitriol_source)
#define VITRIOL_IOC_BIND_DEVICE _IOW(VITRIOL_IOC_MAGIC, 7, struct vitriol_bind_req)
#define VITRIOL_IOC_READ_BAR1  _IOR(VITRIOL_IOC_MAGIC, 8, struct vitriol_bar1_read)

/* ── BAR1 Physical Info (for userspace mmap) ──────────────────── */

struct vitriol_bar1_info {
    __u64  phys_addr;      /* BAR1 physical address               */
    __u64  size;           /* BAR1 size in bytes                  */
} __attribute__((packed));

#define VITRIOL_IOC_GET_BAR1_PHYS _IOR(VITRIOL_IOC_MAGIC, 9, struct vitriol_bar1_info)

/* ── FLOW Buffer Readback ──────────────────────────────────────── */

struct vitriol_flow_buf {
    __u64  buf;            /* Userspace destination buffer         */
    __u64  size;           /* Max bytes to read                   */
} __attribute__((packed));

#define VITRIOL_IOC_GET_FLOW_BUF _IOR(VITRIOL_IOC_MAGIC, 10, struct vitriol_flow_buf)

#endif /* VITRIOL_ALKA_KERNEL_H */
