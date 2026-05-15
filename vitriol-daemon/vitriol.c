/*
 * VITRIOL Kernel Module - NVMe to GPU Direct Memory Access
 * Based on NVIDIA GPUDirect Storage patterns
 *
 * Target Hardware: GTX 1070 Ti (Pascal, device ID 1b82)
 * BAR Configuration: BAR 0 = Control, BAR 1 = Data (256MB window)
 *
 * Supports two IOCTL interfaces:
 *   - Legacy 'V' magic (vitriol-util compatibility)
 *   - Alka 0xA1 magic (Drop packet execution)
 */

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/init.h>
#include <linux/pci.h>
#include <linux/fs.h>
#include <linux/cdev.h>
#include <linux/device.h>
#include <linux/uaccess.h>
#include <linux/io.h>
#include <linux/dma-mapping.h>
#include <linux/scatterlist.h>
#include <linux/blkdev.h>
#include <linux/fs.h>
#include <linux/timex.h>
#include <linux/delay.h>
#include <linux/crc32.h>
#include <linux/vmalloc.h>
#include <linux/kallsyms.h>
#include <linux/kprobes.h>

/* Forward declare nvidia P2P types (minimal compatible subset) */
enum nvidia_p2p_page_size_type {
    NVIDIA_P2P_PAGE_SIZE_4KB = 0,
    NVIDIA_P2P_PAGE_SIZE_64KB,
    NVIDIA_P2P_PAGE_SIZE_128KB,
    NVIDIA_P2P_PAGE_SIZE_COUNT
};

struct nvidia_p2p_page {
    uint64_t physical_address;
    uint32_t registers[6];
};

struct nvidia_p2p_page_table {
    uint32_t version;
    uint32_t page_size;
    struct nvidia_p2p_page **pages;
    uint32_t entries;
    uint8_t *gpu_uuid;
};

#include "vitriol_alka_kernel.h"

#define DEVICE_NAME "vitriol"
#define CLASS_NAME "vitriol_class"

/* GPU Device IDs */
#define NVIDIA_VENDOR 0x10de
#define GPU_DEVICE_1070TI 0x1b82
#define GPU_DEVICE_960    0x1401

/* BAR Sizes */
#define BAR_0_SIZE 0x1000000    /* 16MB - Control Plane */
#define BAR_1_SIZE 0x10000000  /* 256MB - Data Plane (VRAM Window) */

/* Vessel tracking */
static char *vitriol_devnode(const struct device *dev, umode_t *mode)
{
    if (mode)
        *mode = 0666;
    return NULL;
}

#define MAX_VESSELS 8
#define MAX_EXEC_HISTORY 64

struct vitriol_vessel {
    __u16  id;
    __u8   claimed;
    __u64  pci_addr;
    __u64  bar0_addr;
    __u64  bar1_addr;
    __u64  bar1_offset;    /* Current SHIFT window offset */
    char   name[32];
};

/* VITRIOL State */
static struct {
    struct pci_dev *pci_dev;
    void __iomem *bar0;       /* Control plane (read-only) */
    void __iomem *bar1;       /* Data plane (DMA target) */
    void *dma_buffer;
    size_t dma_size;
    bool mapped;
    dev_t dev_num;
    struct cdev vitriol_cdev;
    struct class *class;
    struct device *device;

    /* Alka state */
    struct vitriol_vial current_vial;
    struct vitriol_vessel vessels[MAX_VESSELS];
    int vessel_count;
    bool vial_set;

    /* Execution tracking */
    struct vitriol_result last_result;
    int exec_history_count;
    struct {
        __u8 opcode;
        int success;
        __u64 cycles;
    } exec_history[MAX_EXEC_HISTORY];

    /* Rollback stack */
    struct vitriol_azoth rollback_stack[MAX_EXEC_HISTORY];
    int rollback_count;

    /* Source file (GGUF on NVMe) */
    struct file *source_file;
    bool source_set;

    /* FLOW buffer tracking (for userspace BAR1 write) */
    __u32 last_flow_size;

    /* BAR1 physical address (for userspace mmap) */
    phys_addr_t bar1_phys;
    size_t bar1_size;

    /* Fallback buffer when PCI probe doesn't run (nvidia owns GPU) */
    void *fallback_buffer;

    /* nvidia P2P cooperative DMA (Level 3) */
    bool nvidia_p2p_available;
    int (*nvidia_p2p_get_pages)(uint64_t, uint32_t, uint64_t, uint64_t,
                                 struct nvidia_p2p_page_table **,
                                 void (*)(void *), void *);
    int (*nvidia_p2p_put_pages)(uint64_t, uint32_t, uint64_t,
                                 struct nvidia_p2p_page_table *);
    int (*nvidia_p2p_free_page_table)(struct nvidia_p2p_page_table *);
} vitriol_state;

/* Device Node Operations */
static int vitriol_open(struct inode *inode, struct file *filp);
static int vitriol_release(struct inode *inode, struct file *filp);
static ssize_t vitriol_read(struct file *filp, char __user *buf, size_t count, loff_t *ppos);
static ssize_t vitriol_write(struct file *filp, const char __user *buf, size_t count, loff_t *ppos);
static long vitriol_ioctl(struct file *filp, unsigned int cmd, unsigned long arg);

/* Source file helpers */
static int handle_set_source(struct vitriol_source __user *arg);
static ssize_t read_source_file(loff_t offset, void *buf, size_t count);

static int vitriol_mmap(struct file *filp, struct vm_area_struct *vma);

static const struct file_operations vitriol_fops = {
    .owner          = THIS_MODULE,
    .open           = vitriol_open,
    .release        = vitriol_release,
    .read           = vitriol_read,
    .write          = vitriol_write,
    .unlocked_ioctl = vitriol_ioctl,
    .mmap           = vitriol_mmap,
};

