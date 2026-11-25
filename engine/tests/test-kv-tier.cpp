// PROVENANCE: kintsugi-engine 8f12baacf (MIT), tests/test-kv-tier.cpp -- taken over unchanged except
// for this note and the comments below. In that engine the host tail is reached through a staging alias;
// here it is read in place over PCIe (no staging, no set_rows redirect -- see ggml-cuda.cu
// ggml_backend_cuda_tier_buffer_type), which makes this test the stronger of the two claims: the bytes
// and the attention output must still match an ordinary device buffer, now across a direct host read.
//
// Tiered K/V buffer (CUDA VMM, llama_context_params.n_kv_vram_cells): writes and reads past the VRAM line must give
// the bytes of an ordinary device buffer.
//
// One "layer": K then V in one buffer (2 parts), q4_0, 1,024 values for each cell (576 bytes), 10,240 cells. The K
// tensor is 5.625 MiB, not a whole number of 2 MiB pages, so the last page of the K host tail also holds the first
// cells of V, and rows cross page boundaries. Each step writes 256 rows of K and V with set_rows and runs one
// attention op over all cells, as a prompt batch does (no CUDA graph: redirected writes, write-back, prefetch). The
// steps put rows on each side of each run boundary. The same steps run on an ordinary device buffer; the K bytes, the
// V bytes and each attention output must be equal.
//
// History: the write-back of the redirected set_rows used only the first host run of a tensor. With K not page-aligned
// the first host run of V is the shared page, so the V rows of the real host tail were never written to host memory.
//
// No model file. Without a GPU device or without tiered buffers (no VMM) the test prints SKIP and passes.

#include "ggml.h"
#include "ggml-alloc.h"
#include "ggml-backend.h"

#include <cinttypes>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>

static const int     D       = 256;     // head size
static const int     H       = 4;       // K/V heads (and Q heads)
static const int64_t CELLS   = 10240;
static const int     ROWS    = 256;     // rows written for each step
static const int     NQ      = 64;      // queries for each step
static const int64_t STEPS[] = { 0, 512, 3520, 4224, 6000, 9984 };   // first cell of each step

struct result {
    std::vector<uint8_t>            k, v;
    std::vector<std::vector<float>> out;
};

static uint32_t rng_state;
static float frand() {
    rng_state = rng_state*1664525u + 1013904223u;
    return ((rng_state >> 8) & 0xFFFF)/32768.0f - 1.0f;
}

static bool run(ggml_backend_t backend, ggml_backend_buffer_type_t buft, result & res) {
    ggml_init_params ip = { ggml_tensor_overhead()*8, nullptr, true };
    ggml_context * ctx_kv = ggml_init(ip);
    ggml_tensor * k = ggml_new_tensor_2d(ctx_kv, GGML_TYPE_Q4_0, D*H, CELLS);
    ggml_tensor * v = ggml_new_tensor_2d(ctx_kv, GGML_TYPE_Q4_0, D*H, CELLS);
    ggml_backend_buffer_t buf_kv = ggml_backend_alloc_ctx_tensors_from_buft(ctx_kv, buft);
    if (!buf_kv) {
        ggml_free(ctx_kv);
        return false;
    }
    ggml_backend_buffer_clear(buf_kv, 0);

    ggml_gallocr_t galloc = ggml_gallocr_new(ggml_backend_get_default_buffer_type(backend));
    rng_state = 12345;
    bool ok = true;
    for (int64_t first : STEPS) {
        ggml_init_params gp = { ggml_tensor_overhead()*64 + ggml_graph_overhead(), nullptr, true };
        ggml_context * ctx = ggml_init(gp);
        ggml_tensor * k_rows = ggml_new_tensor_2d(ctx, GGML_TYPE_F32, D*H, ROWS);
        ggml_tensor * v_rows = ggml_new_tensor_2d(ctx, GGML_TYPE_F32, D*H, ROWS);
        ggml_tensor * idx    = ggml_new_tensor_1d(ctx, GGML_TYPE_I64, ROWS);
        ggml_tensor * q      = ggml_new_tensor_3d(ctx, GGML_TYPE_F32, D, NQ, H);
        ggml_tensor * mask   = ggml_new_tensor_2d(ctx, GGML_TYPE_F16, CELLS, NQ);
        for (ggml_tensor * t : { k_rows, v_rows, idx, q, mask }) {
            ggml_set_input(t);
        }

        ggml_cgraph * gf = ggml_new_graph(ctx);
        ggml_build_forward_expand(gf, ggml_set_rows(ctx, k, k_rows, idx));
        ggml_build_forward_expand(gf, ggml_set_rows(ctx, v, v_rows, idx));
        const size_t r1 = ggml_row_size(GGML_TYPE_Q4_0, D), r2 = ggml_row_size(GGML_TYPE_Q4_0, D*H);
        ggml_tensor * kk = ggml_permute(ctx, ggml_view_4d(ctx, k, D, H, CELLS, 1, r1, r2, r2*CELLS, 0), 0, 2, 1, 3);
        ggml_tensor * vv = ggml_permute(ctx, ggml_view_4d(ctx, v, D, H, CELLS, 1, r1, r2, r2*CELLS, 0), 0, 2, 1, 3);
        ggml_tensor * out = ggml_flash_attn_ext(ctx, q, kk, vv, mask, 1.0f/16.0f, 0.0f, 0.0f);
        ggml_set_output(out);
        ggml_build_forward_expand(gf, out);
        if (!ggml_gallocr_alloc_graph(galloc, gf)) {
            printf("graph allocation failed\n");
            ok = false;
        }

        if (ok) {
            std::vector<float> rows((size_t) D*H*ROWS), qd((size_t) D*NQ*H);
            std::vector<int64_t> ids(ROWS);
            std::vector<ggml_fp16_t> zeros((size_t) CELLS*NQ, ggml_fp32_to_fp16(0.0f));
            for (int i = 0; i < ROWS; ++i) {
                ids[i] = first + i;
            }
            for (float & x : rows) { x = frand(); }
            ggml_backend_tensor_set(k_rows, rows.data(), 0, ggml_nbytes(k_rows));
            for (float & x : rows) { x = frand(); }
            ggml_backend_tensor_set(v_rows, rows.data(), 0, ggml_nbytes(v_rows));
            for (float & x : qd) { x = frand(); }
            ggml_backend_tensor_set(q, qd.data(), 0, ggml_nbytes(q));
            ggml_backend_tensor_set(idx, ids.data(), 0, ggml_nbytes(idx));
            ggml_backend_tensor_set(mask, zeros.data(), 0, ggml_nbytes(mask));

            if (ggml_backend_graph_compute(backend, gf) != GGML_STATUS_SUCCESS) {
                printf("graph compute failed\n");
                ok = false;
            } else {
                std::vector<float> o((size_t) ggml_nelements(out));
                ggml_backend_tensor_get(out, o.data(), 0, ggml_nbytes(out));
                res.out.push_back(o);
            }
        }
        ggml_free(ctx);
        if (!ok) {
            break;
        }
    }
    if (ok) {
        res.k.resize(ggml_nbytes(k));
        res.v.resize(ggml_nbytes(v));
        ggml_backend_tensor_get(k, res.k.data(), 0, res.k.size());
        ggml_backend_tensor_get(v, res.v.data(), 0, res.v.size());
    }
    ggml_gallocr_free(galloc);
    ggml_backend_buffer_free(buf_kv);
    ggml_free(ctx_kv);
    return ok;
}

