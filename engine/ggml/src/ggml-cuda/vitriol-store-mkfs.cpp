// vitriol-store-mkfs - convert a multi-shard GGUF into a VITRIOL store.
//
// C++26 per docs/cpp26-conventions.md. Reads each shard with ggml's gguf
// loader, classifies expert/embed tensors, builds the store via vitriol-store.
//
// Usage:
//   vitriol-store-mkfs <store-path> <gguf-shard-01-of-03.gguf> [kv-bytes]
//
// Experts: tensors whose name matches blk.*.ffn_*_exps / *_shexp. Sliced per
// expert (expert axis is the last dim). Slice bytes = tensor_size / n_experts.
// Embed: per_layer_token_embd.weight -> sequential embed zone.

#include "vitriol-store.h"
#include "gguf.h"
#include "ggml.h"

#include <fcntl.h>
#include <unistd.h>
#include <cerrno>
#include <cstdio>
#include <cstring>
#include <memory>
#include <string>
#include <vector>

using namespace vitriol;

// ---- GGUF split naming: "<prefix>-00002-of-00003.gguf" ----------------------

static std::vector<std::string> list_splits(const std::string & first) {
    std::vector<std::string> paths;
    // find "-NNNNN-of-NNNNN.gguf"
    auto ofpos = first.rfind("-of-");
    if (ofpos == std::string::npos) { paths.push_back(first); return paths; }
    auto dash = first.rfind('-', ofpos - 1);
    if (dash == std::string::npos) { paths.push_back(first); return paths; }
    const std::string prefix = first.substr(0, dash + 1);
    const std::string idxstr = first.substr(dash + 1, ofpos - dash - 1);
    std::string nstr = first.substr(ofpos + 4);
    // strip ".gguf" suffix
    auto dot = nstr.find('.');
    if (dot != std::string::npos) nstr = nstr.substr(0, dot);
    const int n_split = std::stoi(nstr);
    char buf[1024];
    for (int i = 0; i < n_split; ++i) {
        snprintf(buf, sizeof buf, "%s%0*d-of-%0*d.gguf",
                 prefix.c_str(), (int)idxstr.size(), i + 1,
                 (int)nstr.size(), n_split);
        paths.emplace_back(buf);
    }
    return paths;
}

// classify tensor kind from name: 0=ffn_up,1=ffn_gate,2=ffn_down,3=shexp; -1=not expert
static int expert_kind(const char * name) {
    if (strstr(name, "ffn_up_exps"))    return 0;
    if (strstr(name, "ffn_gate_exps"))  return 1;
    if (strstr(name, "ffn_down_exps"))  return 2;
    if (strstr(name, "shexp"))          return 3;
    return -1;
}

static int layer_of(const char * name) {
    // blk.N.ffn_... -> N
    const char * b = strstr(name, "blk.");
    if (!b) return 0;
    return atoi(b + 4);
}

