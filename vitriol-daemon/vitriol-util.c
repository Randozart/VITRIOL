/*
 * VITRIOL Userspace Utility
 * Interface with the VITRIOL kernel module
 * 
 * Usage: ./vitriol-util [command]
 *   status    - Check module status
 *   bar1      - Get BAR1 address
 *   transfer  - Trigger DMA transfer (requires root)
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>
#include <stdint.h>

#define DEVICE_PATH "/dev/vitriol"

/* IOCTL Commands (must match kernel module) */
#define VITRIOL_IOC_MAGIC 'V'
#define VITRIOL_IOC_MAP_BAR      _IO(VITRIOL_IOC_MAGIC, 0)
#define VITRIOL_IOC_UNMAP_BAR    _IO(VITRIOL_IOC_MAGIC, 1)
#define VITRIOL_IOC_GET_BAR_ADDR _IOR(VITRIOL_IOC_MAGIC, 3, uint64_t)
#define VITRIOL_IOC_CHECK_STATUS _IO(VITRIOL_IOC_MAGIC, 4)

typedef struct {
    uint64_t gpu_offset;
    uint64_t file_offset;
    uint64_t size;
    uint32_t direction;
} vitriol_transfer_t;

#define VITRIOL_IOC_DMA_TRANSFER _IOW(VITRIOL_IOC_MAGIC, 2, vitriol_transfer_t)

int main(int argc, char *argv[])
{
    int fd;
    int ret;
    
    /* Open device */
    fd = open(DEVICE_PATH, O_RDWR);
    if (fd < 0) {
        fprintf(stderr, "Error: Cannot open %s (need root?)\n", DEVICE_PATH);
        fprintf(stderr, "Try: sudo ./vitriol-util status\n");
        return 1;
    }
    
    if (argc < 2) {
        printf("Usage: %s [status|bar1|transfer]\n", argv[0]);
        close(fd);
        return 1;
    }
    
    if (strcmp(argv[1], "status") == 0) {
        /* Check status */
        ret = ioctl(fd, VITRIOL_IOC_CHECK_STATUS);
        if (ret == 0) {
            printf("VITRIOL: Kernel module loaded and GPU mapped\n");
        } else {
            printf("VITRIOL: Module loaded but GPU not mapped (error %d)\n", ret);
        }
    }
    else if (strcmp(argv[1], "bar1") == 0) {
        /* Get BAR1 address */
        uint64_t bar1_addr;
        ret = ioctl(fd, VITRIOL_IOC_GET_BAR_ADDR, &bar1_addr);
        if (ret == 0) {
            printf("VITRIOL: BAR1 (Data Plane) address: 0x%lx\n", bar1_addr);
            printf("         (256MB VRAM window for DMA)\n");
        } else {
            printf("VITRIOL: Failed to get BAR1 address (error %d)\n", ret);
        }
    }
    else if (strcmp(argv[1], "transfer") == 0) {
        /* Test transfer */
        vitriol_transfer_t transfer = {
            .gpu_offset = 0x100000,     /* 1MB into VRAM */
            .file_offset = 0x1000000,   /* 16MB into model file */
            .size = 0x100000,           /* 1MB transfer */
            .direction = 0               /* Read from NVMe */
        };
        
        printf("VITRIOL: Testing DMA transfer...\n");
        printf("  GPU offset:  0x%lx\n", transfer.gpu_offset);
        printf("  File offset:  0x%lx\n", transfer.file_offset);
        printf("  Size:        0x%lx\n", transfer.size);
        
        ret = ioctl(fd, VITRIOL_IOC_DMA_TRANSFER, &transfer);
        if (ret == 0) {
            printf("VITRIOL: Transfer completed successfully\n");
        } else {
            printf("VITRIOL: Transfer failed (error %d)\n", ret);
        }
    }
    else {
        printf("Unknown command: %s\n", argv[1]);
        printf("Commands: status, bar1, transfer\n");
    }
    
    close(fd);
    return 0;
}