// number of rows that differ and the first one
static int64_t diff_rows(const std::vector<uint8_t> & a, const std::vector<uint8_t> & b, int64_t * first) {
    const size_t row = a.size()/CELLS;
    int64_t n = 0;
    *first = -1;
    for (int64_t c = 0; c < CELLS; ++c) {
        if (memcmp(a.data() + c*row, b.data() + c*row, row) != 0) {
            if (n++ == 0) {
                *first = c;
            }
        }
    }
    return n;
}

int main() {
    ggml_backend_dev_t dev = nullptr;
    for (size_t i = 0; i < ggml_backend_dev_count(); ++i) {
        if (ggml_backend_dev_type(ggml_backend_dev_get(i)) == GGML_BACKEND_DEVICE_TYPE_GPU) {
            dev = ggml_backend_dev_get(i);
            break;
        }
    }
    if (!dev) {
        printf("SKIP: no GPU device\n");
        return 0;
    }
    using tier_fn_t = ggml_backend_buffer_type_t (*)(int, double, int, const char *);
    ggml_backend_reg_t reg = ggml_backend_dev_backend_reg(dev);
    auto * fn = reg ? (tier_fn_t) ggml_backend_reg_get_proc_address(reg, "ggml_backend_cuda_tier_buffer_type") : nullptr;
    if (!fn) {
        printf("SKIP: the backend of %s has no tiered buffers\n", ggml_backend_dev_name(dev));
        return 0;
    }
    ggml_backend_t backend = ggml_backend_dev_init(dev, nullptr);
    if (!backend) {
        printf("SKIP: cannot start %s\n", ggml_backend_dev_name(dev));
        return 0;
    }
    printf("backend %s, K and V %" PRId64 " cells x %zu bytes (K tensor %.3f MiB)\n", ggml_backend_name(backend), CELLS,
        ggml_row_size(GGML_TYPE_Q4_0, D*H), CELLS*ggml_row_size(GGML_TYPE_Q4_0, D*H)/1048576.0);

    result plain, tier;
    if (!run(backend, ggml_backend_dev_buffer_type(dev), plain)) {
        printf("FAIL: the run on the ordinary device buffer failed\n");
        return 1;
    }
    ggml_backend_buffer_type_t tb = fn(0, 0.25, 2, "kvtest_l0");
    if (!tb || !run(backend, tb, tier)) {
        if (!tb || tier.out.empty()) {
            printf("SKIP: no tiered buffer on this build or device (no VMM)\n");
            ggml_backend_free(backend);
            return 0;
        }
        printf("FAIL: the run on the tiered buffer failed\n");
        return 1;
    }

    int fails = 0;
    int64_t first = -1;
    int64_t n = diff_rows(plain.k, tier.k, &first);
    printf("K rows that differ: %" PRId64 " of %" PRId64 "%s\n", n, CELLS, n ? "" : " (ok)");
    if (n) { printf("  first different cell %" PRId64 "\n", first); fails++; }
    n = diff_rows(plain.v, tier.v, &first);
    printf("V rows that differ: %" PRId64 " of %" PRId64 "%s\n", n, CELLS, n ? "" : " (ok)");
    if (n) { printf("  first different cell %" PRId64 "\n", first); fails++; }
    for (size_t s = 0; s < plain.out.size() && s < tier.out.size(); ++s) {
        const bool same = memcmp(plain.out[s].data(), tier.out[s].data(), plain.out[s].size()*sizeof(float)) == 0;
        printf("step %zu (cells %" PRId64 " to %" PRId64 "): attention output %s\n", s, STEPS[s], STEPS[s] + ROWS - 1, same ? "equal" : "DIFFERENT");
        fails += !same;
    }
    ggml_backend_free(backend);
    printf(fails ? "FAIL\n" : "ALL OK\n");
    return fails ? 1 : 0;
}