/* Legacy DMA Transfer Request Structure */
struct vitriol_dma_request {
    __u64 gpu_offset;     /* Offset into GPU VRAM (BAR1) */
    __u64 file_offset;    /* Offset in model file on NVMe */
    __u64 size;           /* Transfer size in bytes */
    __u32 direction;      /* 0 = read from NVMe, 1 = write to NVMe */
};

/* Legacy IOCTL Commands ('V' magic) */
#define VITRIOL_IOC_MAGIC_LEGACY 'V'
#define VITRIOL_IOC_MAP_BAR      _IO(VITRIOL_IOC_MAGIC_LEGACY, 0)
#define VITRIOL_IOC_UNMAP_BAR    _IO(VITRIOL_IOC_MAGIC_LEGACY, 1)
#define VITRIOL_IOC_DMA_TRANSFER _IOW(VITRIOL_IOC_MAGIC_LEGACY, 2, struct vitriol_dma_request)
#define VITRIOL_IOC_GET_BAR_ADDR _IOR(VITRIOL_IOC_MAGIC_LEGACY, 3, __u64)
#define VITRIOL_IOC_CHECK_STATUS _IO(VITRIOL_IOC_MAGIC_LEGACY, 4)

MODULE_LICENSE("GPL");
MODULE_AUTHOR("VITRIOL Project");
MODULE_DESCRIPTION("NVMe to GPU Direct Memory Access Module with Alka ABI support");
MODULE_VERSION("0.2");

/* ── Utility Functions ─────────────────────────────────────────── */

static __u64 get_cycles_now(void)
{
    struct timespec64 ts;
    ktime_get_ts64(&ts);
    return (ts.tv_sec * 1000000000ULL) + ts.tv_nsec;
}

static struct vitriol_vessel *find_vessel(__u16 id)
{
    int i;
    for (i = 0; i < vitriol_state.vessel_count; i++) {
        if (vitriol_state.vessels[i].id == id && vitriol_state.vessels[i].claimed)
            return &vitriol_state.vessels[i];
    }
    return NULL;
}

static struct vitriol_vessel *claim_vessel(__u16 id, const char *name)
{
    if (vitriol_state.vessel_count >= MAX_VESSELS) {
        pr_err("VITRIOL: Max vessels reached (%d)\n", MAX_VESSELS);
        return NULL;
    }
    if (find_vessel(id)) {
        pr_warn("VITRIOL: Vessel 0x%x already claimed\n", id);
        return find_vessel(id);
    }

    struct vitriol_vessel *v = &vitriol_state.vessels[vitriol_state.vessel_count];
    v->id = id;
    v->claimed = 1;
    v->bar1_offset = 0;
    strncpy(v->name, name, sizeof(v->name) - 1);
    v->name[sizeof(v->name) - 1] = '\0';

    /* Auto-detect: if this is our GPU, map BARs */
    if (vitriol_state.bar1 && (id == 0x0001 || id == 0x0010)) {
        v->pci_addr = vitriol_state.pci_dev ?
            ((u64)vitriol_state.pci_dev->device << 16) | vitriol_state.pci_dev->vendor : 0;
        v->bar0_addr = (unsigned long)vitriol_state.bar0;
        v->bar1_addr = (unsigned long)vitriol_state.bar1;
    }

    vitriol_state.vessel_count++;
    pr_info("VITRIOL: CLAIM vessel 0x%x (%s)\n", id, name);
    return v;
}

static void record_execution(__u8 opcode, int success, __u64 cycles)
{
    int idx = vitriol_state.exec_history_count % MAX_EXEC_HISTORY;
    vitriol_state.exec_history[idx].opcode = opcode;
    vitriol_state.exec_history[idx].success = success;
    vitriol_state.exec_history[idx].cycles = cycles;
    vitriol_state.exec_history_count++;
}

/* ── Opcode Handlers ───────────────────────────────────────────── */

static int handle_claim(const struct vitriol_drop *drop)
{
    const char *name;
    switch (drop->vessel_id) {
    case 0x0001: name = "GPU_MAIN"; break;
    case 0x0002: name = "NVME_BOOT"; break;
    case 0x0003: name = "CPU_CORE_0"; break;
    default:     name = "UNKNOWN"; break;
    }
    return claim_vessel(drop->vessel_id, name) ? 0 : -ENODEV;
}

static int handle_limit(const struct vitriol_drop *drop)
{
    __u8 thermal = (__u8)(drop->src_addr & 0xFF);
    pr_info("VITRIOL: LIMIT thermal halt = %u°C\n", thermal);
    vitriol_state.current_vial.thermal_halt = thermal;
    return 0;
}

static int handle_shift(const struct vitriol_drop *drop)
{
    struct vitriol_vessel *v = find_vessel(drop->vessel_id);
    if (!v) {
        pr_err("VITRIOL: SHIFT on unclaimed vessel 0x%x\n", drop->vessel_id);
        return -ENODEV;
    }

    __u64 new_offset = drop->src_addr;

    /* Validate against vial aperture */
    if (vitriol_state.vial_set) {
        if (new_offset + vitriol_state.current_vial.aperture_size > BAR_1_SIZE) {
            pr_err("VITRIOL: SHIFT exceeds BAR1 aperture (offset=0x%llx, size=%llu)\n",
                   new_offset, vitriol_state.current_vial.aperture_size);
            return -EINVAL;
        }
    }

    v->bar1_offset = new_offset;
    pr_info("VITRIOL: SHIFT BAR1 window to offset 0x%llx\n", new_offset);

    /* In a real implementation, this would remap the BAR1 window.
     * On Pascal, BAR1 is a fixed aperture — we track the offset
     * and use it for DMA source/destination calculations. */
    return 0;
}

