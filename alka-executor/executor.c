/*
 * alka-executor — VITRIOL Alka Stream Executor
 *
 * Copyright 2026 Randy Smits-Schreuder Goedheijt
 * Licensed under Apache 2.0 with Runtime Exception.
 *
 * Reads compiled Alka streams (.alkas) and executes them against
 * the VITRIOL kernel module via IOCTLs. Validates each Drop packet
 * against Vial constraints (.alkavl) before execution.
 *
 * Usage: alka-executor <stream.alkas> <vial.alkavl> [--dry-run] [--rollback <azoth>]
 */

#define _POSIX_C_SOURCE 200809L
#define _DEFAULT_SOURCE

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <errno.h>
#include <time.h>
#include <getopt.h>
#include <signal.h>
#include <sys/types.h>
#include <sys/mman.h>

#include "vitriol_alka_user.h"

#define DEVICE_PATH "/dev/vitriol"
#define MAX_VESSELS 16
#define MAX_LINES 256

/* ── BAR1 mmap (userspace BAR1 write path) ────────────────────── */

static void *bar1_map = NULL;
static uint64_t bar1_map_size = 0;

static int setup_bar1_mmap(int fd)
{
    struct vitriol_bar1_info info;
    if (ioctl(fd, VITRIOL_IOC_GET_BAR1_PHYS, &info) != 0) {
        fprintf(stderr, "Warning: BAR1 mmap unavailable (GET_BAR1_PHYS failed)\n");
        return -1;
    }
    if (info.phys_addr == 0 || info.size == 0) {
        fprintf(stderr, "Warning: BAR1 not available (phys=0, size=0)\n");
        return -1;
    }
    /* mmap /dev/mem directly (bypasses vitriol module mmap handler) */
    int mem_fd = open("/dev/mem", O_RDWR | O_SYNC);
    if (mem_fd < 0) {
        fprintf(stderr, "Warning: Cannot open /dev/mem: %s\n", strerror(errno));
        return -1;
    }
    bar1_map_size = info.size;
    bar1_map = mmap(NULL, bar1_map_size, PROT_READ | PROT_WRITE,
                    MAP_SHARED, mem_fd, (off_t)info.phys_addr);
    close(mem_fd);
    if (bar1_map == MAP_FAILED) {
        fprintf(stderr, "Warning: BAR1 /dev/mem mmap failed: %s\n", strerror(errno));
        bar1_map = NULL;
        return -1;
    }
    printf("BAR1 mmap: phys=0x%llx size=%llu at %p\n",
           (unsigned long long)info.phys_addr,
           (unsigned long long)info.size, bar1_map);
    return 0;
}

static void teardown_bar1_mmap(void)
{
    if (bar1_map) {
        munmap(bar1_map, bar1_map_size);
        bar1_map = NULL;
    }
}

static int write_flow_to_bar1(int fd, const struct vitriol_drop *drop)
{
    if (!bar1_map)
        return 0;

    uint32_t size = drop->size;
    if (size == 0 || size > 64 * 1024 * 1024)
        return 0;

    /* Allocate user buffer and read FLOW data from kernel */
    uint8_t *buf = malloc(size);
    if (!buf) {
        fprintf(stderr, "Error: OOM for FLOW buffer (%u bytes)\n", size);
        return -1;
    }

    struct vitriol_flow_buf req;
    req.buf = (uint64_t)(unsigned long)buf;
    req.size = size;
    if (ioctl(fd, VITRIOL_IOC_GET_FLOW_BUF, &req) != 0) {
        fprintf(stderr, "  FLOW buffer readback failed: %s\n", strerror(errno));
        free(buf);
        return -1;
    }

    /* Write to mmap'd BAR1 at drop's destination offset */
    uint64_t bar1_off = drop->dst_addr;
    if (bar1_off + size > bar1_map_size) {
        fprintf(stderr, "  FLOW offset %llu + %u exceeds BAR1 size %llu\n",
                (unsigned long long)bar1_off, size,
                (unsigned long long)bar1_map_size);
        free(buf);
        return -1;
    }

    memcpy((uint8_t *)bar1_map + bar1_off, buf, size);
    __sync_synchronize();

    printf("  [FLOW] %u bytes written to BAR1 offset 0x%llx\n",
           size, (unsigned long long)bar1_off);

    free(buf);
    return 0;
}

/* ── Vial Parser ───────────────────────────────────────────────── */

struct vial_vessel {
    char name[64];
    uint16_t id;
    uint64_t pci_id;
    uint64_t bar0_base;
    uint64_t bar1_base;
    uint64_t bar1_size;
    uint64_t bar1_max_window;
    uint64_t vram_total;
    uint64_t vram_reserved;
    uint32_t thermal_halt;
    uint32_t thermal_throttle;
    int dma_capable;
    uint64_t dma_max_burst;
};

static struct vial_vessel vessels[MAX_VESSELS];
static int vessel_count = 0;

static struct vial_vessel *find_vessel_by_name(const char *name)
{
    for (int i = 0; i < vessel_count; i++) {
        if (strcmp(vessels[i].name, name) == 0)
            return &vessels[i];
    }
    return NULL;
}

