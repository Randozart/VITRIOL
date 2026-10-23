/*
 * vitriol_ce_spike.cpp - Doorbell Phase 2 D2a recon harness
 *
 * Locate CUDA channel UserD + GPFIFO ring from userspace after a real
 * stream op. Read-only: no doorbell write in D2a. Abort gate = no
 * UserD/ring candidates found.
 *
 * Build: g++ -O2 -o vitriol_ce_spike vitriol_ce_spike.cpp -lcuda
 */
#include "vitriol_copy_engine_c6b5.h"

#include <cuda.h>

#include <csetjmp>
#include <csignal>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <map>
#include <string>
#include <sys/mman.h>
#include <sys/uio.h>
#include <unistd.h>
#include <vector>

/* Driver threads can munmap heaps between snapshot and scan (TOCTOU).
 * process_vm_readv returns EFAULT instead of SEGV. */
static ssize_t safe_read(uintptr_t addr, void *buf, size_t n)
{
    struct iovec li{buf, n}, ri{reinterpret_cast<void *>(addr), n};
    return process_vm_readv(getpid(), &li, 1, &ri, 1, 0);
}

/* GUP fails on device BAR maps (not normal pages). In-process read with
 * SEGV/BUS guard for our own mappings only - D2a stays read-only. */
static sigjmp_buf g_jmp;
static void segv_handler(int) { siglongjmp(g_jmp, 1); }

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

static uint64_t fnv(const uint8_t *p, size_t n)
{
    uint64_t h = 1469598103934665603ULL;
    for (size_t i = 0; i < n; i++) { h ^= p[i]; h *= 1099511628211ULL; }
    return h;
}

static std::string maps_snapshot()
{
    std::ifstream f("/proc/self/maps");
    return std::string((std::istreambuf_iterator<char>(f)),
                       std::istreambuf_iterator<char>());
}

struct mapping {
    uintptr_t lo, hi;
    char perms[8];
    std::string tag;
    bool operator<(const mapping &o) const { return lo < o.lo; }
};

static std::map<uintptr_t, mapping> parse_maps(const std::string &s)
{
    std::map<uintptr_t, mapping> out;
    FILE *fp = tmpfile();
    fwrite(s.data(), 1, s.size(), fp);
    rewind(fp);
    char line[512];
    while (fgets(line, sizeof line, fp)) {
        mapping m{};
        char tag[256] = "";
        if (sscanf(line, "%lx-%lx %7s %*s %*s %*s %255s",
                   &m.lo, &m.hi, m.perms, tag) >= 3) {
            m.tag = tag;
            out[m.lo] = m;
        }
    }
    fclose(fp);
    return out;
}

static void diff_maps(const char *label, const std::map<uintptr_t, mapping> &a,
                      const std::map<uintptr_t, mapping> &b,
                      std::vector<mapping> *added)
{
    printf("== %s: new/changed mappings ==\n", label);
    for (const auto &kv : b) {
        auto it = a.find(kv.first);
        if (it == a.end() || strcmp(it->second.perms, kv.second.perms) != 0) {
            printf("  %08lx-%08lx %s %s\n",
                   (unsigned long)kv.second.lo, (unsigned long)kv.second.hi,
                   kv.second.perms, kv.second.tag.c_str());
            if (added) added->push_back(kv.second);
        }
    }
}

/* UserD pattern: Put@0x40 Get@0x44 GPGet@0x88 GPPut@0x8C. Values are NOT
 * small ring offsets: Put/Get are GPU VAs (observed 0x02600xxx, 4-aligned),
 * GP pair are small sequence counters (observed 2,3). Idle after sync:
 * Put==Get, GPGet==GPPut. */
static bool looks_like_userd(const uint8_t *p, size_t n, uint32_t *out)
{
    if (n < 0xA0) return false;
    const uint32_t *w = (const uint32_t *)p;
    uint32_t put   = w[0x40 / 4];
    uint32_t get   = w[0x44 / 4];
    uint32_t gpget = w[0x88 / 4];
    uint32_t gpput = w[0x8C / 4];
    if (!(put || get || gpget || gpput)) return false; /* all-zero = not live */
    if (put != get || gpget != gpput) return false;     /* must be idle-equal */
    if (put % 4 != 0) return false;                     /* dword-aligned VA */
    if (put < 0x1000) return false;                     /* too small for GPU VA */
    if (gpget > (1u << 24)) return false;               /* counters not VAs */
    if (out) { out[0] = put; out[1] = get; out[2] = gpget; out[3] = gpput; }
    return true;
}

/* GPFIFO ring: 512 x 8-byte entries. entry1 length field (bits 30:10) in
 * plausible pushbuffer dwords for at least one entry; fetch bit legal. */