int main(int argc, char ** argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: %s <store-path> <gguf-00001-of-00003.gguf> [kv-bytes]\n", argv[0]);
        fprintf(stderr, "       %s --scan <gguf-00001-of-00003.gguf>   (metadata only, no store write)\n", argv[0]);
        return 1;
    }
    const bool scan_only = std::string(argv[1]) == "--scan";
    const std::string store_path = scan_only ? "/dev/null" : argv[1];
    const std::string gguf_first = scan_only ? argv[2] : argv[2];
    const uint64_t kv_bytes = (!scan_only && argc >= 4) ? strtoull(argv[3], nullptr, 10) : 0;

    const auto shards = list_splits(gguf_first);
    printf("mkfs: %zu shard(s), store=%s\n", shards.size(), store_path.c_str());

    // Pass 1: load all shards, collect expert tensors + embed size.
    std::vector<store_tensor_desc> tensors;
    uint64_t embed_bytes = 0;
    struct shard_tensor { std::string name; size_t data_off; size_t size; };
    std::vector<std::vector<shard_tensor>> shard_tensors(shards.size());

    for (size_t si = 0; si < shards.size(); ++si) {
        // no_alloc=true: header/metadata only; data is stream-copied via pread.
        struct gguf_init_params params = { true, nullptr };
        struct gguf_context * ctx = gguf_init_from_file(shards[si].c_str(), params);
        if (!ctx) {
            fprintf(stderr, "mkfs: cannot open %s\n", shards[si].c_str());
            return 1;
        }
        const size_t data_base = gguf_get_data_offset(ctx);
        const int nt = gguf_get_n_tensors(ctx);
        printf("  shard %zu: %d tensors\n", si, nt);
        for (int ti = 0; ti < nt; ++ti) {
            const char * name = gguf_get_tensor_name(ctx, ti);
            const size_t size = gguf_get_tensor_size(ctx, ti);
            const size_t off = data_base + gguf_get_tensor_offset(ctx, ti);
            shard_tensors[si].push_back({ name, off, size });

            const int kind = expert_kind(name);
            if (kind >= 0) {
                const int64_t * ne = gguf_get_tensor_ne(ctx, ti);
                const int64_t n_experts = ne[2];  // expert axis = last (3D tensor)
                const int layer = layer_of(name);
                store_tensor_desc t{};
                t.layer = (uint32_t)layer;
                t.kind = (uint32_t)kind;
                t.n_experts = (uint32_t)n_experts;
                t.data_bytes = (uint32_t)(size / n_experts);
                t.slice_bytes = (uint32_t)((size / n_experts + 511) & ~(uint64_t)511);
                tensors.push_back(t);
            } else if (strstr(name, "per_layer_token_embd.weight")) {
                embed_bytes = size;
                printf("  embed: per_layer_token_embd %zu bytes\n", size);
            }
        }
        gguf_free(ctx);
    }

    printf("mkfs: %zu expert tensors, embed %llu bytes\n",
           tensors.size(), (unsigned long long)embed_bytes);

    if (scan_only) {
        uint64_t expert_bytes = 0;
        for (const auto & t : tensors) {
            expert_bytes += (uint64_t)t.n_experts * t.slice_bytes;
        }
        printf("SCAN ONLY - no store written. expert_bytes=%llu\n",
               (unsigned long long)expert_bytes);
        return 0;
    }

    // mkfs the store.
    auto m = store_mkfs(store_path.c_str(), tensors, embed_bytes, kv_bytes);
    if (!m) { fprintf(stderr, "mkfs FAIL: %d\n", (int)m.error()); return 1; }
    auto o = store_open(store_path.c_str(), O_RDWR);
    if (!o) { fprintf(stderr, "open FAIL: %d\n", (int)o.error()); return 1; }
    store * st = *o;

    // Pass 2: stream-copy each expert tensor's data into the store.
    size_t tidx = 0;
    for (size_t si = 0; si < shards.size(); ++si) {
        int fd = open(shards[si].c_str(), O_RDONLY);
        if (fd < 0) { fprintf(stderr, "mkfs: open shard %s: %s\n", shards[si].c_str(), strerror(errno)); return 1; }
        for (const auto & t : shard_tensors[si]) {
            const int kind = expert_kind(t.name.c_str());
            if (kind >= 0) {
                const store_tensor_desc & desc = tensors[tidx];
                // source holds n_experts * data_bytes (real, contiguous)
                const size_t real = (size_t)desc.n_experts * desc.data_bytes;
                std::vector<char> buf(real);
                ssize_t r = pread(fd, buf.data(), real, (off_t)t.data_off);
                if (r != (ssize_t)real) {
                    fprintf(stderr, "mkfs: read %s failed (%zd): %s\n", t.name.c_str(), r, strerror(errno));
                    return 1;
                }
                ssize_t w = store_write_expert_tensor(st, tidx, buf.data());
                if (w != (ssize_t)real) {
                    fprintf(stderr, "mkfs: write %s failed (%zd)\n", t.name.c_str(), w);
                    return 1;
                }
                if (tidx % 16 == 0 || tidx + 1 == tensors.size()) {
                    printf("  expert %zu/%zu: %s (%zu B real, %u B slot)\n",
                           tidx + 1, tensors.size(), t.name.c_str(), real, desc.slice_bytes);
                }
                ++tidx;
            } else if (strstr(t.name.c_str(), "per_layer_token_embd.weight")) {
                // stream embed in 16 MiB chunks, last chunk padded to 512.
                const size_t CHUNK = 16 * 1024 * 1024;
                std::vector<char> buf(CHUNK);
                uint64_t copied = 0;
                while (copied < embed_bytes) {
                    const size_t remaining = embed_bytes - copied;
                    const size_t chunk = std::min(CHUNK, remaining);
                    const size_t padded = (chunk + 511) & ~(size_t)511;
                    ssize_t r = pread(fd, buf.data(), chunk, (off_t)(t.data_off + copied));
                    if (r != (ssize_t)chunk) { fprintf(stderr, "mkfs: embed read failed\n"); return 1; }
                    if (padded > chunk) std::memset(buf.data() + chunk, 0, padded - chunk);
                    ssize_t w = store_write_embed(st, copied,
                        std::span<const std::byte>((const std::byte *)buf.data(), padded));
                    if (w != (ssize_t)padded) { fprintf(stderr, "mkfs: embed write failed (%zd)\n", w); return 1; }
                    copied += chunk;
                }
                printf("  embed copied %llu bytes\n", (unsigned long long)copied);
            }
        }
        close(fd);
    }

    store_close(st);
    printf("mkfs DONE: %zu expert tensors, %llu embed bytes\n",
           tensors.size(), (unsigned long long)embed_bytes);
    return 0;
}