static int handle_flow_cooperative(const struct vitriol_drop *drop,
                                   struct vitriol_vessel *v);

static int handle_flow(const struct vitriol_drop *drop)
{
    struct vitriol_vessel *v = find_vessel(drop->vessel_id);
    if (!v) {
        pr_err("VITRIOL: FLOW on unclaimed vessel 0x%x\n", drop->vessel_id);
        return -ENODEV;
    }

    if (drop->size == 0) {
        pr_err("VITRIOL: FLOW zero-size transfer rejected\n");
        return -EINVAL;
    }

    /* Validate against vial aperture */
    if (vitriol_state.vial_set && drop->size > vitriol_state.current_vial.aperture_max) {
        pr_err("VITRIOL: FLOW size %u exceeds aperture max %llu\n",
               drop->size, vitriol_state.current_vial.aperture_max);
        return -EINVAL;
    }

    __u64 src = drop->src_addr;
    __u64 dst = drop->dst_addr + v->bar1_offset;
    __u32 size = drop->size;
    __u32 total_transferred = 0;

    /* Cooperative nvidia P2P path (preferred — no BAR1 needed) */
    if (vitriol_state.vial_set && vitriol_state.current_vial.cooperative &&
        vitriol_state.nvidia_p2p_available &&
        vitriol_state.current_vial.gpu_va != 0) {
        return handle_flow_cooperative(drop, v);
    }

    pr_info("VITRIOL: FLOW 0x%llx → 0x%llx (%u bytes)\n", src, dst, size);

    if (!vitriol_state.source_set || !vitriol_state.source_file) {
        pr_warn("VITRIOL: FLOW no source file set, zeroing buffer\n");
        if (!vitriol_state.bar1) {
            pr_info("VITRIOL: FLOW skipped (BAR1 not mapped)\n");
            return 0;
        }
        if (vitriol_state.dma_size > 0 && size > vitriol_state.dma_size)
            size = vitriol_state.dma_size;
        memset(vitriol_state.dma_buffer, 0, size);
        memcpy_toio(vitriol_state.bar1 + dst, vitriol_state.dma_buffer, size);
        wmb();
        return 0;
    }

    /* Use fallback buffer when PCI probe didn't allocate DMA buffer */
    void *read_buf = vitriol_state.dma_buffer;
    if (!read_buf)
        read_buf = vitriol_state.fallback_buffer;
    if (!read_buf) {
        pr_err("VITRIOL: FLOW no buffer available\n");
        return -ENOMEM;
    }

    while (total_transferred < size) {
        __u32 chunk = size - total_transferred;
        if (vitriol_state.dma_size > 0 && chunk > vitriol_state.dma_size)
            chunk = vitriol_state.dma_size;

        ssize_t nread = read_source_file(src + total_transferred,
                                          read_buf, chunk);
        if (nread < 0) {
            pr_err("VITRIOL: FLOW read failed at offset 0x%llx: %zd\n",
                   src + total_transferred, nread);
            return (int)nread;
        }
        if (nread == 0) {
            pr_warn("VITRIOL: FLOW EOF at offset 0x%llx (transferred %u/%u)\n",
                    src + total_transferred, total_transferred, size);
            break;
        }

        if (vitriol_state.bar1) {
            memcpy_toio(vitriol_state.bar1 + dst + total_transferred,
                        read_buf, nread);
            wmb();
        }
        total_transferred += nread;
    }

    vitriol_state.last_flow_size = total_transferred;
    pr_info("VITRIOL: FLOW transferred %u/%u bytes (buffer ready for userspace)\n", total_transferred, size);

    if (vitriol_state.bar1 && total_transferred > 0) {
        __u32 verify_buf[4];
        memcpy_fromio(verify_buf, vitriol_state.bar1 + dst, min(16u, total_transferred));
        pr_info("VITRIOL: FLOW readback [0]=0x%08x [1]=0x%08x [2]=0x%08x [3]=0x%08x\n",
                verify_buf[0], verify_buf[1], verify_buf[2], verify_buf[3]);
    }

    return 0;
}

static int handle_fence(const struct vitriol_drop *drop)
{
    __u64 expected = drop->dst_addr;

    if (!vitriol_state.bar0) {
        pr_info("VITRIOL: FENCE metapage==%llu (simulated, no BAR0)\n", expected);
        return 0;
    }

    int retries = 1000;
    pr_info("VITRIOL: FENCE waiting for metapage == %llu\n", expected);

    while (retries-- > 0) {
        __u32 status = readl(vitriol_state.bar0);
        if (status >= expected)
            return 0;
        udelay(100);
    }

    pr_warn("VITRIOL: FENCE timeout (metapage != %llu)\n", expected);
    return -ETIMEDOUT;
}

static int handle_sync(const struct vitriol_drop *drop)
{
    pr_info("VITRIOL: SYNC memory barrier\n");
    wmb();
    return 0;
}

static int handle_signal(const struct vitriol_drop *drop)
{
    pr_info("VITRIOL: SIGNAL 0x%llx\n", drop->src_addr);
    /*
     * In production: trigger GPU kernel launch via
     * writing to the GPU command submission ring buffer.
     */
    return 0;
}

static int handle_sense(const struct vitriol_drop *drop)
{
    pr_info("VITRIOL: SENSE vessel 0x%x\n", drop->vessel_id);
    return 0;
}

static int handle_watch(const struct vitriol_drop *drop)
{
    pr_info("VITRIOL: WATCH monitoring enabled\n");
    return 0;
}

static int handle_refract(const struct vitriol_drop *drop)
{
    pr_info("VITRIOL: REFRACT slice 0x%llx → 0x%llx (%u bytes)\n",
            drop->src_addr, drop->dst_addr, drop->size);
    return 0;
}

