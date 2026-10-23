/*
 * vitriol-buffer.cpp — VITRIOL buffer type for expert tensor placement
 *
 * RAM Shot:
 *   - Allocates hugepage-backed system RAM via mmap
 *   - mlock() pins it — never swapped
 *   - cudaHostRegister() page-locks it — GPU reads directly over PCIe DMA
 *   - Reports is_host=true — scheduler routes MUL_MAT_ID to CUDA
 *   - Set_tensor copies expert data normally into the buffer
 *
 * Expert weights live in page-locked host RAM. The GPU's Copy Engine
 * streams them across PCIe during MUL_MAT_ID kernels. No VRAM used for
 * weight storage — only for activations, KV cache, and compute buffers.
 */

#include "vitriol-buffer.h"
#include "vitriol-cuda-integration.h"
#include "vitriol-store.h"
#include <cstdio>
#include <cstdlib>

/* Rust backend ABI (defined in vitriol-cuda-integration.cpp). */
extern "C" {
int vs_open_store(const char * path);
int vs_init(void);
}
bool vitriol_rust_enabled(void);
#include <cstring>
#include <mutex>
#include <string>
#include <sys/mman.h>
#include <unistd.h>
#include <fcntl.h>
#include <cuda_runtime.h>

/* ── Buffer type interface ────────────────────────────────────── */

struct vitriol_buffer_type_context {
    int device;
    std::string name;
};

static const char * vitriol_buffer_type_get_name(ggml_backend_buffer_type_t buft) {
    auto * ctx = (vitriol_buffer_type_context *)buft->context;
    return ctx->name.c_str();
}

/* Check if a buffer type is VITRIOL by checking the context pointer range */
bool vitriol_is_vitriol_buffer_type(ggml_backend_buffer_type_t buft) {
    if (!buft || !buft->context) return false;
    /* VITRIOL context objects are allocated in the static 'types' array
     * in vitriol_get_buffer_type(). We check if the context pointer is
     * within our known range by matching the name string. Since get_name
     * now points to CUDA host's function, we check the context name. */
    auto * ctx = (vitriol_buffer_type_context *)buft->context;
    return ctx->name == "VITRIOL" || ctx->name == "VITRIOL_LAZY";
}

/* VITRIOL buffer context */
struct vitriol_buffer_context {
    void * base;
    size_t size;
    vitriol::store * store = nullptr;
    ggml_backend_buffer_t buffer = nullptr;
    bool base_mlocked = false;
    bool base_registered = false;
    /* false for mmap-wrapped regions (vitriol_buffer_from_host_ptr): we
     * registered the pages for DMA but do not own the mapping itself */
    bool owns_mapping = true;
};

/* ── Atexit cleanup guard ──────────────────────────────────────── */
/* If the process exits without vitriol_buffer_free being called
 * (e.g., SIGTERM during model load, or exit() before ggml teardown),
 * this handler ensures cudaHostUnregister + munlock + munmap still run.
 * This prevents the NVIDIA driver from retaining DMA mappings that
 * would hang the system on shutdown and trigger fsck on next boot. */

static vitriol_buffer_context * g_pending_cleanup = nullptr;
static std::mutex              g_cleanup_mtx;

static void vitriol_buffer_atexit_cleanup(void) {
    std::lock_guard<std::mutex> lock(g_cleanup_mtx);
    if (!g_pending_cleanup || !g_pending_cleanup->base) return;
    auto * ctx = g_pending_cleanup;
    if (ctx->store) vitriol_store_release_buffer(ctx->buffer, ctx->base, ctx->size);
    if (ctx->base_registered) {
        cudaHostUnregister(ctx->base);
    }
    if (ctx->base_mlocked) {
        munlock(ctx->base, ctx->size);
    }
    munmap(ctx->base, ctx->size);
    ctx->base = nullptr;
    ctx->buffer = nullptr;
    ctx->base_mlocked = false;
    ctx->base_registered = false;
    if (ctx->store) {
        vitriol::store_close(ctx->store);
        ctx->store = nullptr;
    }
}