static bool looks_like_gpfifo(const uint8_t *p, size_t n, size_t *plen,
                              int *pnonzero)
{
    size_t entries = n / 8;
    if (entries < 64 || entries > 8192) return false;
    const uint32_t *w = (const uint32_t *)p;
    int plausible = 0, nonzero = 0;
    for (size_t i = 0; i < entries; i++) {
        uint32_t lo = w[i * 2];
        uint32_t hi = w[i * 2 + 1];
        if (lo || hi) nonzero++;
        uint32_t len = (hi >> 10) & 0x7FFFFF;
        uint32_t fetch = lo & 1;
        if (fetch > 1) return false;
        if (len > 0 && len <= 8192 && (lo >> 4)) plausible++;
    }
    if (plausible >= 1) { *plen = entries; *pnonzero = nonzero; return true; }
    /* all-zero windows are NOT evidence: zeroed heaps are everywhere */
    return false;
}

int main(void)
{
    setvbuf(stdout, NULL, _IONBF, 0);

    auto m0 = parse_maps(maps_snapshot());

    CUresult r;
    CUdevice dev = 0;
    if ((r = cuInit(0)) != CUDA_SUCCESS) { printf("cuInit fail %d\n", r); return 1; }
    if ((r = cuDeviceGet(&dev, 0)) != CUDA_SUCCESS) { printf("cuDeviceGet fail\n"); return 1; }
    CUcontext ctx = nullptr;
    if ((r = cuCtxCreate(&ctx, nullptr, 0, dev)) != CUDA_SUCCESS) { printf("cuCtxCreate fail %d\n", r); return 1; }

    auto m1 = parse_maps(maps_snapshot());
    std::vector<mapping> after_ctx;
    diff_maps("after cuCtxCreate", m0, m1, &after_ctx);

    CUstream st = nullptr;
    cuStreamCreate(&st, CU_STREAM_NON_BLOCKING);

    /* real op so the channel GPFIFO/Get/Put advance */
    void *h = nullptr;
    if (cuMemHostAlloc(&h, 1 << 20, CU_MEMHOSTALLOC_DEVICEMAP) != CUDA_SUCCESS) {
        printf("host alloc fail\n"); return 1;
    }
    memset(h, 0xA5, 1 << 20);
    CUdeviceptr d = 0;
    cuMemAlloc(&d, 1 << 20);
    cuMemcpyHtoDAsync(d, h, 1 << 20, st);
    cuStreamSynchronize(st);

    auto m2 = parse_maps(maps_snapshot());
    std::vector<mapping> after_op;
    diff_maps("after stream op", m1, m2, &after_op);

    /* Sliding 4KB-window scan: UserD page and GPFIFO ring sit at page
     * alignment INSIDE UMD heaps (12MB /dev/zero grew post-op in r1),
     * not at mapping base. Per-window process_vm_readv = sparse-safe,
     * driver-unmap-safe. */
    printf("== pattern scan (sliding 4KB windows) ==\n");
    int userd_hits = 0, ring_hits = 0;
    std::vector<mapping> all_new;
    for (auto &v : after_ctx) all_new.push_back(v);
    for (auto &v : after_op)  all_new.push_back(v);
    if (all_new.empty()) {
        for (auto &kv : m2) all_new.push_back(kv.second);
    }
    const size_t WIN = 0x1000;
    uint8_t win[WIN * 4];
    for (auto &m : all_new) {
        if (m.perms[0] != 'r' || m.perms[1] != 'w') continue;
        size_t n = m.hi - m.lo;
        if (n < WIN || n > (64UL << 20)) continue;
        int map_nnz = 0;
        bool big = n >= (1UL << 20);
        for (size_t off = 0; off + WIN <= n; off += WIN) {
            ssize_t got = dev_read(m.lo + off, win, WIN);
            if (got != (ssize_t)WIN) continue;
            uint32_t ud[4] = {};
            if (looks_like_userd(win, WIN, ud)) {
                printf("  USERD %08lx+%04zx (%s %s)  Put=%u Get=%u GPGet=%u GPPut=%u\n",
                       (unsigned long)m.lo, off, m.perms, m.tag.c_str(),
                       ud[0], ud[1], ud[2], ud[3]);
                userd_hits++;
            }
            /* GPFIFO ring may straddle two pages: read 4-page window */
            size_t wleft = n - off;
            size_t want = wleft < WIN * 4 ? wleft : WIN * 4;
            if (want >= WIN && dev_read(m.lo + off, win, want) == (ssize_t)want) {
                size_t entries = 0;
                int nz = 0;
                if (looks_like_gpfifo(win, want, &entries, &nz)) {
                    bool nvid = m.tag.find("nvidia") != std::string::npos;
                    if (nvid || ring_hits < 5)
                        printf("  GPFIFO %08lx+%04zx (%s %s)  entries=%zu nonzero=%d\n",
                               (unsigned long)m.lo, off, m.perms, m.tag.c_str(),
                               entries, nz);
                    ring_hits++;
                }
            }
            if (big) {
                const uint32_t *w = (const uint32_t *)win;
                for (size_t i = 0; i < WIN / 4; i++) if (w[i]) map_nnz++;
            }
        }
        if (big) {
            printf("  heap %08lx-%08lx %s %s nonzero_dwords=%d\n",
                   (unsigned long)m.lo, (unsigned long)m.hi, m.perms,
                   m.tag.c_str(), map_nnz);
            if (map_nnz) {
                /* dump first nonzero page head for structure guess */
                for (size_t off = 0; off + WIN <= n; off += WIN) {
                    if (safe_read(m.lo + off, win, WIN) != (ssize_t)WIN) continue;
                    const uint32_t *w = (const uint32_t *)win;
                    bool any = false;
                    for (size_t i = 0; i < WIN / 4; i++) if (w[i]) { any = true; break; }
                    if (any) {
                        printf("    first-nonzero @+%04zx:", off);
                        for (int i = 0; i < 24; i++) printf(" %08x", w[i]);
                        printf("\n");
                        break;
                    }
                }
            }
        }
    }
    /* Device-mapping phase: channel control / doorbell live in nvidia* maps,
     * not heap. Snapshot page hashes, second op, report what MOVES. */
    printf("== device-mapping phase ==\n");
    std::vector<mapping> devs;
    for (auto &kv : m2)
        if (kv.second.tag.find("nvidia") != std::string::npos)
            devs.push_back(kv.second);
    int mp_ok = 0, mp_fail = 0;
    for (auto &m : devs) {
        if (m.perms[0] == 'r') continue;
        if (mprotect((void *)m.lo, m.hi - m.lo, PROT_READ | PROT_WRITE) == 0) {
            m.perms[0] = 'r';
            mp_ok++;
            printf("  mprotect ok  %08lx %s (was %s)\n",
                   (unsigned long)m.lo, m.tag.c_str(), "-w-s/-w-p");
        } else {
            mp_fail++;
            printf("  mprotect fail %08lx %s errno=%d\n",
                   (unsigned long)m.lo, m.tag.c_str(), errno);
        }
    }
    printf("  nvidia maps=%zu mprotect_ok=%d fail=%d\n", devs.size(), mp_ok, mp_fail);

    struct pg { uintptr_t addr; uint64_t hash; uint8_t head[4096]; size_t map_lo, off; const char *tag; };
    std::vector<pg> snap;
    uint8_t buf[4096];
    for (auto &m : devs) {
        if (m.perms[0] != 'r') continue;
        size_t n = m.hi - m.lo;
        for (size_t off = 0; off + 64 <= n; off += 4096) {
            size_t want = n - off < 4096 ? n - off : 4096;
            if (dev_read(m.lo + off, buf, want) != (ssize_t)want) continue;
            pg p{};
            p.addr = m.lo + off;
            p.hash = fnv(buf, want);
            memcpy(p.head, buf, want); /* full page: enable dword-level diff */
            p.map_lo = m.lo; p.off = off; p.tag = m.tag.c_str();
            snap.push_back(p);
        }
    }
    printf("  snapshotted pages=%zu\n", snap.size());

    cuMemcpyHtoDAsync(d, h, 1 << 20, st);
    cuStreamSynchronize(st);

    int changed = 0, live_ud = 0;
    for (auto &p : snap) {
        size_t want = 4096;
        if (p.off + want > 0 && dev_read(p.addr, buf, want) != (ssize_t)want) continue;
        if (fnv(buf, want) == p.hash) continue;
        if (changed < 32) {
            printf("  CHANGED %s %08lx+%04zx\n", p.tag,
                   (unsigned long)p.map_lo, p.off);
            const uint32_t *was = (const uint32_t *)p.head;
            const uint32_t *now = (const uint32_t *)buf;
            int diffs = 0;
            size_t ndw = want / 4;
            for (size_t i = 0; i < ndw && diffs < 32; i++) {
                if (was[i] != now[i]) {
                    printf("    +%03zx: %08x -> %08x\n", i * 4, was[i], now[i]);
                    diffs++;
                }
            }
        }
        changed++;
        /* sliding userd check on 'now' page, not just fixed layout */
        for (size_t o = 0; o + 0xA0 <= want; o += 4) {
            uint32_t ud[4] = {};
            if (looks_like_userd(buf + o, want - o, ud)) {
                printf("  LIVE userd %08lx+%04zx+%03zx  Put=%u Get=%u GPGet=%u GPPut=%u\n",
                       (unsigned long)p.map_lo, p.off, o, ud[0], ud[1], ud[2], ud[3]);
                live_ud++;
                break;
            }
        }
    }
    printf("  changed_pages=%d live_userd=%d\n", changed, live_ud);

    printf("RESULT userd=%d live_userd=%d gpfifo=%d changed_pages=%d\n",
           userd_hits, live_ud, ring_hits, changed);
    printf("%s\n", (live_ud > 0 && ring_hits > 0) ? "D2a_PASS" : "D2a_FAIL");

    cuMemFree(d);
    cuMemFreeHost(h);
    cuStreamDestroy(st);
    cuCtxDestroy(ctx);
    return 0;
}
