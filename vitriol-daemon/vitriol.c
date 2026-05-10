/*
 * VITRIOL Kernel Module - NVMe to GPU Direct Memory Access
 * Based on NVIDIA GPUDirect Storage patterns
 * 
 * Target Hardware: GTX 1070 Ti (Pascal, device ID 1b82)
 * BAR Configuration: BAR 0 = Control, BAR 1 = Data (256MB window)
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

#define DEVICE_NAME "vitriol"
#define CLASS_NAME "vitriol_class"

/* GPU Device IDs */
#define NVIDIA_VENDOR 0x10de
#define GPU_DEVICE_1070TI 0x1b82

/* BAR Sizes */
#define BAR_0_SIZE 0x1000000    /* 16MB - Control Plane */
#define BAR_1_SIZE 0x10000000  /* 256MB - Data Plane (VRAM Window) */

/* VITRIOL State */
static struct {
    struct pci_dev *pci_dev;
    void __iomem *bar0;       /* Control plane (read-only) */
    void __iomem *bar1;       /* Data plane (DMA target) */
    dma_addr_t dma_handle;
    void *dma_buffer;
    size_t dma_size;
    bool mapped;
    dev_t dev_num;
    struct cdev vitriol_cdev;
    struct class *class;
    struct device *device;
} vitriol_state;

/* Device Node Operations */
static int vitriol_open(struct inode *inode, struct file *filp);
static int vitriol_release(struct inode *inode, struct file *filp);
static ssize_t vitriol_read(struct file *filp, char __user *buf, size_t count, loff_t *ppos);
static ssize_t vitriol_write(struct file *filp, const char __user *buf, size_t count, loff_t *ppos);
static long vitriol_ioctl(struct file *filp, unsigned int cmd, unsigned long arg);

static const struct file_operations vitriol_fops = {
    .owner          = THIS_MODULE,
    .open           = vitriol_open,
    .release        = vitriol_release,
    .read           = vitriol_read,
    .write          = vitriol_write,
    .unlocked_ioctl = vitriol_ioctl,
};

/* DMA Transfer Request Structure */
struct vitriol_dma_request {
    __u64 gpu_offset;     /* Offset into GPU VRAM (BAR1) */
    __u64 file_offset;    /* Offset in model file on NVMe */
    __u64 size;           /* Transfer size in bytes */
    __u32 direction;      /* 0 = read from NVMe, 1 = write to NVMe */
};

/* IOCTL Commands */
#define VITRIOL_IOC_MAGIC 'V'
#define VITRIOL_IOC_MAP_BAR      _IO(VITRIOL_IOC_MAGIC, 0)
#define VITRIOL_IOC_UNMAP_BAR    _IO(VITRIOL_IOC_MAGIC, 1)
#define VITRIOL_IOC_DMA_TRANSFER _IOW(VITRIOL_IOC_MAGIC, 2, struct vitriol_dma_request)
#define VITRIOL_IOC_GET_BAR_ADDR _IOR(VITRIOL_IOC_MAGIC, 3, __u64)
#define VITRIOL_IOC_CHECK_STATUS _IO(VITRIOL_IOC_MAGIC, 4)

MODULE_LICENSE("GPL");
MODULE_AUTHOR("VITRIOL Project");
MODULE_DESCRIPTION("NVMe to GPU Direct Memory Access Module");
MODULE_VERSION("0.1");

/*
 * PCI Probe - Find and configure the GPU
 */