static void vitriol_buffer_free(ggml_backend_buffer_t buffer) {
    auto * ctx = (vitriol_buffer_context *)buffer->context;
    if (ctx->store) vitriol_store_release_buffer(buffer, ctx->base, ctx->size);
    {
        std::lock_guard<std::mutex> lock(g_cleanup_mtx);
        if (g_pending_cleanup == ctx) {
            g_pending_cleanup = nullptr;
        }
    }
    if (ctx->base && ctx->size) {
        if (ctx->base_registered) {
            cudaHostUnregister(ctx->base);
        }
        if (ctx->base_mlocked) {
            munlock(ctx->base, ctx->size);
        }
        if (ctx->owns_mapping) {
            munmap(ctx->base, ctx->size);
        }
    }
    if (ctx->store) {
        vitriol::store_close(ctx->store);
    }
    delete ctx;
}

static void * vitriol_buffer_get_base(ggml_backend_buffer_t buffer) {
    auto * ctx = (vitriol_buffer_context *)buffer->context;
    return ctx->base;
}

static bool vitriol_name_is_expert(const char * name) {
    return name && (strstr(name, "ffn_up_exps") || strstr(name, "ffn_gate_exps") ||
                    strstr(name, "ffn_down_exps"));
}

static void vitriol_buffer_set_tensor(ggml_backend_buffer_t buffer, struct ggml_tensor * tensor, const void * data, size_t offset, size_t size) {
    GGML_ASSERT(tensor);
    if (g_vitriol_config.store_backed && vitriol_name_is_expert(tensor->name)) {
        const bool bound = vitriol_store_bind_tensor(
            buffer, tensor->data, tensor->name, ggml_nbytes(tensor), tensor->nb[2]);
        GGML_ASSERT(bound);
        /* VITRIOL_STORE_HYBRID=1: fill the host buffer from the GGUF anyway
         * (like the gguf source arm) so the process memory map and buffer
         * content are identical across arms; the LRU still fills from the
         * store. Diagnostic for isolating zeroed-buffer effects. */
        static const bool hybrid = getenv("VITRIOL_STORE_HYBRID") != nullptr;
        if (vitriol_store_source_device() && !hybrid) return;
    }
    memcpy((char *)tensor->data + offset, data, size);
    GGML_UNUSED(buffer);
}

static void vitriol_buffer_get_tensor(ggml_backend_buffer_t buffer, const struct ggml_tensor * tensor, void * data, size_t offset, size_t size) {
    GGML_ASSERT(tensor);
    memcpy(data, (const char *)tensor->data + offset, size);
    GGML_UNUSED(buffer);
}

static void vitriol_buffer_set_tensor_2d(ggml_backend_buffer_t buffer, struct ggml_tensor * tensor, const void * data,
        size_t offset, size_t size, size_t n_copies, size_t stride_tensor, size_t stride_data) {
    GGML_ASSERT(tensor);
    char * base = (char *)tensor->data + offset;
    const char * src = (const char *)data;
    for (size_t i = 0; i < n_copies; i++) {
        memcpy(base + i * stride_tensor, src + i * stride_data, size);
    }
    GGML_UNUSED(buffer);
}

static void vitriol_buffer_clear(ggml_backend_buffer_t buffer, uint8_t value) {
    auto * ctx = (vitriol_buffer_context *)buffer->context;
    memset(ctx->base, value, ctx->size);
}

static const struct ggml_backend_buffer_i vitriol_buffer_interface = {
    /* .free_buffer    = */ vitriol_buffer_free,
    /* .get_base       = */ vitriol_buffer_get_base,
    /* .init_tensor    = */ nullptr,
    /* .memset_tensor  = */ nullptr,
    /* .set_tensor     = */ vitriol_buffer_set_tensor,
    /* .get_tensor     = */ vitriol_buffer_get_tensor,
    /* .set_tensor_2d  = */ vitriol_buffer_set_tensor_2d,
    /* .get_tensor_2d  = */ nullptr,
    /* .cpy_tensor     = */ nullptr,
    /* .clear          = */ vitriol_buffer_clear,
    /* .reset          = */ nullptr,
};

/* ── Buffer type allocation ───────────────────────────────────── */

static bool vitriol_buffer_type_is_host(ggml_backend_buffer_type_t buft) {
    return true;  // page-locked host RAM (or disk offload with cudaHostRegister), GPU via DMA
}

