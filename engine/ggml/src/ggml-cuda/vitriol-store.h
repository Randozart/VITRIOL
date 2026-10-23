// VITRIOL Store - raw Optane 900P zone for expert/embed/KV streaming
//
// C++26 per docs/cpp26-conventions.md. Contracts (pre/post) on public API;
// std::expected at boundaries; scalar returns on hot read path.
//
// No-filesystem fixed-slot layout on a raw partition. O(1) addressing:
// offset = zone_base + slot * stride. RAM-resident index rebuilt from a
// persisted index zone at open (1 superblock + 1 index read, then O(1)).

#pragma once

#include <cstddef>
#include <cstdint>
#include <expected>
#include <span>
#include <sys/types.h>

namespace vitriol {

constexpr uint32_t VITRIOL_STORE_MAGIC = 0x56495453;  // 'VITS'
constexpr uint32_t VITRIOL_STORE_VERSION = 1;
constexpr uint32_t VITRIOL_STORE_BLK = 4096;          // superblock size
constexpr uint64_t VITRIOL_STORE_SLOT = 2 * 1024 * 1024;  // 2 MiB expert slot
constexpr uint64_t VITRIOL_STORE_KVCELL = 2 * 1024 * 1024; // 2 MiB KV cell

constexpr uint32_t VITRIOL_STORE_Z_EXPERT = 1;
constexpr uint32_t VITRIOL_STORE_Z_EMBED  = 2;
constexpr uint32_t VITRIOL_STORE_Z_KV     = 3;

// On-disk superblock. Compile-time layout invariants enforced by static_assert
// below (always free, always on).
struct alignas(4096) store_super {
    uint32_t magic;
    uint32_t version;
    uint32_t expert_count;       // number of live expert slots
    uint32_t _pad0;
    uint64_t expert_base;        // byte offset of EXPERT zone
    uint64_t expert_stride;      // bytes between slots (== VITRIOL_STORE_SLOT)
    uint64_t embed_base;         // byte offset of EMBED zone (0 if absent)
    uint64_t embed_bytes;        // total embed tensor bytes
    uint64_t kv_base;            // byte offset of KV ring (0 if absent)
    uint64_t kv_ring_bytes;      // total KV ring size (0 if absent)
    uint64_t kv_cell_size;
    uint32_t _pad[1005];         // pad fields 72..4092 so crc32 lands at 4092
    uint32_t crc32;              // LAST field: CRC over bytes 0..4091
};

static_assert(sizeof(store_super) == 4096, "superblock must be 4 KiB");
static_assert(alignof(store_super) == 4096, "superblock must be 4KiB aligned");
static_assert(offsetof(store_super, crc32) == 4092, "crc32 must be the last field at 4092");

// Error type for boundary APIs.
enum class store_error {
    none = 0,
    open_failed,
    not_a_store,      // bad magic / version
    corrupt,          // CRC mismatch
    index_corrupt,
    io_error,
    out_of_bounds,    // slot >= expert_count
    not_implemented,  // KV ring not yet wired
};

struct store;
struct store_impl;

// Persistent per-expert-tensor descriptor (index zone). Dense array [0, n_tensors).
// Each expert occupies a PADDED slot (stride = align_up_512(slice_bytes)) so
// every store offset is 512-aligned (O_DIRECT requires it on block devices).
// data_bytes = real on-disk bytes; slice_bytes = padded slot size (512-mult).
struct store_tensor_desc {
    uint32_t layer;         // layer index
    uint32_t kind;          // 0=ffn_up, 1=ffn_gate, 2=ffn_down, 3=shexp
    uint32_t n_experts;     // 512 routed (or 1 shared expert)
    uint32_t slice_bytes;   // padded per-expert slot (align_up_512(data_bytes))
    uint32_t data_bytes;    // real per-expert on-disk bytes
    uint32_t _pad0;
    uint64_t store_offset;  // byte offset of this tensor in the EXPERT zone
};

// ---- boundary API (std::expected) ------------------------------------------

// Open store on raw device or plain file. O_RDONLY or O_RDWR.
// Reads superblock + index zone; errno-style error via store_error.
std::expected<store *, store_error> store_open(const char * path, int flags)
    pre(path != nullptr)
    pre(path[0] != '\0');

void store_close(store * st);

uint32_t store_expert_count(const store * st) pre(st != nullptr);
uint32_t store_slot_size(const store * st) pre(st != nullptr);
uint64_t store_embed_bytes(const store * st) pre(st != nullptr);
uint32_t store_tensor_count(const store * st) pre(st != nullptr);
const store_tensor_desc & store_tensor_desc_at(const store * st, uint32_t tid)
    pre(st != nullptr)
    pre(tid < store_tensor_count(st));
uint64_t store_expert_offset(const store * st, uint32_t tid, uint32_t expert_idx)
    pre(st != nullptr)
    pre(tid < store_tensor_count(st))
    pre(expert_idx < store_tensor_desc_at(st, tid).n_experts);

// ---- hot read path (scalar returns, no expected) ---------------------------

// Read one expert slice of tensor `tid`, expert index `expert_idx`, into dst.
// dst_size must equal data_bytes. Returns data_bytes on success, or -1. This is
// the hot MoE path - scalar.
ssize_t store_read_expert(store * st, uint32_t tid, uint32_t expert_idx, void * dst, size_t dst_size)
    pre(st != nullptr)
    pre(dst != nullptr)
    pre(dst_size > 0)
    pre(tid < store_tensor_count(st))
    pre(expert_idx < store_tensor_desc_at(st, tid).n_experts)
    pre(dst_size == store_tensor_desc_at(st, tid).data_bytes);

// Read a slice of the embed tensor. Sequential/random-row bulk path.
// Returns bytes read, or -1.
ssize_t store_read_embed(store * st, uint64_t offset, std::span<std::byte> dst)
    pre(st != nullptr)
    pre(offset + dst.size() <= store_embed_bytes(st));

// Asynchronous read of one expert slice via io_uring. The destination must be
// 512-aligned and large enough for the padded on-disk slot. Returns 0 queued,
// -1 otherwise.
int store_read_expert_async(store * st, uint32_t tid, uint32_t expert_idx, void * dst, size_t dst_size)
    pre(st != nullptr)
    pre(dst != nullptr)
    pre(dst_size > 0)
    pre(tid < store_tensor_count(st))
    pre(expert_idx < store_tensor_desc_at(st, tid).n_experts);

// Wait for the outstanding async op. Returns bytes read or -1.
ssize_t store_read_expert_wait(store * st) pre(st != nullptr);

// ---- mkfs ------------------------------------------------------------------

// Format a raw device/file. Writes super + tensor index; caller then streams
// expert/embed data with store_write_expert_tensor / store_write_embed.
std::expected<void, store_error> store_mkfs(
    const char * path,
    std::span<const store_tensor_desc> tensors,
    uint64_t embed_bytes,
    uint64_t kv_bytes)
    pre(path != nullptr);

// Write one expert tensor's full data (mkfs tool). src == n_experts*slice_bytes.
ssize_t store_write_expert_tensor(store * st, uint32_t tid, const void * src)
    pre(st != nullptr)
    pre(src != nullptr)
    pre(tid < store_tensor_count(st));

// Write the embed tensor bytes (mkfs tool).
ssize_t store_write_embed(store * st, uint64_t offset, std::span<const std::byte> src)
    pre(st != nullptr)
    pre(offset + src.size() <= store_embed_bytes(st));

// ---- diagnostics ------------------------------------------------------------

// CRC32 (IEEE) over a byte span. Used by superblock + index validation.
uint32_t store_crc32(std::span<const std::byte> data);

}  // namespace vitriol