static int handle_dry_run(const struct vitriol_drop *drop)
{
    pr_info("VITRIOL: DRY_RUN opcode 0x%x vessel 0x%x\n",
            drop->op_code, drop->vessel_id);
    return 0;
}

static int handle_bind_device(struct vitriol_bind_req __user *arg)
{
    pr_warn("VITRIOL: BIND not supported in kernel (use userspace --bind or --cooperative)\n");
    return -ENOTSUPP;
}

static int handle_set_source(struct vitriol_source __user *arg)
{
    struct vitriol_source src;
    struct file *filp;

    if (copy_from_user(&src, arg, sizeof(src)))
        return -EFAULT;

    if (src.fd < 0)
        return -EBADF;

    if (vitriol_state.source_file) {
        filp_close(vitriol_state.source_file, NULL);
        vitriol_state.source_file = NULL;
    }

    filp = fget(src.fd);
    if (!filp) {
        pr_err("VITRIOL: SET_SOURCE invalid fd %d\n", src.fd);
        return -EBADF;
    }

    vitriol_state.source_file = filp;
    vitriol_state.source_set = true;

    pr_info("VITRIOL: SET_SOURCE fd=%d (file=%p, size=%lld, mode=0x%x, flags=0x%x)\n",
            src.fd, filp, file_inode(filp)->i_size,
            filp->f_mode, filp->f_flags);
    return 0;
}

static ssize_t read_source_file(loff_t offset, void *buf, size_t count)
{
    ssize_t ret;

    if (!vitriol_state.source_set || !vitriol_state.source_file)
        return -ENODEV;

    loff_t pos = vfs_llseek(vitriol_state.source_file, offset, SEEK_SET);
    if (pos != offset)
        return (ssize_t)(pos < 0 ? pos : -EIO);

    ret = kernel_read(vitriol_state.source_file, buf, count, NULL);
    return ret;
}

static int handle_unknown(__u8 opcode)
{
    if (!vitriol_state.bar0) {
        pr_info("VITRIOL: Unknown opcode 0x%x (simulated, accepting)\n", opcode);
        return 0;
    }
    pr_warn("VITRIOL: Unknown opcode 0x%x\n", opcode);
    return -ENOSYS;
}

static int execute_drop(const struct vitriol_drop *drop)
{
    int ret;
    __u64 start_cycles = get_cycles_now();

    switch (drop->op_code) {
    case OP_CLAIM:    ret = handle_claim(drop); break;
    case OP_LIMIT:    ret = handle_limit(drop); break;
    case OP_SHIFT:    ret = handle_shift(drop); break;
    case OP_FLOW:     ret = handle_flow(drop); break;
    case OP_FENCE:    ret = handle_fence(drop); break;
    case OP_SYNC:     ret = handle_sync(drop); break;
    case OP_SIGNAL:   ret = handle_signal(drop); break;
    case OP_SENSE:    ret = handle_sense(drop); break;
    case OP_WATCH:    ret = handle_watch(drop); break;
    case OP_REFRACT:  ret = handle_refract(drop); break;
    case OP_DRY_RUN:  ret = handle_dry_run(drop); break;
    default:          ret = handle_unknown(drop->op_code); break;
    }

    record_execution(drop->op_code, ret == 0, get_cycles_now() - start_cycles);
    return ret;
}

/* ── Rollback via Azoth packets ────────────────────────────────── */

static void execute_rollback(void)
{
    int i;
    pr_info("VITRIOL: Executing rollback (%d azoth packets)\n",
            vitriol_state.rollback_count);

    for (i = vitriol_state.rollback_count - 1; i >= 0; i--) {
        struct vitriol_azoth *az = &vitriol_state.rollback_stack[i];
        struct vitriol_drop drop = {
            .op_code = az->op_code,
            .flags = az->flags,
            .vessel_id = az->vessel_id,
            .src_addr = az->src_addr,
            .dst_addr = az->dst_addr,
            .size = az->size,
            .reserved = az->reserved,
            .crc = az->crc,
        };
        execute_drop(&drop);
    }
}

/* ── GPU Acquisition (side-load alongside nvidia) ─────────────── */

static int vitriol_acquire_gpu(void)
{
    struct pci_dev *pdev;

    pdev = pci_get_device(NVIDIA_VENDOR, GPU_DEVICE_960, NULL);
    if (!pdev) {
        pdev = pci_get_device(NVIDIA_VENDOR, GPU_DEVICE_1070TI, NULL);
        if (!pdev) {
            pr_err("VITRIOL: No supported GPU found\n");
            return -ENODEV;
        }
    }

    pr_info("VITRIOL: Found GPU %04x:%04x\n",
            pdev->vendor, pdev->device);

    vitriol_state.bar0 = pci_iomap(pdev, 0, BAR_0_SIZE);
    if (!vitriol_state.bar0) {
        pr_err("VITRIOL: Failed to map BAR 0\n");
        pci_dev_put(pdev);
        return -ENOMEM;
    }
    pr_info("VITRIOL: BAR 0 (Control) mapped at %px\n", vitriol_state.bar0);

    /* Store BAR1 physical address for userspace mmap (kernel ioremap conflicts with nvidia) */
    vitriol_state.bar1_phys = pci_resource_start(pdev, 1);
    vitriol_state.bar1_size = BAR_1_SIZE;
    vitriol_state.bar1 = NULL;
    pr_info("VITRIOL: BAR 1 (Data) phys=0x%llx size=%zu (userspace mmap)\n",
            (unsigned long long)vitriol_state.bar1_phys, vitriol_state.bar1_size);

    vitriol_state.pci_dev = pdev;
    vitriol_state.mapped = true;

    claim_vessel(0x0001, "GPU_MAIN");

    pr_info("VITRIOL: GPU acquired successfully (side-load)\n");
    return 0;
}

