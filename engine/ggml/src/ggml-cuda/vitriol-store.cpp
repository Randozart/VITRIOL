// VITRIOL Store - implementation. C++26 per docs/cpp26-conventions.md.
//
// Raw-device layout:
//   [0x0000] super 4K (2 copies at +0 and +4096)
//   [index]  dense store_tensor_desc array (n_tensors) + CRC trailer
//   [expert] per-tensor contiguous expert slices (tensor store_offset)
//   [embed]  embed_bytes sequential
//   [kv]     kv_ring_bytes (reserved, P3)
//
// I/O: pread/pwrite with O_DIRECT; io_uring async path with pread fallback.

#include "vitriol-store.h"

#include <fcntl.h>
#include <unistd.h>
#include <cerrno>
#include <cstring>
#include <cstdio>
#include <memory>
#include <vector>

#include <liburing.h>

namespace vitriol {

// Index zone sits right after the two superblock copies.
constexpr uint64_t STORE_INDEX_BASE = 2 * VITRIOL_STORE_BLK;

uint32_t store_crc32(std::span<const std::byte> data) {
    uint32_t crc = 0xFFFFFFFFu;
    for (auto b : data) {
        crc ^= static_cast<uint32_t>(b);
        for (int i = 0; i < 8; ++i) {
            crc = (crc >> 1) ^ (0xEDB88320u & -(crc & 1));
        }
    }
    return ~crc;
}

// 4096-aligned buffer (O_DIRECT requires 512B alignment on block devices).
// Also round the I/O length up to 512 so every pwrite/pread length is a
// 512-multiple (O_DIRECT rejects partial sectors).
static uint64_t align_up_512(uint64_t n) {
    return (n + 511) & ~(uint64_t)511;
}

template <typename T>
struct aligned_deleter {
    void operator()(T * p) const { std::free(p); }
};

template <typename T>
using aligned_ptr = std::unique_ptr<T, aligned_deleter<T>>;

template <typename T>
aligned_ptr<T> aligned_new(size_t count) {
    void * p = std::aligned_alloc(4096, count * sizeof(T));
    if (!p) return nullptr;
    std::memset(p, 0, count * sizeof(T));
    return aligned_ptr<T>(static_cast<T *>(p));
}

struct alignas(512) store_scratch_block {
    std::byte data[512];
};

struct store {
    int fd = -1;
    uint32_t flags = 0;
    store_super super{};
    aligned_ptr<store_tensor_desc> index;
    uint32_t n_tensors = 0;

    // io_uring state
    bool uring_ok = false;
    struct io_uring ring;
    uint32_t async_tid = 0;
    uint32_t async_expert = 0;
    uint32_t async_data_bytes = 0;
    uint32_t async_slice_bytes = 0;

