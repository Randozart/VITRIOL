/*
 * vitriol_ce_d2b.cpp - Doorbell Phase 2 write spike (B2)
 *
 * Submit path mirrors UMD byte-for-byte after r1 learnings:
 *   - method header: (1<<29)|(cnt<<16)|(subch<<13)|(method>>2), subch=4
 *   - methods written into channel PB via nvidiactl BAR alias (VA bias
 *     discovered by page-hash diff across a warmup op)
 *   - GPFIFO entry raw VA, Put/GPPut advance, doorbell @ UserD+0x90
 *   - fence = C6B5 semaphore release to pinned metapage
 *
 * Build: g++ -O2 -o vitriol_ce_d2b vitriol_ce_d2b.cpp -lcuda
 */
#include "vitriol_copy_engine_c6b5.h"

#include <cuda.h>

#include <csetjmp>
#include <csignal>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <fstream>
#include <string>
#include <sys/mman.h>
#include <sys/uio.h>
#include <unistd.h>
#include <vector>

#define FENCE_MAGIC     0x56495452u /* 'VITR' */
#define FENCE_TIMEOUT_MS 3000
#define CE_SUBCHAN      4u
#define LAUNCH_COPY_SEM1W \
    (2u /* NON_PIPELINED */ | (1u << 3) /* SEM_REL_1W */ | \
     (1u << 7) /* SRC_PITCH */ | (1u << 8) /* DST_PITCH */)

#define HDR(cnt, meth) \
    ((1u << 29) | ((uint32_t)(cnt) << 16) | (CE_SUBCHAN << 13) | ((meth) >> 2))

static sigjmp_buf g_jmp;
static void segv_handler(int) { siglongjmp(g_jmp, 1); }

static ssize_t safe_read(uintptr_t addr, void *buf, size_t n)
{
    struct iovec li{buf, n}, ri{reinterpret_cast<void *>(addr), n};
    return process_vm_readv(getpid(), &li, 1, &ri, 1, 0);
}

static ssize_t dev_read(uintptr_t addr, void *buf, size_t n)
{
    ssize_t got = safe_read(addr, buf, n);
    if (got == (ssize_t)n) return got;
    struct sigaction sa{}, old_segv{}, old_bus{};
    sa.sa_handler = segv_handler;
    sigemptyset(&sa.sa_mask);
    sigaction(SIGSEGV, &sa, &old_segv);
    sigaction(SIGBUS, &sa, &old_bus);
    if (sigsetjmp(g_jmp, 1) == 0) memcpy(buf, (const void *)addr, n);
    else n = 0;
    sigaction(SIGSEGV, &old_segv, nullptr);
    sigaction(SIGBUS, &old_bus, nullptr);
    return (ssize_t)n;
}

static ssize_t dev_write(uintptr_t addr, const void *buf, size_t n)
{
    struct sigaction sa{}, old_segv{}, old_bus{};
    sa.sa_handler = segv_handler;
    sigemptyset(&sa.sa_mask);
    sigaction(SIGSEGV, &sa, &old_segv);
    sigaction(SIGBUS, &sa, &old_bus);
    if (sigsetjmp(g_jmp, 1) == 0) memcpy((void *)addr, buf, n);
    else n = 0;
    sigaction(SIGSEGV, &old_segv, nullptr);
    sigaction(SIGBUS, &old_bus, nullptr);
    return (ssize_t)n;
}

struct mapping {
    uintptr_t lo, hi;
    char perms[8];
    std::string tag;
};

static std::vector<mapping> parse_maps()
{
    std::vector<mapping> out;
    std::ifstream f("/proc/self/maps");
    std::string line;
    while (std::getline(f, line)) {
        mapping m{};
        char tag[256] = "";
        unsigned long lo = 0, hi = 0;
        if (sscanf(line.c_str(), "%lx-%lx %7s %*s %*s %*s %255s",
                   &lo, &hi, m.perms, tag) >= 3) {
            m.lo = lo; m.hi = hi; m.tag = tag;
            out.push_back(m);
        }
    }
    return out;
}