static uint64_t parse_size(const char *str)
{
    char *end;
    uint64_t val = strtoull(str, &end, 0);
    if (strstr(end, "GB")) val *= 1024ULL * 1024 * 1024;
    else if (strstr(end, "MB")) val *= 1024 * 1024;
    else if (strstr(end, "KB")) val *= 1024;
    else if (strstr(end, "B")) { /* already bytes */ }
    else if (*end == 'C' && *(end+1) == '\0') { /* temperature, not size */ val = val; }
    return val;
}

static uint32_t parse_temp(const char *str)
{
    char *end;
    uint32_t val = strtoul(str, &end, 0);
    /* Handle millicelsius (e.g., 98000 = 98°C) */
    if (val > 200) val = val / 1000;
    return val;
}

static int parse_vial(const char *path)
{
    FILE *f = fopen(path, "r");
    if (!f) {
        fprintf(stderr, "Error: Cannot open vial file: %s\n", path);
        return -1;
    }

    char line[MAX_LINES];
    char current_vessel[64] = {0};
    int in_aperture = 0;
    int in_thermal = 0;
    int in_memory = 0;
    char aperture_name[64] = {0};

    while (fgets(line, sizeof(line), f)) {
        /* Strip comments */
        char *comment = strchr(line, '/');
        if (comment && *(comment-1) == '/') {
            *(comment-1) = '\0';
        }

        /* Strip whitespace */
        char *p = line;
        while (*p == ' ' || *p == '\t') p++;

        if (strlen(p) < 2 || *p == '\n' || *p == '\0') continue;

        /* Vessel declaration */
        if (strncmp(p, "Vessel ", 7) == 0) {
            char *name_start = p + 7;
            char *name_end = strchr(name_start, '{');
            if (name_end) {
                *name_end = '\0';
                /* Trim whitespace */
                while (*name_end == ' ') name_end--;
                *(name_end + 1) = '\0';
            }
            strncpy(current_vessel, name_start, sizeof(current_vessel) - 1);

            if (vessel_count < MAX_VESSELS) {
                struct vial_vessel *v = &vessels[vessel_count];
                memset(v, 0, sizeof(*v));
                strncpy(v->name, current_vessel, sizeof(v->name) - 1);
                v->id = vessel_count + 1;
                vessel_count++;
            }
            in_aperture = 0;
            in_thermal = 0;
            in_memory = 0;
            continue;
        }

        struct vial_vessel *v = vessel_count > 0 ? &vessels[vessel_count - 1] : NULL;
        if (!v) continue;

        /* Aperture block */
        if (strncmp(p, "Aperture ", 9) == 0) {
            char *name_start = p + 9;
            char *name_end = strchr(name_start, '{');
            if (name_end) {
                *name_end = '\0';
            }
            strncpy(aperture_name, name_start, sizeof(aperture_name) - 1);
            in_aperture = 1;
            in_thermal = 0;
            in_memory = 0;
            continue;
        }

        /* Thermal block */
        if (strncmp(p, "Thermal ", 8) == 0) {
            in_thermal = 1;
            in_aperture = 0;
            in_memory = 0;
            continue;
        }

        /* Memory block */
        if (strncmp(p, "Memory ", 7) == 0) {
            in_memory = 1;
            in_aperture = 0;
            in_thermal = 0;
            continue;
        }

        /* Closing brace */
        if (p[0] == '}') {
            if (in_aperture) in_aperture = 0;
            else if (in_thermal) in_thermal = 0;
            else if (in_memory) in_memory = 0;
            continue;
        }

        /* Parse key: value — handle values with colons (e.g., PCI_ID: 10de:1401) */
        char *colon = strchr(p, ':');
        if (!colon) continue;
        *colon = '\0';
        char *key = p;
        char *val = colon + 1;
        /* Skip leading whitespace in value */
        while (*val == ' ') val++;
        /* Strip trailing whitespace/semicolon/newline */
        char *val_end = val + strlen(val) - 1;
        while (val_end > val && (*val_end == ' ' || *val_end == '\n' || *val_end == '\r' || *val_end == ';'))
            *val_end-- = '\0';

        if (in_aperture) {
            if (strcmp(key, "BAR") == 0) {
                /* BAR number — we track base from comments or defaults */
            } else if (strcmp(key, "BASE") == 0) {
                uint64_t addr = strtoull(val, NULL, 0);
                if (strstr(aperture_name, "DATA") || strstr(aperture_name, "data"))
                    v->bar1_base = addr;
                else if (strstr(aperture_name, "CTRL") || strstr(aperture_name, "ctrl"))
                    v->bar0_base = addr;
            } else if (strcmp(key, "SIZE") == 0) {
                uint64_t sz = parse_size(val);
                if (strstr(aperture_name, "DATA") || strstr(aperture_name, "data"))
                    v->bar1_size = sz;
            } else if (strcmp(key, "MAX_WINDOW") == 0) {
                v->bar1_max_window = parse_size(val);
            } else if (strcmp(key, "TYPE") == 0) {
                /* Prefetchable / NonPrefetchable */
            }
        } else if (in_thermal) {
            if (strcmp(key, "HALT_AT") == 0) {
                v->thermal_halt = parse_temp(val);
            } else if (strcmp(key, "THROTTLE_AT") == 0) {
                v->thermal_throttle = parse_temp(val);
            }
        } else if (in_memory) {
            if (strcmp(key, "TOTAL") == 0) {
                v->vram_total = parse_size(val);
            } else if (strcmp(key, "RESERVED") == 0) {
                v->vram_reserved = parse_size(val);
            }
        } else {
            if (strcmp(key, "PCI_ID") == 0) {
                /* Handle "vendor:device" format — Alka stores as (device << 16) | vendor */
                char *sep = strchr(val, ':');
                if (sep) {
                    uint64_t vendor = strtoull(val, NULL, 16);
                    uint64_t device = strtoull(sep + 1, NULL, 16);
                    v->pci_id = (device << 16) | vendor;
                } else {
                    v->pci_id = strtoull(val, NULL, 0);
                }
            } else if (strcmp(key, "DMA_CAPABLE") == 0) {
                v->dma_capable = (strstr(val, "true") || strstr(val, "1")) ? 1 : 0;
            } else if (strcmp(key, "DMA_MAX_BURST") == 0) {
                v->dma_max_burst = strtoull(val, NULL, 0);
            } else if (strcmp(key, "BDF") == 0) {
                /* Bus:Device:Function — store for info */
            } else if (strcmp(key, "NAME") == 0) {
                /* Already have name from Vessel declaration */
            } else if (strcmp(key, "BLOCK_DEVICE") == 0) {
                /* NVMe device path — store for info */
            }
        }
    }

    fclose(f);
    return 0;
}