static ggml_backend_buffer_t vitriol_buffer_type_alloc_buffer(ggml_backend_buffer_type_t buft, size_t size) {
    bool disk_offload = g_vitriol_config.disk_offload;
    bool store_backed = g_vitriol_config.store_backed;
    void * ptr = nullptr;
    vitriol::store * store = nullptr;
    bool base_mlocked = false;
    bool base_registered = false;

    if (store_backed) {
        /* Store-backed: pinned host RAM filled from the raw-zone store on
         * demand (vitriol_ensure_expert_locked reads via store_read_expert).
         * The store handle is kept in the buffer context for those reads. */
        const char * spath = vitriol_store_path();
        if (!spath) {
            fprintf(stderr, "VITRIOL: store_backed but no store path\n");
            return nullptr;
        }
        auto o = vitriol::store_open(spath, O_RDONLY);
        if (!o) {
            fprintf(stderr, "VITRIOL: store_open(%s) failed: %d\n", spath, (int)o.error());
            return nullptr;
        }
        store = *o;
        if (g_vitriol_config.verbose)
            printf("VITRIOL: store-backed buffer (tensors=%u embed=%llu)\n",
                   vitriol::store_tensor_count(store),
                   (unsigned long long)vitriol::store_embed_bytes(store));
        /* pinned host RAM, same as the stream-mode anon branch below */
        ptr = mmap(NULL, size, PROT_READ | PROT_WRITE,
                   MAP_PRIVATE | MAP_ANONYMOUS | MAP_POPULATE, -1, 0);
        if (ptr == MAP_FAILED) {
            fprintf(stderr, "VITRIOL: store mmap(%zu) failed: %m\n", size);
            vitriol::store_close(store);
            return nullptr;
        }
        {
            volatile char * touch = (volatile char *)ptr;
            for (size_t offset = 0; offset < size; offset += 4096) {
                touch[offset] = 0;
            }
        }
        madvise(ptr, size, MADV_HUGEPAGE);
        if (mlock(ptr, size) != 0) {
            fprintf(stderr, "VITRIOL: store mlock(%zu) failed: %m\n", size);
        } else {
            base_mlocked = true;
        }
        cudaError_t err = cudaHostRegister(ptr, size, 0);
        if (err != cudaSuccess) {
            fprintf(stderr, "VITRIOL: store cudaHostRegister(%zu) failed: %s\n",
                    size, cudaGetErrorString(err));
            if (base_mlocked) munlock(ptr, size);
            munmap(ptr, size);
            vitriol::store_close(store);
            return nullptr;
        }
        base_registered = true;
    } else if (disk_offload) {
        const char * model_path = getenv("VITRIOL_MODEL_PATH");
        if (!model_path || !*model_path) {
            fprintf(stderr, "VITRIOL: disk offload requires VITRIOL_MODEL_PATH\n");
            return nullptr;
        }
        int fd = open(model_path, O_RDONLY);
        if (fd < 0) {
            fprintf(stderr, "VITRIOL: cannot open model %s: %m\n", model_path);
            return nullptr;
        }
        off_t file_size = lseek(fd, 0, SEEK_END);
        if (file_size < 0) {
            fprintf(stderr, "VITRIOL: lseek failed: %m\n");
            close(fd);
            return nullptr;
        }
        if ((size_t)file_size < size) {
            fprintf(stderr, "VITRIOL: file smaller (%lld) than requested (%zu)\n",
                    (long long)file_size, size);
            close(fd);
            return nullptr;
        }
        ptr = mmap(NULL, size, PROT_READ | PROT_WRITE,
                   MAP_PRIVATE | MAP_POPULATE, fd, 0);
        close(fd);
        if (ptr == MAP_FAILED) {
            fprintf(stderr, "VITRIOL: file mmap(%zu) failed: %m\n", size);
            return nullptr;
        }
        madvise(ptr, size, MADV_HUGEPAGE);

        /* cudaHostRegister pins pages for GPU DMA access.
         * No mlock — pages can be evicted by OS if not accessed by GPU. */
        cudaError_t err = cudaHostRegister(ptr, size, 0);
        if (err != cudaSuccess) {
            fprintf(stderr, "VITRIOL: disk offload cudaHostRegister(%zu) failed: %s\n",
                    size, cudaGetErrorString(err));
            munmap(ptr, size);
            return nullptr;
        }
        base_registered = true;

        if (g_vitriol_config.verbose)
            printf("VITRIOL: disk offload — file-backed mmap (%zu MiB)\n",
                   size / 1024 / 1024);
    } else {
        ptr = mmap(NULL, size, PROT_READ | PROT_WRITE,
                   MAP_PRIVATE | MAP_ANONYMOUS | MAP_POPULATE, -1, 0);
        if (ptr == MAP_FAILED) {
            fprintf(stderr, "VITRIOL: mmap(%zu) failed: %m\n", size);
            return nullptr;
        }

        /* Touch each page to ensure it's backed by physical RAM */
        {
            volatile char * touch = (volatile char *)ptr;
            for (size_t offset = 0; offset < size; offset += 4096) {
                touch[offset] = 0;
            }
        }

        /* Hint: coalesce into transparent hugepages (2 MB) for lower TLB pressure */
        madvise(ptr, size, MADV_HUGEPAGE);

        /* Pin in RAM — never swap */
        if (mlock(ptr, size) != 0) {
            fprintf(stderr, "VITRIOL: mlock(%zu) failed: %m — continuing without mlock (may cause swap stutter)\n", size);
        } else {
            base_mlocked = true;
        }

        /* Register for GPU DMA access — makes it accessible from CUDA kernels */
        cudaError_t err = cudaHostRegister(ptr, size, 0);
        if (err != cudaSuccess) {
            fprintf(stderr, "VITRIOL: cudaHostRegister(%zu) failed: %s\n", size, cudaGetErrorString(err));
            munmap(ptr, size);
            return nullptr;
        }
        base_registered = true;
    }

    auto * ctx = new vitriol_buffer_context{ptr, size};
    ctx->store = store;
    ctx->base_mlocked = base_mlocked;
    ctx->base_registered = base_registered;

    ggml_backend_buffer_t buffer = ggml_backend_buffer_init(buft, vitriol_buffer_interface, ctx, size);
    if (!buffer) {
        if (ctx->base_registered) cudaHostUnregister(ctx->base);
        if (ctx->base_mlocked) munlock(ctx->base, ctx->size);
        munmap(ctx->base, ctx->size);
        if (ctx->store) vitriol::store_close(ctx->store);
        delete ctx;
        return nullptr;
    }
    ctx->buffer = buffer;
    if (store && !vitriol_store_register_buffer(buffer, store, ptr, size)) {
        ggml_backend_buffer_free(buffer);
        return nullptr;
    }

    {
        static std::once_flag flag;
        std::call_once(flag, [] { atexit(vitriol_buffer_atexit_cleanup); });
    }
    g_pending_cleanup = ctx;

    return buffer;
}