    ~store() {
        if (uring_ok) {
            io_uring_queue_exit(&ring);
        }
        if (fd >= 0) {
            close(fd);
        }
    }
};

static bool super_valid(const store_super & s) {
    if (s.magic != VITRIOL_STORE_MAGIC || s.version != VITRIOL_STORE_VERSION) {
        return false;
    }
    std::span<const std::byte> bytes(
        reinterpret_cast<const std::byte *>(&s), VITRIOL_STORE_BLK - 4);
    return store_crc32(bytes) == s.crc32;
}

static store_error write_super_at(int fd, uint64_t off, const store_super & s) {
    const std::byte * p = reinterpret_cast<const std::byte *>(&s);
    if (pwrite(fd, p, VITRIOL_STORE_BLK, static_cast<off_t>(off)) !=
        static_cast<ssize_t>(VITRIOL_STORE_BLK)) {
        return store_error::io_error;
    }
    return store_error::none;
}

static store_error write_super(int fd, store_super & s) {
    // CRC over bytes 0..4091; crc32 is the last field at 4092 (excluded).
    s.crc32 = 0;
    std::span<const std::byte> bytes(
        reinterpret_cast<const std::byte *>(&s), VITRIOL_STORE_BLK - 4);
    s.crc32 = store_crc32(bytes);
    if (auto e = write_super_at(fd, 0, s); e != store_error::none) return e;
    return write_super_at(fd, VITRIOL_STORE_BLK, s);  // mirror copy
}

std::expected<store *, store_error> store_open(const char * path, int flags)
    pre(path != nullptr)
    pre(path[0] != '\0') {
    int fd = open(path, flags | O_DIRECT);
    if (fd < 0) {
        return std::unexpected(store_error::open_failed);
    }

    auto st = std::make_unique<store>();
    st->fd = fd;
    st->flags = flags;

    // super blocks: read into 4096-aligned buffers (O_DIRECT requires it).
    auto sb0 = aligned_new<store_super>(1);
    auto sb1 = aligned_new<store_super>(1);
    if (!sb0 || !sb1) {
        return std::unexpected(store_error::io_error);
    }
    const store_super & s0 = *sb0;
    const store_super & s1 = *sb1;
    if (pread(fd, sb0.get(), VITRIOL_STORE_BLK, 0) != VITRIOL_STORE_BLK ||
        pread(fd, sb1.get(), VITRIOL_STORE_BLK, VITRIOL_STORE_BLK) != VITRIOL_STORE_BLK) {
        return std::unexpected(store_error::io_error);
    }
    if (!super_valid(s0) && !super_valid(s1)) {
        return std::unexpected(store_error::not_a_store);
    }
    st->super = super_valid(s0) ? s0 : s1;

    // Load index zone: dense store_tensor_desc array + CRC trailer.
    const uint32_t n_tensors = st->super.expert_count;  // field reused as tensor count
    st->n_tensors = n_tensors;
    if (n_tensors > 0) {
        auto idx = aligned_new<store_tensor_desc>(n_tensors);
        if (!idx) {
            return std::unexpected(store_error::io_error);
        }
        const uint64_t index_bytes =
            static_cast<uint64_t>(n_tensors) * sizeof(store_tensor_desc);
        const uint64_t zone_bytes = align_up_512(index_bytes + sizeof(uint64_t));
        auto zone = aligned_new<std::byte>(zone_bytes);
        if (!zone) {
            return std::unexpected(store_error::io_error);
        }
        if (pread(fd, zone.get(), zone_bytes,
                  static_cast<off_t>(STORE_INDEX_BASE)) !=
                static_cast<ssize_t>(zone_bytes)) {
            return std::unexpected(store_error::io_error);
        }
        const std::byte * ip = zone.get();
        std::span<const std::byte> idx_bytes(ip, index_bytes);
        uint64_t stored_crc = 0;
        std::memcpy(&stored_crc, ip + index_bytes, sizeof(stored_crc));
        if (store_crc32(idx_bytes) != stored_crc) {
            return std::unexpected(store_error::index_corrupt);
        }
        std::memcpy(idx.get(), ip, index_bytes);
        st->index = std::move(idx);
    }

    // Set up io_uring (best-effort; fall back to pread).
    if (io_uring_queue_init(8, &st->ring, 0) == 0) {
        st->uring_ok = true;
    }

    return st.release();
}

void store_close(store * st) {
    delete st;
}

uint32_t store_expert_count(const store * st) {
    return st->super.expert_count;
}

uint32_t store_slot_size(const store * st) {
    return static_cast<uint32_t>(st->super.expert_stride);
}

uint64_t store_embed_bytes(const store * st) {
    return st->super.embed_bytes;
}

uint32_t store_tensor_count(const store * st) {
    return st->n_tensors;
}

const store_tensor_desc & store_tensor_desc_at(const store * st, uint32_t tid) {
    return st->index.get()[tid];
}

uint64_t store_expert_offset(const store * st, uint32_t tid, uint32_t expert_idx) {
    // Padded stride (512-multiple) keeps every offset O_DIRECT-aligned.
    return st->index.get()[tid].store_offset +
        static_cast<uint64_t>(expert_idx) * st->index.get()[tid].slice_bytes;
}

ssize_t store_read_expert(store * st, uint32_t tid, uint32_t expert_idx, void * dst, size_t dst_size) {
    if (!st || !dst || tid >= st->n_tensors) return -1;
    const store_tensor_desc & t = st->index.get()[tid];
    if (expert_idx >= t.n_experts || t.data_bytes == 0 || t.slice_bytes < t.data_bytes ||
        dst_size != t.data_bytes) {
        return -1;
    }

    const uint64_t off = store_expert_offset(st, tid, expert_idx);
    ssize_t r;
    if (std::is_sufficiently_aligned<512>(dst) && dst_size >= t.slice_bytes) {
        r = pread(st->fd, dst, t.slice_bytes, static_cast<off_t>(off));
    } else {
        thread_local std::vector<store_scratch_block> scratch;
        const size_t blocks = (static_cast<size_t>(t.slice_bytes) + 511) / 512;
        scratch.resize(blocks);
        r = pread(st->fd, scratch.data(), t.slice_bytes, static_cast<off_t>(off));
        if (r == static_cast<ssize_t>(t.slice_bytes)) {
            std::memcpy(dst, scratch.data(), t.data_bytes);
        }
    }
    return r == static_cast<ssize_t>(t.slice_bytes) ? static_cast<ssize_t>(t.data_bytes) : -1;
}

ssize_t store_read_embed(store * st, uint64_t offset, std::span<std::byte> dst) {
    if (st->super.embed_base == 0 || dst.empty()) {
        return -1;
    }
    return pread(st->fd, dst.data(), dst.size(),
                 static_cast<off_t>(st->super.embed_base + offset));
}

int store_read_expert_async(store * st, uint32_t tid, uint32_t expert_idx, void * dst, size_t dst_size) {
    if (!st || !dst || tid >= st->n_tensors) return -1;
    const store_tensor_desc & t = st->index.get()[tid];
    if (expert_idx >= t.n_experts || dst_size < t.data_bytes) return -1;

    auto fallback = [&]() {
        std::vector<std::byte> temp(t.data_bytes);
        const ssize_t result = store_read_expert(st, tid, expert_idx, temp.data(), temp.size());
        if (result < 0) return -1;
        std::memcpy(dst, temp.data(), t.data_bytes);
        st->async_data_bytes = t.data_bytes;
        st->async_slice_bytes = 0;
        return 0;
    };

    if (!st->uring_ok || !std::is_sufficiently_aligned<512>(dst) || dst_size < t.slice_bytes) {
        return fallback();
    }
    struct io_uring_sqe * sqe = io_uring_get_sqe(&st->ring);
    if (!sqe) {
        return fallback();
    }
    st->async_tid = tid;
    st->async_expert = expert_idx;
    st->async_data_bytes = t.data_bytes;
    st->async_slice_bytes = t.slice_bytes;
    const uint64_t off = store_expert_offset(st, tid, expert_idx);
    io_uring_prep_read(sqe, st->fd, dst, t.slice_bytes, off);
    return io_uring_submit(&st->ring) == 0 ? 0 : fallback();
}

ssize_t store_read_expert_wait(store * st) {
    if (!st) {
        return -1;
    }
    if (st->async_slice_bytes == 0) {
        return static_cast<ssize_t>(st->async_data_bytes);
    }
    if (!st->uring_ok) {
        return -1;
    }
    struct io_uring_cqe * cqe = nullptr;
    if (io_uring_wait_cqe(&st->ring, &cqe) != 0) {
        return -1;
    }
    const int res = cqe->res;
    io_uring_cqe_seen(&st->ring, cqe);
    return res == static_cast<int>(st->async_slice_bytes)
        ? static_cast<ssize_t>(st->async_data_bytes)
        : -1;
}

std::expected<void, store_error> store_mkfs(
    const char * path,
    std::span<const store_tensor_desc> tensors,
    uint64_t embed_bytes,
    uint64_t kv_bytes)
    pre(path != nullptr) {
    int fd = open(path, O_RDWR | O_DIRECT | O_CREAT, 0600);
    if (fd < 0) {
        return std::unexpected(store_error::open_failed);
    }

    store_super s{};
    s.magic = VITRIOL_STORE_MAGIC;
    s.version = VITRIOL_STORE_VERSION;
    s.expert_count = static_cast<uint32_t>(tensors.size());
    s.expert_stride = VITRIOL_STORE_SLOT;  // alignment unit; real stride per-tensor
    s.expert_base = STORE_INDEX_BASE +
        static_cast<uint64_t>(tensors.size()) * sizeof(store_tensor_desc) +
        sizeof(uint64_t);  // + index CRC trailer
    s.expert_base = (s.expert_base + (1024*1024 - 1)) & ~(uint64_t)(1024*1024 - 1);

    // Compute expert zone total = sum of tensor bytes (exact, no padding).
    uint64_t expert_bytes = 0;
    for (const auto & t : tensors) {
        expert_bytes += static_cast<uint64_t>(t.n_experts) * t.slice_bytes;
    }
    s.embed_base = s.expert_base + expert_bytes;
    s.embed_bytes = embed_bytes;
    if (kv_bytes > 0) {
        s.kv_base = s.embed_base + embed_bytes;
        s.kv_ring_bytes = kv_bytes;
        s.kv_cell_size = VITRIOL_STORE_KVCELL;
    }

    if (auto e = write_super(fd, s); e != store_error::none) {
        close(fd);
        return std::unexpected(e);
    }

    // Preallocate: extend file to full length so later pwrite/read is O(1).
    // Skip on block devices (raw partitions) - ftruncate is invalid there
    // and the device already has fixed size.
    struct stat sb;
    const bool is_block = (fstat(fd, &sb) == 0) && S_ISBLK(sb.st_mode);
    const uint64_t total = s.kv_ring_bytes > 0
        ? (s.kv_base + s.kv_ring_bytes)
        : (s.embed_base + s.embed_bytes);
    if (!is_block && ftruncate(fd, static_cast<off_t>(total)) != 0) {
        close(fd);
        return std::unexpected(store_error::io_error);
    }

    // Write index zone: tensors with store_offset assigned sequentially.
    auto idx = aligned_new<store_tensor_desc>(tensors.size());
    if (!idx) {
        close(fd);
        return std::unexpected(store_error::io_error);
    }
    uint64_t off = s.expert_base;
    for (size_t i = 0; i < tensors.size(); ++i) {
        idx.get()[i] = tensors[i];
        idx.get()[i].store_offset = off;
        off += static_cast<uint64_t>(tensors[i].n_experts) * tensors[i].slice_bytes;
    }
    const uint64_t index_bytes =
        static_cast<uint64_t>(tensors.size()) * sizeof(store_tensor_desc);
    const uint64_t zone_bytes = align_up_512(index_bytes + sizeof(uint64_t));
    // one aligned block: [index data][crc64]; length rounded to 512.
    auto zone = aligned_new<std::byte>(zone_bytes);
    if (!zone) {
        close(fd);
        return std::unexpected(store_error::io_error);
    }
    std::byte * zp = zone.get();
    std::memcpy(zp, idx.get(), index_bytes);
    const uint64_t idx_crc = store_crc32(
        std::span<const std::byte>(
            reinterpret_cast<const std::byte *>(idx.get()), index_bytes));
    std::memcpy(zp + index_bytes, &idx_crc, sizeof(idx_crc));
    if (pwrite(fd, zp, zone_bytes, static_cast<off_t>(STORE_INDEX_BASE)) !=
            static_cast<ssize_t>(zone_bytes)) {
        close(fd);
        return std::unexpected(store_error::io_error);
    }

    close(fd);
    return {};
}

ssize_t store_write_expert_tensor(store * st, uint32_t tid, const void * src) {
    const store_tensor_desc & t = st->index.get()[tid];
    const uint8_t * s = static_cast<const uint8_t *>(src);
    // Source holds n_experts * data_bytes (contiguous, real). Store layout is
    // n_experts padded slots (slice_bytes each). Copy real data + zero pad.
    auto slot = aligned_new<std::byte>(t.slice_bytes);
    if (!slot) {
        return -1;
    }
    for (uint32_t e = 0; e < t.n_experts; ++e) {
        std::memset(slot.get(), 0, t.slice_bytes);
        std::memcpy(slot.get(), s + (size_t)e * t.data_bytes, t.data_bytes);
        const uint64_t off = t.store_offset +
            static_cast<uint64_t>(e) * t.slice_bytes;
        if (pwrite(st->fd, slot.get(), t.slice_bytes, static_cast<off_t>(off)) !=
            static_cast<ssize_t>(t.slice_bytes)) {
            return -1;
        }
    }
    return static_cast<ssize_t>((uint64_t)t.n_experts * t.data_bytes);
}

ssize_t store_write_embed(store * st, uint64_t offset, std::span<const std::byte> src) {
    if (st->super.embed_base == 0 || src.empty()) {
        return -1;
    }
    return pwrite(st->fd, src.data(), src.size(),
                  static_cast<off_t>(st->super.embed_base + offset));
}

}  // namespace vitriol