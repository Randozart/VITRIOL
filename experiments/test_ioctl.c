#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>
#include <stdint.h>
#include <string.h>
#include <errno.h>

#define VITRIOL_IOC_MAGIC 0xA1

struct vitriol_drop {
    uint8_t  op_code;
    uint8_t  flags;
    uint16_t vessel_id;
    uint64_t src_addr;
    uint64_t dst_addr;
    uint32_t size;
    uint32_t reserved;
    uint32_t crc;
} __attribute__((packed));

struct vitriol_vial {
    uint64_t aperture_size;
    uint64_t aperture_max;
    uint8_t  thermal_halt;
    uint8_t  thermal_throttle;
    uint8_t  dma_capable;
    uint8_t  _pad[5];
} __attribute__((packed));

#define VITRIOL_IOC_SET_VIAL  _IOW(VITRIOL_IOC_MAGIC, 3, struct vitriol_vial)
#define VITRIOL_IOC_EXECUTE   _IOW(VITRIOL_IOC_MAGIC, 1, struct vitriol_drop)

int main(void) {
    int fd = open("/dev/vitriol", O_RDWR);
    if (fd < 0) {
        perror("open /dev/vitriol");
        return 1;
    }
    printf("Opened /dev/vitriol (fd=%d)\n", fd);

    struct vitriol_vial vial = {
        .aperture_size = 0x10000000,
        .aperture_max = 0x1000000,
        .thermal_halt = 85,
        .dma_capable = 1,
    };

    int ret = ioctl(fd, VITRIOL_IOC_SET_VIAL, &vial);
    printf("SET_VIAL ioctl(0x%x) returned %d (errno=%d: %s)\n",
           VITRIOL_IOC_SET_VIAL, ret, errno, strerror(errno));

    struct vitriol_drop drop = {
        .op_code = 0x01,
        .vessel_id = 0x0001,
        .size = 0,
    };

    ret = ioctl(fd, VITRIOL_IOC_EXECUTE, &drop);
    printf("EXECUTE ioctl(0x%x) returned %d (errno=%d: %s)\n",
           VITRIOL_IOC_EXECUTE, ret, errno, strerror(errno));

    close(fd);
    return 0;
}