static void print_vial_info(void)
{
    printf("Vial: %d vessel(s)\n", vessel_count);
    for (int i = 0; i < vessel_count; i++) {
        struct vial_vessel *v = &vessels[i];
        printf("  [%d] %s (PCI: 0x%lx)\n", v->id, v->name, (unsigned long)v->pci_id);
        if (v->bar1_size)
            printf("      BAR1: base=0x%lx size=%luMB max_window=%luMB\n",
                   (unsigned long)v->bar1_base,
                   (unsigned long)(v->bar1_size / (1024*1024)),
                   (unsigned long)(v->bar1_max_window / (1024*1024)));
        if (v->vram_total)
            printf("      VRAM: %luMB (reserved: %luMB)\n",
                   (unsigned long)(v->vram_total / (1024*1024)),
                   (unsigned long)(v->vram_reserved / (1024*1024)));
        if (v->thermal_halt)
            printf("      Thermal: halt=%u°C throttle=%u°C\n",
                   v->thermal_halt, v->thermal_throttle);
    }
}

/* ── Drop Validation ───────────────────────────────────────────── */

static int validate_drop(const struct vitriol_drop *drop)
{
    /* CRC check */
    if (drop->crc != 0) {
        uint32_t computed = compute_drop_crc(drop);
        if (drop->crc != computed) {
            fprintf(stderr, "  CRC mismatch: expected=0x%x got=0x%x\n",
                    computed, drop->crc);
            return -1;
        }
    }

    /*
     * Alka encoding: vessel_id may be 0. The actual vessel reference
     * is encoded in src_addr as a PCI ID (vendor:device packed).
     * Match vessels by PCI ID when vessel_id is 0.
     */
    struct vial_vessel *v = NULL;

    /* First try matching by vessel_id */
    if (drop->vessel_id != 0) {
        for (int i = 0; i < vessel_count; i++) {
            if (vessels[i].id == drop->vessel_id) {
                v = &vessels[i];
                break;
            }
        }
    }

    /* If vessel_id is 0, try matching by PCI ID in src_addr */
    if (!v && drop->src_addr != 0) {
        for (int i = 0; i < vessel_count; i++) {
            if (vessels[i].pci_id == drop->src_addr) {
                v = &vessels[i];
                break;
            }
        }
    }

    /* CLAIM, REFRACT, SYNC, WATCH, FENCE, SIGNAL, LIMIT, SHIFT don't require vessel ref when zero */
    if (!v && drop->op_code != OP_CLAIM && drop->op_code != OP_REFRACT &&
        drop->op_code != OP_SYNC && drop->op_code != OP_WATCH &&
        drop->op_code != OP_FENCE && drop->op_code != OP_SIGNAL &&
        drop->op_code != OP_LIMIT && drop->op_code != OP_SHIFT &&
        drop->op_code != OP_FLOW) {
        fprintf(stderr, "  Unknown vessel 0x%x (not claimed)\n", drop->vessel_id);
        return -1;
    }

    /* Opcode-specific validation */
    switch (drop->op_code) {
    case OP_FLOW:
        if (drop->size == 0) {
            fprintf(stderr, "  FLOW: zero-size transfer rejected\n");
            return -1;
        }
        if (v && v->bar1_max_window && drop->size > v->bar1_max_window) {
            fprintf(stderr, "  FLOW: size %u exceeds max window %lu\n",
                    drop->size, (unsigned long)v->bar1_max_window);
            return -1;
        }
        if (v && !v->dma_capable && v->pci_id != 0) {
            fprintf(stderr, "  FLOW: vessel not DMA capable\n");
            return -1;
        }
        break;

    case OP_SHIFT:
        if (v && v->bar1_max_window) {
            if (drop->src_addr + v->bar1_max_window > v->bar1_size) {
                fprintf(stderr, "  SHIFT: offset 0x%lx exceeds BAR1 size\n",
                        (unsigned long)drop->src_addr);
                return -1;
            }
        }
        break;

    case OP_FENCE:
        /* dst_addr encodes expected metapage value — no validation needed */
        break;

    case OP_SIGNAL:
        /* src_addr encodes signal ID — must be non-zero */
        if (drop->src_addr == 0) {
            fprintf(stderr, "  SIGNAL: zero signal ID rejected\n");
            return -1;
        }
        break;

    /* LIMIT with zero operands — skip validation (template stream) */
    case OP_LIMIT:
        if (v && v->thermal_halt && (drop->src_addr != 0 || drop->dst_addr != 0)) {
            /* Thermal value is in dst_addr (millicelsius) or low byte of src_addr */
            uint32_t thermal;
            if (drop->dst_addr != 0)
                thermal = (uint32_t)(drop->dst_addr > 200 ? drop->dst_addr / 1000 : drop->dst_addr);
            else
                thermal = (uint32_t)(drop->src_addr & 0xFF);

            if (thermal > v->thermal_halt) {
                fprintf(stderr, "  LIMIT: thermal %u exceeds halt %u\n",
                        thermal, v->thermal_halt);
                return -1;
            }
        }
        break;

    case OP_REFRACT:
        if (v && v->bar1_size) {
            uint64_t total = drop->dst_addr;
            if (total > v->bar1_size) {
                fprintf(stderr, "  REFRACT: total range %lu exceeds BAR1 size %lu\n",
                        (unsigned long)total, (unsigned long)v->bar1_size);
                return -1;
            }
        }
        break;
    }

    return 0;
}