static int vitriol_map_bar1(void)
{
    struct pci_dev *pdev = vitriol_state.pci_dev;
    if (!pdev)
        return -ENODEV;
    if (vitriol_state.bar1)
        return 0;

    vitriol_state.bar1 = pci_iomap_wc(pdev, 1, BAR_1_SIZE);
    if (!vitriol_state.bar1) {
        pr_warn("VITRIOL: BAR1 WC failed, trying UC\n");
        vitriol_state.bar1 = pci_iomap(pdev, 1, BAR_1_SIZE);
    }
    if (!vitriol_state.bar1) {
        pr_err("VITRIOL: Failed to map BAR1 (nvidia still owns device?)\n");
        return -EBUSY;
    }
    pr_info("VITRIOL: BAR1 mapped at %px\n", vitriol_state.bar1);
    return 0;
}

static void vitriol_release_gpu(void)
{
    if (!vitriol_state.mapped || !vitriol_state.pci_dev)
        return;

    if (vitriol_state.source_file) {
        filp_close(vitriol_state.source_file, NULL);
        vitriol_state.source_file = NULL;
        vitriol_state.source_set = false;
    }

    if (vitriol_state.bar1)
        pci_iounmap(vitriol_state.pci_dev, vitriol_state.bar1);
    if (vitriol_state.bar0)
        pci_iounmap(vitriol_state.pci_dev, vitriol_state.bar0);

    pci_dev_put(vitriol_state.pci_dev);
    vitriol_state.pci_dev = NULL;
    vitriol_state.bar0 = NULL;
    vitriol_state.bar1 = NULL;
    vitriol_state.mapped = false;

    pr_info("VITRIOL: GPU released\n");
}

/* ── File Operations ───────────────────────────────────────────── */

static int vitriol_open(struct inode *inode, struct file *filp)
{
    pr_info("VITRIOL: Device opened\n");
    return 0;
}

static int vitriol_release(struct inode *inode, struct file *filp)
{
    if (vitriol_state.source_file) {
        filp_close(vitriol_state.source_file, NULL);
        vitriol_state.source_file = NULL;
        vitriol_state.source_set = false;
    }
    pr_info("VITRIOL: Device closed\n");
    return 0;
}

static ssize_t vitriol_read(struct file *filp, char __user *buf,
                            size_t count, loff_t *ppos)
{
    pr_info("VITRIOL: Read request (count=%zu)\n", count);
    return 0;
}

static int vitriol_mmap(struct file *filp, struct vm_area_struct *vma)
{
    unsigned long size = vma->vm_end - vma->vm_start;
    unsigned long pfn = vitriol_state.bar1_phys >> PAGE_SHIFT;
    int ret;

    if (size > vitriol_state.bar1_size)
        return -EINVAL;

    /* Try write-combining first, fall back to uncached */
    vma->vm_page_prot = pgprot_writecombine(vma->vm_page_prot);
    ret = io_remap_pfn_range(vma, vma->vm_start, pfn, size, vma->vm_page_prot);
    if (ret) {
        pr_warn("VITRIOL: mmap WC failed (%d), trying UC\n", ret);
        vma->vm_page_prot = pgprot_noncached(vma->vm_page_prot);
        ret = io_remap_pfn_range(vma, vma->vm_start, pfn, size, vma->vm_page_prot);
    }
    if (ret) {
        pr_err("VITRIOL: mmap BAR1 failed: %d\n", ret);
        return ret;
    }

    pr_info("VITRIOL: mmap BAR1 at phys=0x%llx size=%lu\n",
            (unsigned long long)vitriol_state.bar1_phys, size);
    return 0;
}

static ssize_t vitriol_write(struct file *filp, const char __user *buf,
                             size_t count, loff_t *ppos)
{
    pr_info("VITRIOL: Write request (count=%zu)\n", count);
    return count;
}

/* ── IOCTL Handler ─────────────────────────────────────────────── */