static int vitriol_pci_probe(struct pci_dev *dev, const struct pci_device_id *id)
{
    int ret;
    
    pr_info("VITRIOL: Probing GPU device %04x:%04x\n", 
            id->vendor, id->device);
    
    /* Enable PCI device */
    ret = pci_enable_device(dev);
    if (ret < 0) {
        pr_err("VITRIOL: Failed to enable PCI device\n");
        return ret;
    }
    
    /* Request BAR 0 (Control Plane) */
    ret = pci_request_region(dev, 0, "vitriol_bar0");
    if (ret < 0) {
        pr_err("VITRIOL: Failed to request BAR 0\n");
        goto err_disable;
    }
    
    /* Request BAR 1 (Data Plane - VRAM Window) */
    ret = pci_request_region(dev, 1, "vitriol_bar1");
    if (ret < 0) {
        pr_err("VITRIOL: Failed to request BAR 1\n");
        goto err_bar0;
    }
    
    /* Map BAR 0 (Control - read only for monitoring) */
    vitriol_state.bar0 = pci_iomap(dev, 0, BAR_0_SIZE);
    if (!vitriol_state.bar0) {
        pr_err("VITRIOL: Failed to map BAR 0\n");
        goto err_bar1;
    }
    pr_info("VITRIOL: BAR 0 (Control) mapped at %px\n", vitriol_state.bar0);
    
    /* Map BAR 1 (Data - for DMA target) */
    vitriol_state.bar1 = pci_iomap(dev, 1, BAR_1_SIZE);
    if (!vitriol_state.bar1) {
        pr_err("VITRIOL: Failed to map BAR 1\n");
        goto err_bar0_unmap;
    }
    pr_info("VITRIOL: BAR 1 (Data) mapped at %px (256MB VRAM window)\n", 
            vitriol_state.bar1);
    
    /* Enable DMA */
    ret = dma_set_mask_and_coherent(&dev->dev, DMA_BIT_MASK(64));
    if (ret < 0) {
        pr_warn("VITRIOL: 64-bit DMA not available, trying 32-bit\n");
        ret = dma_set_mask_and_coherent(&dev->dev, DMA_BIT_MASK(32));
        if (ret < 0) {
            pr_err("VITRIOL: DMA not available\n");
            goto err_bar1_unmap;
        }
    }
    
    /* Allocate DMA buffer for transfers */
    vitriol_state.dma_size = 1024 * 1024; /* 1MB default buffer */
    vitriol_state.dma_buffer = dma_alloc_coherent(&dev->dev, 
                                                   vitriol_state.dma_size,
                                                   &vitriol_state.dma_handle,
                                                   GFP_KERNEL);
    if (!vitriol_state.dma_buffer) {
        pr_err("VITRIOL: Failed to allocate DMA buffer\n");
        ret = -ENOMEM;
        goto err_bar1_unmap;
    }
    pr_info("VITRIOL: DMA buffer allocated at %pad (size: %zu)\n",
            &vitriol_state.dma_handle, vitriol_state.dma_size);
    
    /* Store PCI device reference */
    vitriol_state.pci_dev = dev;
    vitriol_state.mapped = true;
    
    pr_info("VITRIOL: GPU configured successfully\n");
    pr_info("VITRIOL: Control Plane (BAR 0): %px\n", vitriol_state.bar0);
    pr_info("VITRIOL: Data Plane (BAR 1): %px\n", vitriol_state.bar1);
    
    return 0;

err_bar1_unmap:
    pci_iounmap(dev, vitriol_state.bar1);
err_bar0_unmap:
    pci_iounmap(dev, vitriol_state.bar0);
err_bar1:
    pci_release_region(dev, 1);
err_bar0:
    pci_release_region(dev, 0);
err_disable:
    pci_disable_device(dev);
    return ret;
}

/*
 * PCI Remove - Cleanup
 */
static void vitriol_pci_remove(struct pci_dev *dev)
{
    if (vitriol_state.mapped) {
        if (vitriol_state.dma_buffer) {
            dma_free_coherent(&dev->dev, vitriol_state.dma_size,
                             vitriol_state.dma_buffer, vitriol_state.dma_handle);
        }
        if (vitriol_state.bar1)
            pci_iounmap(dev, vitriol_state.bar1);
        if (vitriol_state.bar0)
            pci_iounmap(dev, vitriol_state.bar0);
        pci_release_region(dev, 1);
        pci_release_region(dev, 0);
        pci_disable_device(dev);
        vitriol_state.mapped = false;
    }
    pr_info("VITRIOL: GPU device removed\n");
}

/* PCI Device ID Table */
static const struct pci_device_id vitriol_pci_table[] = {
    { 0x10de, 0x1b82, PCI_ANY_ID, PCI_ANY_ID, 0, 0, 0 },
    { 0, }
};
MODULE_DEVICE_TABLE(pci, vitriol_pci_table);

static struct pci_driver vitriol_pci_driver = {
    .name     = "vitriol",
    .id_table = vitriol_pci_table,
    .probe    = vitriol_pci_probe,
    .remove   = vitriol_pci_remove,
};