/* ── Execution ─────────────────────────────────────────────────── */

static const char *opcode_name(uint8_t op)
{
    switch (op) {
    case OP_CLAIM:    return "CLAIM";
    case OP_STAKE:    return "STAKE";
    case OP_FLOW:     return "FLOW";
    case OP_SHIFT:    return "SHIFT";
    case OP_FENCE:    return "FENCE";
    case OP_SYNC:     return "SYNC";
    case OP_SENSE:    return "SENSE";
    case OP_PULSE:    return "PULSE";
    case OP_SIGNAL:   return "SIGNAL";
    case OP_YIELD:    return "YIELD";
    case OP_RECAST:   return "RECAST";
    case OP_SNAP:     return "SNAP";
    case OP_REVERT:   return "REVERT";
    case OP_LIMIT:    return "LIMIT";
    case OP_MOLT:     return "MOLT";
    case OP_ECHO:     return "ECHO";
    case OP_STASIS:   return "STASIS";
    case OP_FLUX:     return "FLUX";
    case OP_AUDIT:    return "AUDIT";
    case OP_DRY_RUN:  return "DRY_RUN";
    case OP_MOCK:     return "MOCK";
    case OP_PROVE:    return "PROVE";
    case OP_WATCH:    return "WATCH";
    case OP_TRACE:    return "TRACE";
    case OP_GUARD:    return "GUARD";
    case OP_ISOLATE:  return "ISOLATE";
    case OP_VERIFY:   return "VERIFY";
    case OP_OSSIFY:   return "OSSIFY";
    case OP_BOND:     return "BOND";
    case OP_STILL:    return "STILL";
    case OP_RESONATE: return "RESONATE";
    case OP_OSCILLATE:return "OSCILLATE";
    case OP_IMC_HIJACK:return "IMC_HIJACK";
    case OP_OCCUPY:   return "OCCUPY";
    case OP_REFRACT:  return "REFRACT";
    case OP_PIPE:     return "PIPE";
    default:          return "UNKNOWN";
    }
}