static long vitriol_ioctl(struct file *filp, unsigned int cmd, unsigned long arg)
{
    int ret = 0;

    pr_info("VITRIOL: ioctl called cmd=0x%x type=0x%x nr=%u size=%u\n",
            cmd, _IOC_TYPE(cmd), _IOC_NR(cmd), _IOC_SIZE(cmd));

    /* Check for Alka 0xA1 magic */
    if (_IOC_TYPE(cmd) == VITRIOL_IOC_MAGIC) {
        pr_info("VITRIOL: Alka 0xA1 magic detected\n");
        switch (cmd) {
        case VITRIOL_IOC_SET_VIAL: {
            struct vitriol_vial vial;
            if (copy_from_user(&vial, (void __user *)arg, sizeof(vial)))
                return -EFAULT;
            vitriol_state.current_vial = vial;
            vitriol_state.vial_set = true;
            pr_info("VITRIOL: Vial set (aperture=%llu, thermal_halt=%u)\n",
                    vial.aperture_size, vial.thermal_halt);
            break;
        }

        case VITRIOL_IOC_EXECUTE: {
            struct vitriol_drop drop;
            __u64 start;
            if (copy_from_user(&drop, (void __user *)arg, sizeof(drop)))
                return -EFAULT;

            /* CRC validation */
            __u32 expected_crc = vitriol_compute_drop_crc(&drop);
            if (drop.crc != 0 && drop.crc != expected_crc) {
                pr_err("VITRIOL: Drop CRC mismatch (expected=0x%x, got=0x%x)\n",
                       expected_crc, drop.crc);
                return -EINVAL;
            }

            start = get_cycles_now();
            ret = execute_drop(&drop);

            /* Push to rollback stack */
            if (vitriol_state.rollback_count < MAX_EXEC_HISTORY) {
                struct vitriol_azoth *az = &vitriol_state.rollback_stack[vitriol_state.rollback_count++];
                az->op_code = drop.op_code;
                az->flags = drop.flags;
                az->vessel_id = drop.vessel_id;
                az->src_addr = drop.src_addr;
                az->dst_addr = drop.dst_addr;
                az->size = drop.size;
                az->reserved = drop.reserved;
                az->crc = drop.crc;
            }

            vitriol_state.last_result.success = (ret == 0) ? 1 : 0;
            vitriol_state.last_result.cycles_spent = get_cycles_now() - start;
            break;
        }

        case VITRIOL_IOC_VALIDATE: {
            struct vitriol_drop drop;
            if (copy_from_user(&drop, (void __user *)arg, sizeof(drop)))
                return -EFAULT;

            __u32 expected_crc = vitriol_compute_drop_crc(&drop);
            if (drop.crc != 0 && drop.crc != expected_crc) {
                pr_err("VITRIOL: Validate CRC mismatch\n");
                return -EINVAL;
            }

            /* Validate against vial if set */
            if (vitriol_state.vial_set) {
                if (drop.size > vitriol_state.current_vial.aperture_max) {
                    pr_err("VITRIOL: Validate: size exceeds aperture\n");
                    return -EINVAL;
                }
            }
            break;
        }

        case VITRIOL_IOC_GET_RESULT: {
            if (copy_to_user((void __user *)arg, &vitriol_state.last_result,
                            sizeof(vitriol_state.last_result)))
                return -EFAULT;
            break;
        }

        case VITRIOL_IOC_BIND_DEVICE: {
            ret = handle_bind_device((struct vitriol_bind_req __user *)arg);
            break;
        }

        case VITRIOL_IOC_STREAM: {
            struct vitriol_stream_req req;
            struct vitriol_drop *drops;
            int i;

            if (copy_from_user(&req, (void __user *)arg, sizeof(req)))
                return -EFAULT;

            if (req.drop_count == 0 || req.drop_count > MAX_EXEC_HISTORY) {
                pr_err("VITRIOL: Stream drop_count out of range (%u)\n", req.drop_count);
                return -EINVAL;
            }

            drops = memdup_user((void __user *)req.drops_ptr,
                               req.drop_count * sizeof(struct vitriol_drop));
            if (IS_ERR(drops))
                return PTR_ERR(drops);

            /* Load vial if provided */
            if (req.vial_ptr) {
                struct vitriol_vial vial;
                if (copy_from_user(&vial, (void __user *)req.vial_ptr, sizeof(vial))) {
                    kfree(drops);
                    return -EFAULT;
                }
                vitriol_state.current_vial = vial;
                vitriol_state.vial_set = true;
            }

            /* Reset rollback stack */
            vitriol_state.rollback_count = 0;

            __u64 total_bytes = 0;
            __u64 start = get_cycles_now();

            for (i = 0; i < req.drop_count; i++) {
                ret = execute_drop(&drops[i]);
                if (ret != 0) {
                    pr_err("VITRIOL: Stream failed at drop %d (opcode 0x%x)\n",
                           i, drops[i].op_code);

                    /* Execute rollback */
                    execute_rollback();

                    vitriol_state.last_result.success = 0;
                    snprintf(vitriol_state.last_result.error_message,
                            sizeof(vitriol_state.last_result.error_message),
                            "Drop %d failed: opcode 0x%x", i, drops[i].op_code);
                    goto stream_done;
                }
                total_bytes += drops[i].size;
            }

            vitriol_state.last_result.success = 1;
            vitriol_state.last_result.bytes_transferred = total_bytes;
            vitriol_state.last_result.cycles_spent = get_cycles_now() - start;

stream_done:
            kfree(drops);
            break;
        }

        case VITRIOL_IOC_SET_SOURCE: {
            ret = handle_set_source((struct vitriol_source __user *)arg);
            break;
        }

        case VITRIOL_IOC_READ_BAR1: {
            struct vitriol_bar1_read req;
            if (copy_from_user(&req, (void __user *)arg, sizeof(req))) {
                ret = -EFAULT;
                break;
            }
            if (!vitriol_state.bar1) {
                pr_err("VITRIOL: READ_BAR1 but BAR1 not mapped\n");
                ret = -ENODEV;
                break;
            }
            if (req.bar1_offset + req.size > BAR_1_SIZE) {
                pr_err("VITRIOL: READ_BAR1 offset %llu + size %llu exceeds BAR1\n",
                       req.bar1_offset, req.size);
                ret = -EINVAL;
                break;
            }
            void *read_buf = vitriol_state.dma_buffer ?: vitriol_state.fallback_buffer;
            if (!read_buf) {
                pr_err("VITRIOL: READ_BAR1 no staging buffer\n");
                ret = -ENOMEM;
                break;
            }
            __u32 chunk = req.size;
            if (vitriol_state.dma_size > 0 && chunk > vitriol_state.dma_size)
                chunk = vitriol_state.dma_size;
            memcpy_fromio(read_buf, vitriol_state.bar1 + req.bar1_offset, chunk);
            if (copy_to_user((void __user *)(unsigned long)req.buf, read_buf, chunk)) {
                ret = -EFAULT;
                break;
            }
            pr_info("VITRIOL: READ_BAR1 offset=0x%llx size=%u\n",
                    req.bar1_offset, chunk);
            ret = 0;
            break;
        }

        case VITRIOL_IOC_GET_BAR1_PHYS: {
            struct vitriol_bar1_info info;
            info.phys_addr = vitriol_state.bar1_phys;
            info.size = vitriol_state.bar1_size;
            if (copy_to_user((void __user *)arg, &info, sizeof(info))) {
                ret = -EFAULT;
                break;
            }
            pr_info("VITRIOL: GET_BAR1_PHYS phys=0x%llx size=%llu\n",
                    (unsigned long long)info.phys_addr,
                    (unsigned long long)info.size);
            ret = 0;
            break;
        }

        case VITRIOL_IOC_GET_FLOW_BUF: {
            struct vitriol_flow_buf req;
            if (copy_from_user(&req, (void __user *)arg, sizeof(req))) {
                ret = -EFAULT;
                break;
            }
            void *read_buf = vitriol_state.dma_buffer ?: vitriol_state.fallback_buffer;
            if (!read_buf || vitriol_state.last_flow_size == 0) {
                pr_err("VITRIOL: GET_FLOW_BUF no data available\n");
                ret = -ENODATA;
                break;
            }
            __u32 chunk = vitriol_state.last_flow_size;
            if (chunk > req.size)
                chunk = req.size;
            if (copy_to_user((void __user *)(unsigned long)req.buf, read_buf, chunk)) {
                ret = -EFAULT;
                break;
            }
            pr_info("VITRIOL: GET_FLOW_BUF returned %u bytes\n", chunk);
            ret = 0;
            break;
        }

        default:
            ret = -ENOTTY;
        }
        return ret;
    }

    /* ── Legacy IOCTL Commands ── */
    switch (cmd) {
    case VITRIOL_IOC_GET_BAR_ADDR:
        if (copy_to_user((__u64 __user *)arg,
                        &vitriol_state.bar1, sizeof(__u64)))
            ret = -EFAULT;
        pr_info("VITRIOL: BAR1 address returned to userspace\n");
        break;

    case VITRIOL_IOC_CHECK_STATUS:
        ret = vitriol_state.mapped ? 0 : -ENODEV;
        break;

    case VITRIOL_IOC_MAP_BAR:
        if (!vitriol_state.mapped) {
            pr_warn("VITRIOL: No GPU acquired\n");
            ret = -ENODEV;
        } else if (vitriol_state.bar1) {
            pr_info("VITRIOL: BAR1 already mapped\n");
            ret = 0;
        } else {
            ret = vitriol_map_bar1();
        }
        break;

    case VITRIOL_IOC_UNMAP_BAR:
        if (!vitriol_state.mapped) {
            pr_warn("VITRIOL: BARs not mapped\n");
            ret = -ENODEV;
        } else {
            pr_info("VITRIOL: UNMAP_BAR requested (use rmmod instead)\n");
            ret = -ENOSYS;
        }
        break;

    case VITRIOL_IOC_DMA_TRANSFER: {
        struct vitriol_dma_request req;
        if (copy_from_user(&req, (void __user *)arg, sizeof(req)))
            return -EFAULT;
        pr_info("VITRIOL: Legacy DMA transfer: gpu=0x%llx file=0x%llx size=0x%llx\n",
                req.gpu_offset, req.file_offset, req.size);
        ret = 0;
        break;
    }

    default:
        ret = -ENOTTY;
    }

    return ret;
}