static bool looks_like_userd(const uint8_t *p, size_t n, uint32_t *put,
                             uint32_t *gp)
{
    if (n < 0xA0) return false;
    const uint32_t *w = (const uint32_t *)p;
    uint32_t p40 = w[0x40 / 4], g44 = w[0x44 / 4];
    uint32_t pg = w[0x88 / 4], pp = w[0x8C / 4];
    if (!(p40 || g44 || pg || pp)) return false;
    if (p40 != g44 || pg != pp) return false;
    if (p40 % 4 || p40 < 0x1000) return false;
    if (pg > (1u << 24)) return false;
    if (put) *put = p40;
    if (gp) *gp = pg;
    return true;
}

static uint64_t now_ms()
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
}

static int fail(const char *why)
{
    printf("D2b_FAIL %s\n", why);
    return 1;
}

static uint64_t fnv(const void *d, size_t n)
{
    const uint8_t *p = (const uint8_t *)d;
    uint64_t h = 1469598103934665603ull;
    for (size_t i = 0; i < n; i++) { h ^= p[i]; h *= 1099511628211ull; }
    return h;
}

int main(void)
{
    setvbuf(stdout, NULL, _IONBF, 0);

    cuInit(0);
    CUdevice dev = 0;
    cuDeviceGet(&dev, 0);
    CUcontext ctx = nullptr;
    if (cuCtxCreate(&ctx, nullptr, 0, dev) != CUDA_SUCCESS)
        return fail("ctx");
    CUstream st = nullptr;
    cuStreamCreate(&st, CU_STREAM_NON_BLOCKING);

    void *wh = nullptr;
    if (cuMemHostAlloc(&wh, 1 << 20, CU_MEMHOSTALLOC_DEVICEMAP) != CUDA_SUCCESS)
        return fail("hostalloc");
    memset(wh, 0xA5, 1 << 20);
    CUdeviceptr wd = 0;
    cuMemAlloc(&wd, 1 << 20);
    cuMemcpyHtoDAsync(wd, wh, 1 << 20, st);
    cuStreamSynchronize(st);

    /* ── locate live UserD + ring (sliding scan of -w-s/-rw-s maps) ── */
    uint8_t page[4096], page2[4096];
    uintptr_t userd_map = 0, userd_off = 0;
    uint32_t put0 = 0, gp0 = 0;
    std::vector<mapping> maps = parse_maps();
    for (auto &m : maps) {
        if (m.tag.find("nvidia0") == std::string::npos) continue;
        if (m.hi - m.lo > 8 * 1024 * 1024) continue;
        if (m.perms[0] != 'r') {
            if (mprotect((void *)m.lo, m.hi - m.lo, PROT_READ | PROT_WRITE) != 0)
                continue;
        }
        for (size_t off = 0; off + 0xA0 <= m.hi - m.lo; off += 4096) {
            if (dev_read(m.lo + off, page, 4096) != 4096) continue;
            uint32_t pu = 0, gp = 0;
            if (!looks_like_userd(page, 4096, &pu, &gp)) continue;
            userd_map = m.lo; userd_off = off; put0 = pu; gp0 = gp;
            cuMemcpyHtoDAsync(wd, wh, 1 << 20, st);
            cuStreamSynchronize(st);
            uint32_t pu2 = 0, gp2 = 0;
            if (dev_read(m.lo + off, page2, 4096) == 4096 &&
                looks_like_userd(page2, 4096, &pu2, &gp2) &&
                (pu2 != pu || gp2 != gp)) {
                printf("P0 live UserD %08lx+%04zx Put=%u GP=%u -> %u/%u\n",
                       (unsigned long)m.lo, off, put0, gp0, pu2, gp2);
                goto found_userd;
            }
            userd_map = 0;
        }
    }
    return fail("no-live-userd");
found_userd:;
    nvc0_userd_t *ud = (nvc0_userd_t *)(userd_map + userd_off);
    /* Doorbell = clientRegion (WRITE_ONLY NVC361 map, nv_gpu_ops.c:5696)
     * at +NVC361_NOTIFY_CHANNEL_PENDING(0x90); token written by UMD/RM,
     * likely readable after mprotect. Also try UserD+0x90. */
    uintptr_t dbell_map = 0;
    for (auto &m : maps) {
        if (m.perms[0] == '-' && m.perms[1] == 'w' &&
            m.tag.find("nvidia0") != std::string::npos &&
            m.hi - m.lo <= 128 * 1024) {
            if (mprotect((void *)m.lo, m.hi - m.lo, PROT_READ | PROT_WRITE) != 0)
                continue;
            dbell_map = m.lo;
            break;
        }
    }
    uint32_t token = 0, token_ud = 0;
    if (dbell_map) {
        dev_read(dbell_map + USERD_DOORBELL_OFFSET, &token, 4);
        uint32_t dump[40];
        dev_read(dbell_map, dump, sizeof(dump));
        printf("P0 clientRegion %08lx doorbell+90=%08x\n",
               (unsigned long)dbell_map, token);
        for (int i = 0; i < 40; i++)
            if (dump[i]) printf("  [%03x] %08x\n", i * 4, dump[i]);
    }
    dev_read(userd_map + userd_off + USERD_DOORBELL_OFFSET, &token_ud, 4);
    printf("P0 userd+90=%08x clientRegion_db=%08x\n", token_ud, token);

    uintptr_t ring_base = userd_map + userd_off - 0x2000;
    if (dev_read(ring_base, page, 4096) != 4096) return fail("ring-read");
    {
        uint32_t lo0 = ((uint32_t *)page)[0];
        uint32_t hi0 = ((uint32_t *)page)[1];
        size_t len0 = (hi0 >> 10) & 0x7FFFFF;
        if (!(lo0 % 4 == 0 && lo0 >= 0x1000 && len0 > 0 && len0 < 8192))
            return fail("ring-pattern");
    }
    printf("P0 ring=%08lx gpput=%u\n", (unsigned long)ring_base, ud->GPPut);

    /* ── locate channel PB CPU alias: hash nvidiactl pages, diff ──── */
    std::vector<mapping> ctl_maps;
    for (auto &m : maps)
        if (m.tag.find("nvidiactl") != std::string::npos &&
            m.hi - m.lo >= 4096 && m.hi - m.lo <= 64ul * 1024 * 1024)
            ctl_maps.push_back(m);
    if (ctl_maps.empty()) return fail("no-nvidiactl");

    std::vector<uint64_t> before;
    for (auto &m : ctl_maps) {
        for (uintptr_t a = m.lo; a < m.hi; a += 4096) {
            if (dev_read(a, page, 4096) == 4096) before.push_back(fnv(page, 4096));
            else before.push_back(0);
        }
    }
    uint32_t put_before_map = ud->Put;
    cuMemcpyHtoDAsync(wd, wh, 1 << 20, st);
    cuStreamSynchronize(st);

    uintptr_t pb_page_va = 0;
    size_t pb_page_off = 0;
    {
        size_t idx = 0;
        for (auto &m : ctl_maps) {
            for (uintptr_t a = m.lo; a < m.hi; a += 4096, idx++) {
                if (dev_read(a, page, 4096) != 4096) { idx += 0; continue; }
                if (fnv(page, 4096) == before[idx]) continue;
                /* changed page: write must start at Put_before & 0xfff */
                size_t want = put_before_map & 0xFFF;
                const uint32_t *w = (const uint32_t *)page;
                /* accept if dword at `want` looks like method header subch4 */
                uint32_t hdr = w[want / 4];
                uint32_t op = hdr >> 29, subch = (hdr >> 13) & 7;
                printf("P0 PB changed %08lx+%zx hdr@+%x=%08x (op=%u subch=%u)\n",
                       (unsigned long)m.lo, a - m.lo, want, hdr, op, subch);
                if (op == 1 && subch == CE_SUBCHAN) {
                    pb_page_va = a;
                    pb_page_off = a - m.lo;
                    goto pb_found;
                }
            }
        }
    }
    return fail("no-pb-page");
pb_found:;
    /* cpu_addr(va) = ctl_base + (va - bias); derive bias from Put */
    mapping *pb_map = nullptr;
    for (auto &m : ctl_maps)
        if (m.lo <= pb_page_va && pb_page_va < m.hi) { pb_map = &m; break; }
    if (!pb_map) return fail("pb-map");
    uintptr_t put_cpu = pb_page_va + (put_before_map & 0xFFF);
    uint32_t va_bias = put_before_map - (uint32_t)(pb_map->lo - pb_map->lo);
    /* bias: va -> bar-relative offset. bar_off(put) = (pb_page_va-map.lo)
     * + (put&0xfff); store map + bar delta directly instead. */
    uint64_t bias = (uint64_t)put_before_map -
                    ((uint64_t)(pb_page_va - pb_map->lo) +
                     (put_before_map & 0xFFF));
    printf("P0 pb map=%08lx bias=0x%llx put_cpu=%08lx\n",
           (unsigned long)pb_map->lo, (unsigned long long)bias,
           (unsigned long)put_cpu);
    (void)va_bias; (void)pb_page_off;

    auto pb_cpu_addr = [&](uint32_t va) -> uintptr_t {
        return pb_map->lo + ((uint64_t)va - bias);
    };

    /* dump decoded UMD packet at current Put for confirmation */
    {
        uint32_t dump[32];
        uintptr_t a = pb_cpu_addr(ud->Put - ud->Put + put_before_map);
        /* dump at block that was just written: Put advanced by ~60B */
        a = pb_cpu_addr(ud->Get - 60 > 0 ? ud->Get : put_before_map);
        if (dev_read(a, dump, sizeof(dump)) == (ssize_t)sizeof(dump)) {
            printf("P0 UMD packet @%08lx:\n", (unsigned long)a);
            for (int i = 0; i < 16; i++)
                printf("  [%02x] %08x\n", i * 4, dump[i]);
        }
    }

    /* ── phase 1: doorbell smoke ─────────────────────────────────── */
    if (!dbell_map) return fail("no-clientregion");
    uint32_t kick = token ? token : 1;
    if (dev_write(dbell_map + USERD_DOORBELL_OFFSET, &kick, 4) != 4)
        return fail("doorbell-unwritable");
    cuMemcpyHtoDAsync(wd, wh, 1 << 20, st);
    if (cuStreamSynchronize(st) != CUDA_SUCCESS)
        return fail("channel-dead-after-kick");
    printf("P1 kick ok Put=%u GP=%u\n", ud->Put, ud->GPPut);

    /* wait channel idle before hijack submit */
    for (int i = 0; i < 100000 && !(ud->Put == ud->Get &&
                                     ud->GPGet == ud->GPPut); i++)
        ;

    /* ── phase 2: CE DMA via channel PB ──────────────────────────── */
    const uint32_t XFER = 1 << 20;
    CUdeviceptr src_gva = 0, dst_gva = 0;
    if (cuMemAlloc(&src_gva, XFER) != CUDA_SUCCESS) return fail("src");
    if (cuMemAlloc(&dst_gva, XFER) != CUDA_SUCCESS) return fail("dst");
    std::vector<uint32_t> pattern(XFER / 4);
    for (uint32_t i = 0; i < XFER / 4; i++) pattern[i] = 0xC0DE0000u + i;
    cuMemcpyHtoD(src_gva, pattern.data(), XFER);

    void *fence_cpu = nullptr;
    CUdeviceptr fence_gva = 0;
    if (cuMemHostAlloc(&fence_cpu, 64, CU_MEMHOSTALLOC_DEVICEMAP) != CUDA_SUCCESS)
        return fail("fence");
    cuMemHostGetDevicePointer(&fence_gva, fence_cpu, 0);
    *(volatile uint32_t *)fence_cpu = 0;

    uint32_t pb[32];
    int n = 0;
    pb[n++] = HDR(4, 0x400);
    pb[n++] = (uint32_t)((src_gva >> 32) & 0x1FFFF);
    pb[n++] = (uint32_t)(src_gva & 0xFFFFFFFF);
    pb[n++] = (uint32_t)((dst_gva >> 32) & 0x1FFFF);
    pb[n++] = (uint32_t)(dst_gva & 0xFFFFFFFF);
    pb[n++] = HDR(4, 0x410);
    pb[n++] = XFER; /* pitch_in */
    pb[n++] = XFER; /* pitch_out */
    pb[n++] = XFER; /* line_length */
    pb[n++] = 1;    /* line_count */
    pb[n++] = HDR(3, 0x240);
    pb[n++] = (uint32_t)((fence_gva >> 32) & 0x1FFFF);
    pb[n++] = (uint32_t)(fence_gva & 0xFFFFFFFF);
    pb[n++] = FENCE_MAGIC;
    pb[n++] = HDR(1, 0x300);
    pb[n++] = LAUNCH_COPY_SEM1W;

    uint32_t put_old = ud->Put;
    uint32_t idx = ud->GPPut;
    if (idx >= CE_GPFIFO_ENTRIES) return fail("gpput-oob");
    uintptr_t pb_cpu = pb_cpu_addr(put_old);
    if (dev_write(pb_cpu, pb, n * 4) != n * 4) return fail("pb-write");

    nvc0_gpfifo_entry_t ent;
    ent.addr_lo = put_old; /* raw VA, fetch=0 (empirical) */
    ent.addr_hi_priv_level_len_sync = (uint32_t)n << 10;
    if (dev_write(ring_base + (uintptr_t)idx * 8, &ent, 8) != 8)
        return fail("ring-write");

    __sync_synchronize();
    ud->Put = put_old + (uint32_t)n * 4;
    __sync_synchronize();
    ud->GPPut = idx + 1;
    __sync_synchronize();
    uint32_t kick2 = token ? token : 1;
    if (dev_write(dbell_map + USERD_DOORBELL_OFFSET, &kick2, 4) != 4)
        return fail("doorbell2");
    printf("P2 submitted idx=%u put %u->%u dwords=%d pb_cpu=%08lx\n",
           idx, put_old, ud->Put, n, (unsigned long)pb_cpu);

    uint64_t t0 = now_ms();
    uint32_t fv = 0;
    while ((fv = *(volatile uint32_t *)fence_cpu) != FENCE_MAGIC) {
        if (now_ms() - t0 > FENCE_TIMEOUT_MS) {
            printf("P2 TIMEOUT fence=%08x Put=%u Get=%u GPGet=%u GPPut=%u\n",
                   fv, ud->Put, ud->Get, ud->GPGet, ud->GPPut);
            _exit(fail("fence-timeout"));
        }
    }
    printf("P2 fence ok in %llums Put=%u Get=%u GP=%u\n",
           (unsigned long long)(now_ms() - t0), ud->Put, ud->Get, ud->GPGet);

    std::vector<uint32_t> back(XFER / 4);
    if (cuMemcpyDtoH(back.data(), dst_gva, XFER) != CUDA_SUCCESS)
        return fail("verify-copy");
    for (uint32_t i = 0; i < XFER / 4; i++) {
        if (back[i] != 0xC0DE0000u + i) {
            printf("P2 data mismatch at %u: %08x\n", i, back[i]);
            return fail("verify-data");
        }
    }
    printf("P2 data ok (1MiB verified)\n");
    printf("D2b_PASS\n");

    cuMemFree(dst_gva);
    cuMemFree(src_gva);
    cuMemFreeHost(fence_cpu);
    cuMemFree(wd);
    cuMemFreeHost(wh);
    cuStreamDestroy(st);
    cuCtxDestroy(ctx);
    return 0;
}
