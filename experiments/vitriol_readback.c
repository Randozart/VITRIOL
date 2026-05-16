/*
 * vitriol_readback.c — Verify VITRIOL BAR1 DMA via READ_BAR1 IOCTL
 *
 * After the executor writes data to BAR1 via FLOW, this program
 * reads it back and compares it with the source file.
 *
 * Usage:
 *   ./vitriol_readback <gguf_path> [bar1_offset] [size]
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>

struct vitriol_bar1_read {
    uint64_t bar1_offset;
    uint64_t size;
    uint64_t buf;
} __attribute__((packed));

#define VITRIOL_IOC_MAGIC     0xA1
#define VITRIOL_IOC_READ_BAR1  _IOR(VITRIOL_IOC_MAGIC, 8, struct vitriol_bar1_read)

static const char *GGUF_MAGIC = "GGUF";

int main(int argc, char *argv[])
{
    if (argc < 2) {
        fprintf(stderr, "Usage: %s <gguf_path> [bar1_offset] [size]\n", argv[0]);
        return 1;
    }

    const char *gguf_path = argv[1];
    uint64_t bar1_offset = argc > 2 ? strtoull(argv[2], NULL, 0) : 0;
    uint64_t size = argc > 3 ? strtoull(argv[3], NULL, 0) : 4096;

    /* ── Read source GGUF file ── */
    FILE *f = fopen(gguf_path, "rb");
    if (!f) {
        perror("fopen GGUF");
        return 1;
    }

    unsigned char *expected = malloc(size);
    if (!expected) {
        perror("malloc");
        fclose(f);
        return 1;
    }

    size_t nread = fread(expected, 1, size, f);
    fclose(f);

    if (nread == 0) {
        fprintf(stderr, "Empty GGUF file\n");
        free(expected);
        return 1;
    }

    if (nread < size) {
        printf("WARN: GGUF file is smaller than read size (%zu < %lu)\n",
               nread, (unsigned long)size);
        size = nread;
    }

    /* ── Open VITRIOL device ── */
    int fd = open("/dev/vitriol", O_RDWR);
    if (fd < 0) {
        perror("open /dev/vitriol");
        free(expected);
        return 1;
    }

    /* ── Read from BAR1 ── */
    unsigned char *vram_data = malloc(size);
    if (!vram_data) {
        perror("malloc");
        close(fd);
        free(expected);
        return 1;
    }

    struct vitriol_bar1_read req;
    req.bar1_offset = bar1_offset;
    req.size = size;
    req.buf = (uint64_t)(unsigned long)vram_data;

    printf("READ_BAR1: offset=0x%lx size=%lu\n",
           (unsigned long)bar1_offset, (unsigned long)size);

    int ret = ioctl(fd, VITRIOL_IOC_READ_BAR1, &req);
    if (ret != 0) {
        perror("VITRIOL_IOC_READ_BAR1");
        fprintf(stderr, "  BAR1 may not be mapped (vitriol doesn't own GPU)\n");
        close(fd);
        free(expected);
        free(vram_data);
        return 1;
    }

    close(fd);

    /* ── Verify ── */
    printf("\n=== BAR1 DMA Verification ===\n\n");

    /* Check if VRAM is all zeros (no DMA happened) */
    int all_zero = 1;
    for (size_t i = 0; i < size; i++) {
        if (vram_data[i] != 0) {
            all_zero = 0;
            break;
        }
    }

    if (all_zero) {
        printf("FAIL: BAR1 is all zeros — DMA did not write to VRAM\n");
        free(expected);
        free(vram_data);
        return 1;
    }

    printf("PASS: BAR1 contains non-zero data — DMA wrote to VRAM\n\n");

    /* Check GGUF magic */
    printf("Expected first 4 bytes (from GGUF): %02x %02x %02x %02x (%c%c%c%c)\n",
           expected[0], expected[1], expected[2], expected[3],
           expected[0], expected[1], expected[2], expected[3]);
    printf("VRAM    first 4 bytes:              %02x %02x %02x %02x (%c%c%c%c)\n",
           vram_data[0], vram_data[1], vram_data[2], vram_data[3],
           vram_data[0], vram_data[1], vram_data[2], vram_data[3]);

    /* Compare byte-by-byte */
    int match = 1;
    size_t check_len = (nread < size) ? nread : size;
    for (size_t i = 0; i < check_len; i++) {
        if (vram_data[i] != expected[i]) {
            if (match && i < 64) {
                printf("\nMISMATCH at byte %zu:\n", i);
                printf("  VRAM:     ");
                for (size_t j = i; j < i + 16 && j < check_len; j++)
                    printf("%02x ", vram_data[j]);
                printf("\n  Expected: ");
                for (size_t j = i; j < i + 16 && j < check_len; j++)
                    printf("%02x ", expected[j]);
                printf("\n");
            }
            match = 0;
        }
    }

    if (match) {
        printf("\nPASS: All %zu bytes match GGUF source — DMA is CORRECT\n", check_len);
    } else {
        printf("\nFAIL: Data mismatch — DMA corruption or wrong offset\n");
    }

    /* Print first 64 bytes as hex */
    printf("\nFirst 64 bytes of VRAM:\n");
    for (size_t i = 0; i < 64 && i < size; i++) {
        if (i > 0 && i % 16 == 0) printf("\n");
        printf("%02x ", vram_data[i]);
    }
    printf("\n");

    free(expected);
    free(vram_data);
    return match ? 0 : 1;
}