/* ── nvidia P2P cooperative DMA ───────────────────────────────── */

static void nvidia_p2p_free_callback(void *data) __maybe_unused;
static void nvidia_p2p_free_callback(void *data)
{
}

static int handle_flow_cooperative(const struct vitriol_drop *drop,
                                   struct vitriol_vessel *v)
{
    __u64 p2p_token = vitriol_state.current_vial.p2p_token;
    __u32 va_space_token = vitriol_state.current_vial.va_space_token;
    __u64 gpu_va = vitriol_state.current_vial.gpu_va + drop->dst_addr;
    __u32 size = drop->size;
    struct nvidia_p2p_page_table *page_table = NULL;
    void *read_buf;
    int ret, i;

    if (!vitriol_state.nvidia_p2p_available) {
        pr_err("VITRIOL: P2P FLOW but nvidia P2P not available\n");
        return -ENOTSUPP;
    }

    read_buf = vitriol_state.dma_buffer ?: vitriol_state.fallback_buffer;
    if (!read_buf) {
        pr_err("VITRIOL: P2P FLOW no staging buffer\n");
        return -ENOMEM;
    }

    pr_info("VITRIOL: P2P FLOW GPU_VA=0x%llx size=%u p2p_token=0x%llx va_token=%u\n",
            gpu_va, size, p2p_token, va_space_token);

    /* Step 1: Pin VRAM pages via nvidia P2P */
    ret = vitriol_state.nvidia_p2p_get_pages(p2p_token, va_space_token,
                                              gpu_va, size,
                                              &page_table,
                                              nvidia_p2p_free_callback,
                                              NULL);
    if (ret != 0) {
        pr_err("VITRIOL: nvidia_p2p_get_pages failed: %d\n", ret);
        return ret;
    }

    __u32 page_size = 4096;
    switch (page_table->page_size) {
    case NVIDIA_P2P_PAGE_SIZE_4KB:  page_size = 4096; break;
    case NVIDIA_P2P_PAGE_SIZE_64KB: page_size = 65536; break;
    case NVIDIA_P2P_PAGE_SIZE_128KB: page_size = 131072; break;
    }
    pr_info("VITRIOL: P2P got %u pages of %u bytes\n",
            page_table->entries, page_size);

    /* Step 2: DMA from GGUF file into VRAM pages */
    __u32 file_offset = drop->src_addr;
    __u32 transferred = 0;

    for (i = 0; i < page_table->entries && transferred < size; i++) {
        __u64 phys = page_table->pages[i]->physical_address;
        __u32 chunk = size - transferred;
        if (chunk > page_size)
            chunk = page_size;

        ssize_t nread = read_source_file(file_offset + transferred,
                                          read_buf, chunk);
        if (nread <= 0) {
            pr_err("VITRIOL: P2P FLOW read failed at offset %u\n",
                   file_offset + transferred);
            ret = (int)nread;
            goto out;
        }

        void __iomem *vram = ioremap(phys, nread);
        if (!vram) {
            pr_err("VITRIOL: P2P ioremap failed for phys=0x%llx\n", phys);
            ret = -ENOMEM;
            goto out;
        }
        memcpy_toio(vram, read_buf, nread);
        iounmap(vram);

        transferred += nread;
    }