static size_t vitriol_buffer_type_get_alignment(ggml_backend_buffer_type_t buft) {
    return 32;
}

static size_t vitriol_buffer_type_get_alloc_size(ggml_backend_buffer_type_t buft, const struct ggml_tensor * tensor) {
    size_t size = ggml_nbytes(tensor);
    return (size + 31) & ~(size_t)31;
}

static const struct ggml_backend_buffer_type_i vitriol_buffer_type_interface = {
    /* .get_name         = */ vitriol_buffer_type_get_name,
    /* .alloc_buffer     = */ vitriol_buffer_type_alloc_buffer,
    /* .get_alignment    = */ vitriol_buffer_type_get_alignment,
    /* .get_max_size     = */ nullptr,
    /* .get_alloc_size   = */ vitriol_buffer_type_get_alloc_size,
    /* .is_host          = */ vitriol_buffer_type_is_host,
};

/* ── Lazy buffer type (pageable, no mlock, no cudaHostRegister) ── */

static ggml_backend_buffer_t vitriol_buffer_type_alloc_buffer_lazy(ggml_backend_buffer_type_t buft, size_t size) {
    vitriol::store * store = nullptr;
    if (g_vitriol_config.store_backed) {
        const char * spath = vitriol_store_path();
        if (!spath) {
            fprintf(stderr, "VITRIOL: store_backed but no store path\n");
            return nullptr;
        }
        if (vitriol_rust_enabled()) {
            /* Rust mode: the crate owns the store fd. The C++ ctx keeps a
             * null store so no C++ store code path touches it. */
            if (vs_open_store(spath) != 0) {
                fprintf(stderr, "VITRIOL(rust): store_open(%s) failed\n", spath);
                return nullptr;
            }
        } else {
            auto opened = vitriol::store_open(spath, O_RDONLY);
            if (!opened) {
                fprintf(stderr, "VITRIOL: store_open(%s) failed: %d\n", spath, (int)opened.error());
                return nullptr;
            }
            store = *opened;
        }
    }

    void * ptr = mmap(NULL, size, PROT_READ | PROT_WRITE,
                       MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (ptr == MAP_FAILED) {
        fprintf(stderr, "VITRIOL: lazy mmap(%zu) failed: %m\n", size);
        if (store) vitriol::store_close(store);
        return nullptr;
    }
    if (g_vitriol_config.verbose) {
        printf("VITRIOL: lazy buffer - pageable mmap (%zu MiB, not locked%s)\n",
               size / 1024 / 1024, store ? ", store-backed" : "");
    }
    auto * ctx = new vitriol_buffer_context{ptr, size};
    ctx->store = store;
    ggml_backend_buffer_t buffer = ggml_backend_buffer_init(buft, vitriol_buffer_interface, ctx, size);
    if (!buffer) {
        munmap(ptr, size);
        if (store) vitriol::store_close(store);
        delete ctx;
        return nullptr;
    }
    ctx->buffer = buffer;
    if (store && !vitriol_store_register_buffer(buffer, store, ptr, size)) {
        ggml_backend_buffer_free(buffer);
        return nullptr;
    }
    {
        static std::once_flag flag;
        std::call_once(flag, [] { atexit(vitriol_buffer_atexit_cleanup); });
    }
    g_pending_cleanup = ctx;
    return buffer;
}

static const struct ggml_backend_buffer_type_i vitriol_buffer_type_lazy_interface = {
    /* .get_name         = */ vitriol_buffer_type_get_name,
    /* .alloc_buffer     = */ vitriol_buffer_type_alloc_buffer_lazy,
    /* .get_alignment    = */ vitriol_buffer_type_get_alignment,
    /* .get_max_size     = */ nullptr,
    /* .get_alloc_size   = */ vitriol_buffer_type_get_alloc_size,
    /* .is_host          = */ vitriol_buffer_type_is_host,
};

/* ── Singleton access ─────────────────────────────────────────── */

ggml_backend_buffer_type_t vitriol_get_buffer_type(int device) {
    static std::mutex mtx;
    std::lock_guard<std::mutex> lock(mtx);

    static ggml_backend_buffer_type types[GGML_CUDA_MAX_DEVICES];
    static bool initialized = false;

    if (!initialized) {
        /* Steal the get_name function pointer from CUDA host buffer type.
         * This makes ggml_backend_buft_is_cuda_host() return true for
         * VITRIOL buft, telling the scheduler these weights are
         * CUDA-host-compatible (page-locked RAM, GPU-accessible via DMA).
         * Without this, the scheduler creates ~17 graph splits instead of
         * ~2, adding ~0.5 ms overhead per split per layer. */
        auto cuda_host_buft = ggml_backend_cuda_host_buffer_type();
        auto cuda_host_get_name = cuda_host_buft->iface.get_name;

        for (int i = 0; i < ggml_backend_cuda_get_device_count(); i++) {
            /* Start with the VITRIOL interface, but override get_name
             * to match CUDA host's — this is the only function pointer
             * the scheduler uses for buft identity checks. */
            ggml_backend_buffer_type_i iface = vitriol_buffer_type_interface;
            iface.get_name = cuda_host_get_name;

            types[i] = {
                /* .iface   = */ iface,
                /* .device  = */ ggml_backend_reg_dev_get(ggml_backend_cuda_reg(), i),
                /* .context = */ new vitriol_buffer_type_context{i, "VITRIOL"},
            };
        }
        initialized = true;
    }

    if (device >= ggml_backend_cuda_get_device_count())
        return nullptr;
    return &types[device];
}

ggml_backend_buffer_type_t vitriol_get_buffer_type_lazy(int device) {
    static std::mutex mtx;
    std::lock_guard<std::mutex> lock(mtx);

    static ggml_backend_buffer_type types[GGML_CUDA_MAX_DEVICES];
    static bool initialized = false;

    if (!initialized) {
        auto cuda_host_buft = ggml_backend_cuda_host_buffer_type();
        auto cuda_host_get_name = cuda_host_buft->iface.get_name;

        for (int i = 0; i < ggml_backend_cuda_get_device_count(); i++) {
            ggml_backend_buffer_type_i iface = vitriol_buffer_type_lazy_interface;
            iface.get_name = cuda_host_get_name;

            types[i] = {
                /* .iface   = */ iface,
                /* .device  = */ ggml_backend_reg_dev_get(ggml_backend_cuda_reg(), i),
                /* .context = */ new vitriol_buffer_type_context{i, "VITRIOL_LAZY"},
            };
        }
        initialized = true;
    }

    if (device >= ggml_backend_cuda_get_device_count())
        return nullptr;
    return &types[device];
}

/* ── Extra buffer types (model loader discovery) ───────────────────
 * Null-terminated per-device array returned via
 * ggml_backend_dev_get_extra_bufts so make_cpu_buft_list discovers the
 * VITRIOL buft and routes expert (MUL_MAT_ID) weights into host-pinned
 * buffers instead of normal CUDA VRAM. Mirrors the SYCL fix (5fb6a67a9).
 * ──────────────────────────────────────────────────────────────────*/
ggml_backend_buffer_type_t * vitriol_cuda_get_extra_bufts(ggml_backend_dev_t dev) {
    if (g_vitriol_config.mode != VITRIOL_MODE_STREAM) return nullptr;

    /* Stage 4.5: honor the queried device. Returning the full array to every
     * device made the loader bind all expert weights to one device's buft,
     * collapsing every expert byte onto a single GPU (and a single LRU
     * pool). Each device now sees only its own buft. */
    static ggml_backend_buffer_type_t extra_all[GGML_CUDA_MAX_DEVICES + 1];
    static ggml_backend_buffer_type_t extra_per_dev[GGML_CUDA_MAX_DEVICES][2];
    static bool extra_initialized = false;
    if (!extra_initialized) {
        const int n_devs = ggml_backend_cuda_get_device_count();
        for (int i = 0; i < n_devs && i < GGML_CUDA_MAX_DEVICES; i++) {
            extra_all[i] = g_vitriol_config.max_locked_mb > 0
                ? vitriol_get_buffer_type_lazy(i)
                : vitriol_get_buffer_type(i);
        }
        extra_all[n_devs] = nullptr;
        extra_initialized = true;
    }
    for (int i = 0; i < GGML_CUDA_MAX_DEVICES && extra_all[i]; i++) {
        if (ggml_backend_buft_get_device(extra_all[i]) == dev) {
            extra_per_dev[i][0] = extra_all[i];
            extra_per_dev[i][1] = nullptr;
            return extra_per_dev[i];
        }
    }
    return nullptr;
}

/* ── Zero-copy mmap wrapping (loader proc-address API) ─────────── */
/* llama-model.cpp queries these so expert (MUL_MAT_ID) tensors placed in
 * the vitriol buft can stay in file-backed GGUF pages under mmap=on: the
 * buffer wraps the mapping instead of allocating a copy, the compute gate
 * (vitriol_is_vitriol_buffer_type) stays true, and the streaming hooks DMA
 * from the mapping into the device LRU. Unregistered procs here were the
 * silent disarm: the wrap path never engaged and the hooks never saw a
 * vitriol buft. */

bool vitriol_buft_supports_host_ptr(ggml_backend_buffer_type_t buft) {
    return g_vitriol_config.mode == VITRIOL_MODE_STREAM && vitriol_is_vitriol_buffer_type(buft);
}

ggml_backend_buffer_t vitriol_buffer_from_host_ptr(ggml_backend_dev_t dev, void * ptr, size_t size, size_t max_tensor_size) {
    (void) max_tensor_size;
    if (g_vitriol_config.mode != VITRIOL_MODE_STREAM) return nullptr;
    if (!ptr || size == 0) return nullptr;

    /* resolve the same per-device buft the extra-bufts path handed out */
    ggml_backend_buffer_type_t * bufts = vitriol_cuda_get_extra_bufts(dev);
    if (!bufts || !*bufts) return nullptr;
    ggml_backend_buffer_type_t buft = *bufts;

    /* NO upfront cudaHostRegister: it would pin the whole mapping (OOM
     * beyond RAM). Page access stays demand-paged; the lazy pin tier
     * registers individual expert slices on demand and the LRU H2D copies
     * stage through the driver for unlocked (pageable) slices. */
    auto * ctx = new vitriol_buffer_context{ptr, size};
    ctx->owns_mapping = false;

    ggml_backend_buffer_t buffer = ggml_backend_buffer_init(buft, vitriol_buffer_interface, ctx, size);
    if (!buffer) {
        cudaHostUnregister(ptr);
        delete ctx;
        return nullptr;
    }
    ctx->buffer = buffer;
    {
        static std::once_flag flag;
        std::call_once(flag, [] { atexit(vitriol_buffer_atexit_cleanup); });
    }
    g_pending_cleanup = ctx;
    if (g_vitriol_config.verbose) {
        printf("VITRIOL: wrapped mmap region (%zu MiB) as %s buffer\n",
               size / 1024 / 1024, ggml_backend_buft_name(buft));
    }
    return buffer;
}
