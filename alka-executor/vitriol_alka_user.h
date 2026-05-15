/*
 * vitriol_alka_user.h — VITRIOL Alka ABI (Userspace Version)
 *
 * Copyright 2026 Randy Smits-Schreuder Goedheijt
 * Licensed under Apache 2.0 with Runtime Exception.
 *
 * Userspace-compatible version of the Alka C ABI contract.
 * Mirrors vitriol-daemon/vitriol_alka_kernel.h but uses stdint types.
 */

#ifndef VITRIOL_ALKA_USER_H
#define VITRIOL_ALKA_USER_H

#include <stdint.h>
#include <stddef.h>

/* ── Drop Packet (32 bytes) ────────────────────────────────────── */

struct vitriol_drop {
    uint8_t  op_code;      /* Instruction opcode (0x01–0x3F)    */
    uint8_t  flags;        /* Execution flags                   */
    uint16_t vessel_id;    /* Target Vessel identifier           */
    uint64_t src_addr;     /* Source physical address            */
    uint64_t dst_addr;     /* Destination physical address       */
    uint32_t size;         /* Transfer size in bytes             */
    uint32_t reserved;     /* Reserved (must be zero)            */
    uint32_t crc;          /* Packet CRC checksum                */
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
    uint64_t aperture_size;   /* BAR window size in bytes        */
    uint64_t aperture_max;    /* Maximum transfer window         */
    uint8_t  thermal_halt;    /* Temperature to halt at (°C)     */
    uint8_t  thermal_throttle;/* Temperature to throttle at (°C) */
    uint8_t  dma_capable;     /* 1 if DMA is available           */
    uint8_t  cooperative;     /* 1 = use nvidia P2P cooperative  */
    uint64_t p2p_token;       /* nvidia P2P token for this VA    */
    uint32_t va_space_token;  /* nvidia VA space token           */
    uint32_t _pad;            /* Alignment padding               */
    uint64_t gpu_va;          /* GPU virtual address for coop DMA*/
} __attribute__((packed));

/* ── Execution Result ──────────────────────────────────────────── */

struct vitriol_result {
    int      success;          /* 1 = success, 0 = failure       */
    uint64_t cycles_spent;     /* CPU cycles consumed            */
    uint64_t bytes_transferred;/* Bytes moved by DMA             */
    char     error_message[256];/* Null-terminated error string   */
};

/* ── Azoth Rollback Packet (same layout as Drop) ───────────────── */

struct vitriol_azoth {
    uint8_t  op_code;
    uint8_t  flags;
    uint16_t vessel_id;
    uint64_t src_addr;
    uint64_t dst_addr;
    uint32_t size;
    uint32_t reserved;
    uint32_t crc;
} __attribute__((packed));

/* ── Stream Execution Request ──────────────────────────────────── */

struct vitriol_stream_req {
    uint64_t drops_ptr;       /* User pointer to Drop array       */
    uint32_t drop_count;      /* Number of Drops                  */
    uint32_t _pad0;
    uint64_t vial_ptr;        /* User pointer to Vial struct      */
    uint64_t result_ptr;      /* User pointer to Result struct    */
    uint64_t azoth_ptr;       /* User pointer to Azoth array      */
    uint32_t azoth_count;     /* Number of Azoth packets          */
    uint32_t _pad1;
} __attribute__((packed));

/* ── Source File (GGUF/NVMe path) ──────────────────────────────── */

/* Pass an already-opened file descriptor from userspace */
struct vitriol_source {
    int32_t  fd;            /* Opened file descriptor           */
    uint32_t _pad;
} __attribute__((packed));

/* ── BIND Request ──────────────────────────────────────────────── */

#define VITRIOL_BDF_MAX_LEN 13

struct vitriol_bind_req {
    char bdf[VITRIOL_BDF_MAX_LEN];   /* "0000:01:00.0\0"        */
    uint8_t  force;                   /* 0 = normal, 1 = force   */
    uint8_t  _pad[3];
} __attribute__((packed));

/* ── BAR1 Readback Request ─────────────────────────────────────── */

struct vitriol_bar1_read {
    uint64_t bar1_offset;    /* Offset into BAR1                  */
    uint64_t size;           /* Bytes to read                     */
    uint64_t buf;            /* Userspace buffer address           */
} __attribute__((packed));

/* ── IOCTL Commands (0xA1 magic) ───────────────────────────────── */

#include <sys/ioctl.h>

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
    uint64_t phys_addr;      /* BAR1 physical address               */
    uint64_t size;           /* BAR1 size in bytes                  */
} __attribute__((packed));

#define VITRIOL_IOC_GET_BAR1_PHYS _IOR(VITRIOL_IOC_MAGIC, 9, struct vitriol_bar1_info)

/* ── FLOW Buffer Readback ──────────────────────────────────────── */

struct vitriol_flow_buf {
    uint64_t buf;            /* Userspace destination buffer         */
    uint64_t size;           /* Max bytes to read                   */
} __attribute__((packed));

#define VITRIOL_IOC_GET_FLOW_BUF _IOR(VITRIOL_IOC_MAGIC, 10, struct vitriol_flow_buf)

/* ── Legacy IOCTL (for MAP_BAR after unbind) ─────────────────── */

#define VITRIOL_IOC_MAGIC_LEGACY 'V'
#define VITRIOL_IOC_MAP_BAR      _IO(VITRIOL_IOC_MAGIC_LEGACY, 0)

/* ── CRC32 Utility (Alka ROL-XOR algorithm) ────────────────────── */

static inline uint32_t compute_drop_crc(const struct vitriol_drop *drop)
{
    uint32_t crc = 0;
    const uint8_t *bytes = (const uint8_t *)drop;
    size_t crc_offset = offsetof(struct vitriol_drop, crc);

    for (size_t i = 0; i < crc_offset; i++) {
        crc = (crc << 1) | (crc >> 31);  /* ROL 1 */
        crc ^= bytes[i];
    }

    return crc;
}

#endif /* VITRIOL_ALKA_USER_H */