    pr_info("VITRIOL: P2P FLOW transferred %u/%u bytes\n", transferred, size);
    ret = 0;

out:
    vitriol_state.nvidia_p2p_put_pages(0, 0, gpu_va, page_table);
    vitriol_state.nvidia_p2p_free_page_table(page_table);
    return ret;
}

/* Resolve nvidia P2P symbols from the running nvidia.ko module */
static void resolve_nvidia_p2p(void)
{
    struct kprobe kp = { .symbol_name = "nvidia_p2p_get_pages" };
    if (register_kprobe(&kp) == 0) {
        vitriol_state.nvidia_p2p_get_pages = (void *)kp.addr;
        unregister_kprobe(&kp);
    }
    kp = (struct kprobe){ .symbol_name = "nvidia_p2p_put_pages" };
    if (register_kprobe(&kp) == 0) {
        vitriol_state.nvidia_p2p_put_pages = (void *)kp.addr;
        unregister_kprobe(&kp);
    }
    kp = (struct kprobe){ .symbol_name = "nvidia_p2p_free_page_table" };
    if (register_kprobe(&kp) == 0) {
        vitriol_state.nvidia_p2p_free_page_table = (void *)kp.addr;
        unregister_kprobe(&kp);
    }

    if (vitriol_state.nvidia_p2p_get_pages && 
        vitriol_state.nvidia_p2p_put_pages) {
        vitriol_state.nvidia_p2p_available = true;
        pr_info("VITRIOL: nvidia P2P cooperative DMA available\n");
    } else {
        vitriol_state.nvidia_p2p_available = false;
        pr_info("VITRIOL: nvidia P2P not available\n");
    }
}

/* ── Module Init / Exit ────────────────────────────────────────── */

static int __init vitriol_init(void)
{
    int ret;

    pr_info("VITRIOL: Initializing NVMe-to-GPU DMA module v0.2\n");
    pr_info("VITRIOL: Target: GTX 1070 Ti (1b82) / GTX 960 (1401)\n");
    pr_info("VITRIOL: Alka ABI support enabled (0xA1 magic)\n");

    memset(&vitriol_state, 0, sizeof(vitriol_state));
    vitriol_state.dma_size = 1024 * 1024;  /* Default 1MB chunk, PCI probe may override */

    /* Allocate fallback buffer for Direct DMA reads when BAR1 not mapped */
    vitriol_state.fallback_buffer = vmalloc(vitriol_state.dma_size);
    if (!vitriol_state.fallback_buffer) {
        pr_warn("VITRIOL: Failed to allocate fallback buffer\n");
    } else {
        pr_info("VITRIOL: Fallback buffer allocated (%zu bytes)\n",
                vitriol_state.dma_size);
    }

    /* ── nvidia P2P Symbol Resolution ── */
    resolve_nvidia_p2p();

    ret = alloc_chrdev_region(&vitriol_state.dev_num, 0, 1, DEVICE_NAME);
    if (ret < 0) {
        pr_err("VITRIOL: Failed to allocate chrdev region\n");
        return ret;
    }

    cdev_init(&vitriol_state.vitriol_cdev, &vitriol_fops);
    vitriol_state.vitriol_cdev.owner = THIS_MODULE;
    ret = cdev_add(&vitriol_state.vitriol_cdev, vitriol_state.dev_num, 1);
    if (ret < 0) {
        pr_err("VITRIOL: Failed to add cdev\n");
        goto err_cdev;
    }

    vitriol_state.class = class_create(CLASS_NAME);
    if (IS_ERR(vitriol_state.class)) {
        pr_err("VITRIOL: Failed to create class\n");
        ret = PTR_ERR(vitriol_state.class);
        goto err_class;
    }
    vitriol_state.class->devnode = vitriol_devnode;

    vitriol_state.device = device_create(vitriol_state.class, NULL,
                                          vitriol_state.dev_num, NULL,
                                          DEVICE_NAME);
    if (IS_ERR(vitriol_state.device)) {
        pr_err("VITRIOL: Failed to create device\n");
        ret = PTR_ERR(vitriol_state.device);
        goto err_device;
    }

    ret = vitriol_acquire_gpu();
    if (ret < 0) {
        pr_err("VITRIOL: Failed to acquire GPU\n");
        goto err_pci;
    }

    pr_info("VITRIOL: Module loaded successfully\n");
    pr_info("VITRIOL: Device node: /dev/%s\n", DEVICE_NAME);

    return 0;

err_pci:
    device_destroy(vitriol_state.class, vitriol_state.dev_num);
err_device:
    class_destroy(vitriol_state.class);
err_class:
    cdev_del(&vitriol_state.vitriol_cdev);
err_cdev:
    unregister_chrdev_region(vitriol_state.dev_num, 1);
    return ret;
}

static void __exit vitriol_exit(void)
{
    pr_info("VITRIOL: Unloading module\n");

    if (vitriol_state.source_file) {
        filp_close(vitriol_state.source_file, NULL);
        vitriol_state.source_file = NULL;
        vitriol_state.source_set = false;
    }

    if (vitriol_state.fallback_buffer) {
        vfree(vitriol_state.fallback_buffer);
        vitriol_state.fallback_buffer = NULL;
    }

    vitriol_release_gpu();
    device_destroy(vitriol_state.class, vitriol_state.dev_num);
    class_destroy(vitriol_state.class);
    cdev_del(&vitriol_state.vitriol_cdev);
    unregister_chrdev_region(vitriol_state.dev_num, 1);

    pr_info("VITRIOL: Module unloaded\n");
}

module_init(vitriol_init);
module_exit(vitriol_exit);