static int execute_stream(int fd, struct vitriol_drop *drops, uint32_t count,
                          int dry_run)
{
    struct vitriol_result result = {0};
    uint64_t total_bytes = 0;
    struct timespec start, end;

    printf("Executing %u drop(s)...\n", count);

    clock_gettime(CLOCK_MONOTONIC, &start);

    for (uint32_t i = 0; i < count; i++) {
        struct vitriol_drop *drop = &drops[i];

        printf("  [%2u] %-10s vessel=0x%04x size=%u ",
               i, opcode_name(drop->op_code), drop->vessel_id, drop->size);

        /* Validate */
        if (validate_drop(drop) != 0) {
            printf("FAILED (validation)\n");
            fprintf(stderr, "Stream aborted at drop %u\n", i);
            return -1;
        }

        if (dry_run) {
            printf("(dry-run OK)\n");
            total_bytes += drop->size;
            continue;
        }

        /* Execute via IOCTL */
        int ret = ioctl(fd, VITRIOL_IOC_EXECUTE, drop);
        if (ret != 0) {
            printf("FAILED (ioctl: %s)\n", strerror(errno));
            fprintf(stderr, "Stream aborted at drop %u (opcode 0x%x)\n",
                    i, drop->op_code);

            /* Get result from kernel */
            ioctl(fd, VITRIOL_IOC_GET_RESULT, &result);
            fprintf(stderr, "Kernel error: %s\n", result.error_message);
            return -1;
        }

        /* After FLOW: copy kernel buffer to mmap'd BAR1 */
        if (drop->op_code == OP_FLOW)
            write_flow_to_bar1(fd, drop);

        printf("OK\n");
        total_bytes += drop->size;
    }

    clock_gettime(CLOCK_MONOTONIC, &end);
    double elapsed = (end.tv_sec - start.tv_sec) + (end.tv_nsec - start.tv_nsec) / 1e9;

    /* Get final result */
    if (!dry_run) {
        ioctl(fd, VITRIOL_IOC_GET_RESULT, &result);
    }

    printf("\nStream complete: %u drops, %lu bytes, %.3fs\n",
           count, (unsigned long)total_bytes, elapsed);

    return 0;
}

static int execute_rollback(int fd, struct vitriol_azoth *azoth, uint32_t count)
{
    printf("Executing rollback (%u azoth packets)...\n", count);

    for (int32_t i = count - 1; i >= 0; i--) {
        struct vitriol_drop drop = {
            .op_code = azoth[i].op_code,
            .flags = azoth[i].flags,
            .vessel_id = azoth[i].vessel_id,
            .src_addr = azoth[i].src_addr,
            .dst_addr = azoth[i].dst_addr,
            .size = azoth[i].size,
            .reserved = azoth[i].reserved,
            .crc = azoth[i].crc,
        };

        printf("  [%2d] ROLLBACK %-10s vessel=0x%04x\n",
               i, opcode_name(drop.op_code), drop.vessel_id);

        int ret = ioctl(fd, VITRIOL_IOC_EXECUTE, &drop);
        if (ret != 0) {
            fprintf(stderr, "  Rollback failed at packet %d: %s\n", i, strerror(errno));
        }
    }

    return 0;
}

/* ── File Loading ──────────────────────────────────────────────── */

static struct vitriol_drop *load_alkas(const char *path, uint32_t *count)
{
    FILE *f = fopen(path, "rb");
    if (!f) {
        fprintf(stderr, "Error: Cannot open stream file: %s\n", path);
        return NULL;
    }

    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);

    if (size % sizeof(struct vitriol_drop) != 0) {
        fprintf(stderr, "Error: Stream file size (%ld) not multiple of Drop size (%zu)\n",
                size, sizeof(struct vitriol_drop));
        fclose(f);
        return NULL;
    }

    *count = size / sizeof(struct vitriol_drop);
    struct vitriol_drop *drops = malloc(size);
    if (!drops) {
        fprintf(stderr, "Error: Out of memory\n");
        fclose(f);
        return NULL;
    }

    if (fread(drops, 1, (size_t)size, f) != (size_t)size) {
        fprintf(stderr, "Error: Failed to read stream file\n");
        free(drops);
        fclose(f);
        return NULL;
    }

    fclose(f);
    return drops;
}

static struct vitriol_azoth *load_azoth(const char *path, uint32_t *count)
{
    FILE *f = fopen(path, "rb");
    if (!f) {
        fprintf(stderr, "Warning: Cannot open azoth file: %s\n", path);
        return NULL;
    }

    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);

    if (size % sizeof(struct vitriol_azoth) != 0) {
        fprintf(stderr, "Error: Azoth file size (%ld) not multiple of Azoth size (%zu)\n",
                size, sizeof(struct vitriol_azoth));
        fclose(f);
        return NULL;
    }

    *count = size / sizeof(struct vitriol_azoth);
    struct vitriol_azoth *azoth = malloc(size);
    if (!azoth) {
        fprintf(stderr, "Error: Out of memory\n");
        fclose(f);
        return NULL;
    }

    if (fread(azoth, 1, (size_t)size, f) != (size_t)size) {
        fprintf(stderr, "Error: Failed to read azoth file\n");
        free(azoth);
        fclose(f);
        return NULL;
    }

    fclose(f);
    return azoth;
}

/* ── Main ──────────────────────────────────────────────────────── */

static void usage(const char *prog)
{
    printf("Usage: %s <stream.alkas> <vial.alkavl> [options]\n", prog);
    printf("\nOptions:\n");
    printf("  --dry-run           Validate without executing\n");
    printf("  --rollback <file>   Azoth rollback file\n");
    printf("  --source <path>     GGUF source file for FLOW transfers\n");
    printf("  --bind <BDF>        Bind PCI device to VITRIOL via fork-safe sysfs\n");
    printf("                      (e.g. 0000:02:00.0). Uses driver_override +\n");
    printf("                      hot-remove + rescan with 30s timeout.\n");
    printf("  --cooperative       Use nvidia P2P cooperative DMA (no unbind needed)\n");
    printf("  --gpu-va <addr>     GPU virtual address for cooperative DMA (hex)\n");
    printf("  --p2p-token <val>   nvidia P2P token for GPU VA (from CUDA driver API)\n");
    printf("  --va-space-token <v> nvidia VA space token for GPU VA\n");
    printf("  --device <path>     VITRIOL device path (default: /dev/vitriol)\n");
    printf("  --verbose           Print detailed validation info\n");
    printf("  --help              Show this help\n");
}