/* File Operations */
static int vitriol_open(struct inode *inode, struct file *filp)
{
    pr_info("VITRIOL: Device opened\n");
    return 0;
}

static int vitriol_release(struct inode *inode, struct file *filp)
{
    pr_info("VITRIOL: Device closed\n");
    return 0;
}

static ssize_t vitriol_read(struct file *filp, char __user *buf, 
                            size_t count, loff_t *ppos)
{
    pr_info("VITRIOL: Read request (count=%zu)\n", count);
    return 0;
}

static ssize_t vitriol_write(struct file *filp, const char __user *buf,
                             size_t count, loff_t *ppos)
{
    pr_info("VITRIOL: Write request (count=%zu)\n", count);
    return count;
}

/*
 * IOCTL - Main control interface
 */
static long vitriol_ioctl(struct file *filp, unsigned int cmd, unsigned long arg)
{
    int ret = 0;
    
    switch (cmd) {
    case VITRIOL_IOC_GET_BAR_ADDR:
        /* Return BAR 1 address to userspace */
        if (copy_to_user((__u64 __user *)arg, 
                        &vitriol_state.bar1, sizeof(__u64)))
            ret = -EFAULT;
        pr_info("VITRIOL: BAR1 address returned to userspace\n");
        break;
        
    case VITRIOL_IOC_CHECK_STATUS:
        /* Return DMA status */
        ret = vitriol_state.mapped ? 0 : -ENODEV;
        break;
        
    default:
        ret = -ENOTTY;
    }
    
    return ret;
}

/*
 * Module Initialization
 */
static int __init vitriol_init(void)
{
    int ret;
    
    pr_info("VITRIOL: Initializing NVMe-to-GPU DMA module\n");
    pr_info("VITRIOL: Target: GTX 1070 Ti (device 1b82)\n");
    pr_info("VITRIOL: Architecture: BAR 0 = Control, BAR 1 = Data (256MB)\n");
    
    /* Initialize state */
    memset(&vitriol_state, 0, sizeof(vitriol_state));
    
    /* Register character device */
    ret = alloc_chrdev_region(&vitriol_state.dev_num, 0, 1, DEVICE_NAME);
    if (ret < 0) {
        pr_err("VITRIOL: Failed to allocate chrdev region\n");
        return ret;
    }
    pr_info("VITRIOL: Device number allocated: %d:%d\n",
            MAJOR(vitriol_state.dev_num), MINOR(vitriol_state.dev_num));
    
    /* Initialize and add cdev */
    cdev_init(&vitriol_state.vitriol_cdev, &vitriol_fops);
    vitriol_state.vitriol_cdev.owner = THIS_MODULE;
    ret = cdev_add(&vitriol_state.vitriol_cdev, vitriol_state.dev_num, 1);
    if (ret < 0) {
        pr_err("VITRIOL: Failed to add cdev\n");
        goto err_cdev;
    }
    
    /* Create device class */
    vitriol_state.class = class_create(CLASS_NAME);
    if (IS_ERR(vitriol_state.class)) {
        pr_err("VITRIOL: Failed to create class\n");
        ret = PTR_ERR(vitriol_state.class);
        goto err_class;
    }
    
    /* Create device node */
    vitriol_state.device = device_create(vitriol_state.class, NULL,
                                          vitriol_state.dev_num, NULL,
                                          DEVICE_NAME);
    if (IS_ERR(vitriol_state.device)) {
        pr_err("VITRIOL: Failed to create device\n");
        ret = PTR_ERR(vitriol_state.device);
        goto err_device;
    }
    
    /* Register PCI driver */
    ret = pci_register_driver(&vitriol_pci_driver);
    if (ret < 0) {
        pr_err("VITRIOL: Failed to register PCI driver\n");
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

/*
 * Module Cleanup
 */
static void __exit vitriol_exit(void)
{
    pr_info("VITRIOL: Unloading module\n");
    
    pci_unregister_driver(&vitriol_pci_driver);
    device_destroy(vitriol_state.class, vitriol_state.dev_num);
    class_destroy(vitriol_state.class);
    cdev_del(&vitriol_state.vitriol_cdev);
    unregister_chrdev_region(vitriol_state.dev_num, 1);
    
    pr_info("VITRIOL: Module unloaded\n");
}

module_init(vitriol_init);
module_exit(vitriol_exit);