/* ── Safe Userspace PCI Device Binding ──────────────────────────── */
/*
 * Uses fork-based timeout to avoid kernel D-state hangs.
 * The child process does the sysfs writes; if it blocks in D-state,
 * the parent times out and the orphaned child is safely reaped by init.
 * No kernel workqueue involvement — rmmod is always safe.
 */

#include <sys/wait.h>
#include <sys/types.h>

static int bind_pci_device(int dev_fd, const char *bdf)
{
    char path[256];
    int retry;
    pid_t pid;
    int status;
    const char *bdf_short;

    /* Strip domain prefix for sysfs (some kernels want "0000:02:00.0", some want "02:00.0") */
    bdf_short = strchr(bdf, ':');
    if (!bdf_short) bdf_short = bdf;

    printf("Bind: spawning child for %s sysfs operations...\n", bdf);

    /* Pre-flight check: what driver owns this device? */
    {
        char drvlink[256];
        struct stat drvstat;
        snprintf(drvlink, sizeof(drvlink),
                 "/sys/bus/pci/devices/%s/driver", bdf);
        if (stat(drvlink, &drvstat) == 0) {
            char linkbuf[128];
            ssize_t len = readlink(drvlink, linkbuf, sizeof(linkbuf) - 1);
            if (len > 0) {
                linkbuf[len] = '\0';
                const char *drv = strrchr(linkbuf, '/');
                drv = drv ? drv + 1 : linkbuf;
                if (strcmp(drv, "vitriol") == 0) {
                    printf("Bind: %s already belongs to vitriol, skipping\n", bdf);
                    return 0;
                }
                printf("Bind: %s currently owned by %s\n", bdf, drv);
                if (strcmp(drv, "nvidia") == 0) {
                    printf("Bind: nvidia holds this device — display manager may block\n");
                    printf("Bind: timeout expected; switch to TTY if this fails\n");
                }
            }
        } else {
            printf("Bind: %s has no driver — attempting direct bind\n", bdf);
        }
    }

    /*
     * Strategy: try multiple vectors for claiming the GPU from nvidia.
     *
     * Vector 1 (clean): driver_override + unbind + vitriol bind
     *   - Set driver_override so nvidia can't re-claim
     *   - Write BDF to nvidia's unbind file (graceful release)
     *   - Write BDF to vitriol's bind file (claim it)
     *
     * Vector 2 (force): driver_override + remove + rescan
     *   - Set driver_override
     *   - Remove device from PCI subsystem (bypasses nvidia refcount)
     *   - Rescan PCI bus (vitriol probes on reappearance)
     */

    pid = fork();
    if (pid == -1) {
        fprintf(stderr, "Error: fork failed: %s\n", strerror(errno));
        return -1;
    }

    if (pid == 0) {
        /* ── Child process ── */
        int fd;

        /* Set driver_override first (restraining order against nvidia) */
        snprintf(path, sizeof(path),
                 "/sys/bus/pci/devices/%s/driver_override", bdf);
        fd = open(path, O_WRONLY);
        if (fd >= 0) {
            ssize_t w = write(fd, "vitriol\n", 8);
            (void)w;
            close(fd);
        }

        /* ── Vector 1: Try unbind from nvidia ── */
        snprintf(path, sizeof(path),
                 "/sys/bus/pci/drivers/nvidia/unbind");
        fd = open(path, O_WRONLY);
        if (fd >= 0) {
            ssize_t w = write(fd, bdf_short, strlen(bdf_short));
            (void)w;
            w = write(fd, "\n", 1);
            (void)w;
            close(fd);

            /* If unbind succeeded, try binding to vitriol directly */
            snprintf(path, sizeof(path),
                     "/sys/bus/pci/drivers/vitriol/bind");
            fd = open(path, O_WRONLY);
            if (fd >= 0) {
                ssize_t w = write(fd, bdf_short, strlen(bdf_short));
                (void)w;
                w = write(fd, "\n", 1);
                (void)w;
                close(fd);
                _exit(0);  /* Vector 1 succeeded */
            }
        }

        /* ── Vector 2: Remove + rescan (forceful) ── */
        /* Re-set driver_override (guarantee it's set) */
        snprintf(path, sizeof(path),
                 "/sys/bus/pci/devices/%s/driver_override", bdf);
        fd = open(path, O_WRONLY);
        if (fd >= 0) {
            ssize_t w = write(fd, "vitriol\n", 8);
            (void)w;
            close(fd);
        }

        /* Remove the device (this calls nvidia's remove callback) */
        snprintf(path, sizeof(path),
                 "/sys/bus/pci/devices/%s/remove", bdf);
        fd = open(path, O_WRONLY);
        if (fd >= 0) {
            ssize_t w = write(fd, "1\n", 2);
            (void)w;
            close(fd);
        }

        /* Rescan PCI bus */
        fd = open("/sys/bus/pci/rescan", O_WRONLY);
        if (fd >= 0) {
            ssize_t w = write(fd, "1\n", 2);
            (void)w;
            close(fd);
        }

        _exit(0);
    }

    /*
     * Parent — wait for completion with 30s timeout.
     * If child blocks in D-state, SIGKILL won't immediately
     * wake it, but the parent can safely continue. The orphaned
     * child will be cleaned up when the kernel operation completes.
     */

    printf("Bind: waiting for device to reappear under vitriol...\n");

    for (retry = 0; retry < 60; retry++) {
        int status;
        pid_t ret = waitpid(pid, &status, WNOHANG);
        if (ret == pid) {
            /* Child exited */
            if (WIFEXITED(status) && WEXITSTATUS(status) != 0) {
                fprintf(stderr, "Warning: BIND child failed (exit=%d)\n",
                        WEXITSTATUS(status));
            }
        }

        /* Check if device appeared under vitriol */
        snprintf(path, sizeof(path), "/sys/bus/pci/devices/%s/driver", bdf);
        struct stat st;
        if (stat(path, &st) == 0) {
            char linkbuf[128];
            ssize_t len = readlink(path, linkbuf, sizeof(linkbuf) - 1);
            if (len > 0) {
                linkbuf[len] = '\0';
                const char *drv = strrchr(linkbuf, '/');
                drv = drv ? drv + 1 : linkbuf;
                if (strcmp(drv, "vitriol") == 0) {
                    printf("Bind: %s → vitriol\n", bdf);
                    /* Kill child if still alive */
                    kill(pid, SIGKILL);
                    waitpid(pid, &status, 0);
                    return 0;
                }
            }
        } else {
            if (retry == 0)
                printf("Bind: device removed, waiting for rescan...\n");
        }

        usleep(500000);  /* 0.5s */
    }

    /* Timeout — kill child if still alive */
    fprintf(stderr, "Error: BIND did not complete within 30s\n");
    fprintf(stderr, "  Child PID %d may be in D-state (orphaning safe)\n", pid);
    kill(pid, SIGKILL);
    waitpid(pid, &status, 0);

    fprintf(stderr, "\n");
    fprintf(stderr, "  The nvidia driver likely holds display refs on %s.\n", bdf);
    fprintf(stderr, "  Switch to TTY (Ctrl+Alt+F3) and run:\n");
    fprintf(stderr, "    sudo systemctl stop gdm\n");
    fprintf(stderr, "    echo 'vitriol' | sudo tee /sys/bus/pci/devices/%s/driver_override\n", bdf);
    fprintf(stderr, "    echo '1' | sudo tee /sys/bus/pci/devices/%s/remove\n", bdf);
    fprintf(stderr, "    echo '1' | sudo tee /sys/bus/pci/rescan\n");
    fprintf(stderr, "    ./alka-executor/alka-executor ...\n");
    fprintf(stderr, "  Or use the automated script: sudo ./vitriol_bind_and_test.sh\n");
    return -1;
}

int main(int argc, char *argv[])
{
    const char *stream_path = NULL;
    const char *vial_path = NULL;
    const char *azoth_path = NULL;
    const char *source_path = NULL;
    const char *bind_bdf = NULL;
    const char *gpu_va_str = NULL;
    const char *p2p_token_str = NULL;
    const char *va_space_token_str = NULL;
    const char *device_path = DEVICE_PATH;
    int dry_run = 0;
    int cooperative = 0;
    int map_bar = 0;
    int verbose = 0;

    static struct option long_options[] = {
        {"dry-run",        no_argument,       0, 'd'},
        {"rollback",       required_argument, 0, 'r'},
        {"source",         required_argument, 0, 's'},
        {"bind",           required_argument, 0, 'b'},
        {"cooperative",    no_argument,       0, 'c'},
        {"gpu-va",         required_argument, 0, 'g'},
        {"p2p-token",      required_argument, 0, 'T'},
        {"va-space-token", required_argument, 0, 'S'},
        {"device",         required_argument, 0, 'D'},
        {"verbose",        no_argument,       0, 'v'},
        {"map-bar",        no_argument,       0, 'm'},
        {"help",           no_argument,       0, 'h'},
        {0, 0, 0, 0}
    };

    int opt;
    while ((opt = getopt_long(argc, argv, "dr:s:b:cg:T:S:D:vmh", long_options, NULL)) != -1) {
        switch (opt) {
        case 'd': dry_run = 1; break;
        case 'r': azoth_path = optarg; break;
        case 's': source_path = optarg; break;
        case 'b': bind_bdf = optarg; break;
        case 'c': cooperative = 1; break;
        case 'g': gpu_va_str = optarg; break;
        case 'T': p2p_token_str = optarg; break;
        case 'S': va_space_token_str = optarg; break;
        case 'D': device_path = optarg; break;
        case 'v': verbose = 1; break;
        case 'm': map_bar = 1; break;
        case 'h': usage(argv[0]); return 0;
        default: usage(argv[0]); return 1;
        }
    }

    if (optind >= argc) {
        fprintf(stderr, "Error: Missing stream file\n");
        usage(argv[0]);
        return 1;
    }
    stream_path = argv[optind];

    if (optind + 1 >= argc) {
        fprintf(stderr, "Error: Missing vial file\n");
        usage(argv[0]);
        return 1;
    }
    vial_path = argv[optind + 1];

    /* Load vial */
    printf("Loading vial: %s\n", vial_path);
    if (parse_vial(vial_path) != 0) {
        return 1;
    }
    if (verbose) print_vial_info();

    /* Load stream */
    printf("Loading stream: %s\n", stream_path);
    uint32_t drop_count = 0;
    struct vitriol_drop *drops = load_alkas(stream_path, &drop_count);
    if (!drops) return 1;

    printf("Stream: %u drops (%lu bytes)\n", drop_count,
           (unsigned long)(drop_count * sizeof(struct vitriol_drop)));

    /* Load azoth (optional) */
    uint32_t azoth_count = 0;
    struct vitriol_azoth *azoth = NULL;
    if (azoth_path) {
        azoth = load_azoth(azoth_path, &azoth_count);
        if (azoth)
            printf("Rollback: %u azoth packets\n", azoth_count);
    }

    /* Build vial struct for kernel */
    struct vitriol_vial vial = {0};
    if (vessel_count > 0) {
        struct vial_vessel *main_vessel = find_vessel_by_name("GPU_MAIN");
        if (!main_vessel) main_vessel = &vessels[0];

        vial.aperture_size = main_vessel->bar1_size;
        vial.aperture_max = main_vessel->bar1_max_window ?: main_vessel->bar1_size;
        vial.thermal_halt = main_vessel->thermal_halt;
        vial.thermal_throttle = main_vessel->thermal_throttle;
        vial.dma_capable = main_vessel->dma_capable;
        vial.cooperative = cooperative;
        if (gpu_va_str)
            vial.gpu_va = strtoull(gpu_va_str, NULL, 0);
        if (p2p_token_str)
            vial.p2p_token = strtoull(p2p_token_str, NULL, 0);
        if (va_space_token_str)
            vial.va_space_token = strtoul(va_space_token_str, NULL, 0);
    }

    /* Open device (skip if dry-run) */
    int fd = -1;
    if (!dry_run) {
        fd = open(device_path, O_RDWR);
        if (fd < 0) {
            fprintf(stderr, "Error: Cannot open %s: %s\n",
                    device_path, strerror(errno));
            fprintf(stderr, "Is the vitriol kernel module loaded?\n");
            free(drops);
            if (azoth) free(azoth);
            return 1;
        }
        printf("Device: %s (fd=%d)\n", device_path, fd);

        /* Send vial to kernel */
        int ret = ioctl(fd, VITRIOL_IOC_SET_VIAL, &vial);
        if (ret != 0) {
            fprintf(stderr, "Warning: Failed to set vial in kernel: %s\n",
                    strerror(errno));
        }

        /* Send source file to kernel */
        if (source_path && !dry_run) {
            int src_fd = open(source_path, O_RDONLY);
            if (src_fd < 0) {
                fprintf(stderr, "Warning: Cannot open source file %s: %s\n",
                        source_path, strerror(errno));
            } else {
                struct vitriol_source src = {0};
                src.fd = src_fd;
                ret = ioctl(fd, VITRIOL_IOC_SET_SOURCE, &src);
                if (ret != 0) {
                    fprintf(stderr, "Warning: Failed to set source file: %s\n",
                            strerror(errno));
                } else {
                    printf("Source: %s (fd=%d)\n", source_path, src_fd);
                }
            }
        }

        /* Bind PCI device if requested (via kernel workqueue IOCTL) */
        if (bind_bdf) {
            if (bind_pci_device(fd, bind_bdf) != 0) {
                fprintf(stderr, "Warning: BIND failed for %s, continuing anyway\n",
                        bind_bdf);
            }
        }

        /* Map BAR1 into kernel (after unbind, no PAT conflict) */
        if (map_bar) {
            if (ioctl(fd, VITRIOL_IOC_MAP_BAR) != 0) {
                fprintf(stderr, "Warning: MAP_BAR failed (nvidia still owns device?)\n");
            } else {
                printf("BAR1 mapped in kernel\n");
            }
        }

        /* Map BAR1 into userspace for FLOW data writes */
        setup_bar1_mmap(fd);
    }

    /* Execute */
    int ret = execute_stream(fd, drops, drop_count, dry_run);

    /* Rollback on failure */
    if (ret != 0 && azoth && azoth_count > 0 && fd >= 0) {
        execute_rollback(fd, azoth, azoth_count);
    }

    /* Cleanup */
    teardown_bar1_mmap();
    if (fd >= 0) close(fd);
    free(drops);
    if (azoth) free(azoth);

    return ret;
}
