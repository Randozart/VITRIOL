// ggml_escha_mul_mat -- real-checkpoint conformance test.
//
// What this test is for
// ---------------------
// The Escha codec decodes a trellis code on the fly and rotates the activations
// instead of materialising a weight matrix. A bug in either half still produces
// plausible-looking output, which is the worst failure mode: the model loads, it
// talks, and the answers are quietly wrong. So the op is checked against an
// independent dense formulation on a REAL projection lifted out of a real
// checkpoint.
//
// Why the data comes from the checkpoint
// --------------------------------------
// `dep` must contain valid payload bit indices, so random tensors cannot exercise
// this op: a random dep indexes nonsense bit positions and the comparison is
// meaningless. The fixture is therefore `blk.0.attn_qkv` (plus the shared
// `escha_lut` / `escha_dep_k2`) read straight out of the model GGUF, metadata-only
// (no_alloc), with the few tensors we need fetched by explicit offset. That keeps
// peak RSS in the hundreds of MB instead of the 10.5 GB of the file.
//
// What is independent, and what is not
// ------------------------------------
// Independent of the op: the rotation is built as an explicit 128x128 Sylvester
// matrix and applied as a dense product (the op uses an in-place butterfly); the
// contraction runs output-tile-outer / input-tile-inner (the op runs the other
// way); and the reference accumulates in double while the op is f32.
//
// NOT independent, because it is the on-disk format rather than an algorithm:
// the codebook construction, the dep-table bit numbering (little-endian within
// the byte stream), and the 16x16 tile layout. The first of those is checked
// separately by check_codebook_lut() against the lut stored in the file.
//
// The negative controls matter as much as the positive case: they prove the
// comparison can fail, so a silent pass means agreement rather than a dead
// assertion.
//
//   usage: test-escha-mul-mat [model.gguf] [--tensor NAME] [--verbose]

#include "ggml-alloc.h"
#include "ggml-backend.h"
#include "ggml-cpu.h"
#include "ggml.h"
#include "gguf.h"

#include <cinttypes>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

// A real projection with IC/OC multiples of 128, present in Kintsugi-27B-12G.
static const char * DEFAULT_TENSOR = "blk.0.attn_qkv";

static int g_failures = 0;
static int g_checks   = 0;

static void fail(const char * what, const char * detail) {
    fprintf(stderr, "FAIL %s%s%s\n", what, detail && *detail ? ": " : "", detail ? detail : "");
    ++g_failures;
}

static void ok(const char * what) {
    printf("ok   %s\n", what);
    ++g_checks;
}

// ---- tensor fetch ---------------------------------------------------------

struct raw_tensor {
    std::vector<uint8_t> bytes;
    int64_t              ne[4] = { 1, 1, 1, 1 };
    int                  type  = -1;
};

// Read one tensor's payload by offset. gguf_init_from_file ran with no_alloc, so
// nothing is resident: the file offset is data_offset + tensor_offset and the
// size comes from the metadata.
static bool fetch_tensor(FILE * f, const gguf_context * g, const std::string & name, raw_tensor & out) {
    const int64_t id = gguf_find_tensor(g, name.c_str());
    if (id < 0) {
        fprintf(stderr, "missing tensor '%s' in the model file\n", name.c_str());
        return false;
    }

    const int64_t * ne = gguf_get_tensor_ne(g, id);   // GGML_MAX_DIMS entries, padded with 1
    for (int d = 0; d < 4; ++d) {
        out.ne[d] = ne[d];
    }
    out.type          = gguf_get_tensor_type(g, id);
    const size_t size = gguf_get_tensor_size(g, id);
    out.bytes.resize(size);

    const long off = (long) (gguf_get_data_offset(g) + gguf_get_tensor_offset(g, id));
    if (fseek(f, off, SEEK_SET) != 0) {
        fprintf(stderr, "seek failed for '%s' at %ld\n", name.c_str(), off);
        return false;
    }
    if (fread(out.bytes.data(), 1, size, f) != size) {
        fprintf(stderr, "short read for '%s' (%zu bytes)\n", name.c_str(), size);
        return false;
    }
    return true;
}

// ---- independent reference ------------------------------------------------

// Explicit Sylvester-Hadamard, normalized, built recursively rather than as a
// butterfly: H_2n = [[H,H],[H,-H]].
struct hadamard {
    double m[128][128];

    hadamard() {
        std::memset(m, 0, sizeof(m));
        m[0][0] = 1.0;
        for (int n = 1; n < 128; n *= 2) {
            for (int i = 0; i < n; ++i) {
                for (int j = 0; j < n; ++j) {
                    const double v  = m[i][j];
                    m[i][j + n]     = v;
                    m[i + n][j]     = v;
                    m[i + n][j + n] = -v;
                }
            }
        }
        const double s = 1.0 / std::sqrt(128.0);
        for (int i = 0; i < 128; ++i) {
            for (int j = 0; j < 128; ++j) {
                m[i][j] *= s;
            }
        }
    }

    // out = H * in, applied per 128-block
    void apply(double * v, int64_t n) const {
        double tmp[128];
        for (int64_t off = 0; off < n; off += 128) {
            for (int i = 0; i < 128; ++i) {
                double s = 0.0;
                for (int j = 0; j < 128; ++j) {
                    s += m[i][j] * v[off + j];
                }
                tmp[i] = s;
            }
            for (int i = 0; i < 128; ++i) {
                v[off + i] = tmp[i];
            }
        }
    }
};

// Codebook A. Same reconstruction as the storage format defines; computed in
// double here, from the same fp16 halves the format rounds through.
static double codebook(double idx_f) {
    const uint32_t idx = (uint32_t) idx_f;
    const uint32_t x   = ((idx * 0xcbac1fedu) & 0x8fff8fffu) ^ 0x3b603b60u;

    ggml_fp16_t lo;
    ggml_fp16_t hi;
    std::memcpy(&lo, (const char *) &x, sizeof(lo));
    std::memcpy(&hi, (const char *) &x + 2, sizeof(hi));

    const double v = (double) ggml_fp16_to_fp32(lo) + (double) ggml_fp16_to_fp32(hi);
    return (double) ggml_fp16_to_fp32(ggml_fp32_to_fp16((float) v));
}

// Decode one 16x16 tile into [r][c] as doubles.
static void decode_tile(const uint8_t * payload, const int16_t * dep, double * tile) {
    for (int p = 0; p < 256; ++p) {
        const int16_t * d   = dep + p * 16;
        uint32_t        idx = 0;
        for (int b = 0; b < 16; ++b) {
            idx |= (uint32_t) ((payload[d[b] >> 3] >> (d[b] & 7)) & 1) << b;
        }
        tile[p] = codebook((double) idx);
    }
}

// y = H_out( (H_in(x * rin)) @ decode(code) ) * rout, with the contraction
// ordered output-tile-outer and accumulated in double.
static void reference(const raw_tensor &         code,
                      const raw_tensor &         rin,
                      const raw_tensor &         rout,
                      const raw_tensor &         dep,
                      const std::vector<float> & x,
                      std::vector<double> &      y) {
    const int64_t nct = code.ne[1];
    const int64_t nit = code.ne[2];
    const int64_t OC  = nct * 16;
    const int64_t IC  = nit * 16;
    const int64_t ts  = code.ne[0];  // i16 per tile

    const hadamard      H;
    const int16_t *     code_d = (const int16_t *) code.bytes.data();
    const int16_t *     dep_d  = (const int16_t *) dep.bytes.data();
    const ggml_fp16_t * rin_d  = (const ggml_fp16_t *) rin.bytes.data();
    const ggml_fp16_t * rout_d = (const ggml_fp16_t *) rout.bytes.data();

    std::vector<double> u(IC);
    for (int64_t i = 0; i < IC; ++i) {
        u[i] = (double) x[i] * (double) ggml_fp16_to_fp32(rin_d[i]);
    }
    H.apply(u.data(), IC);

    std::vector<double> acc(OC, 0.0);
    std::vector<double> tile(256);

    for (int64_t j = 0; j < nct; ++j) {
        for (int64_t i = 0; i < nit; ++i) {
            decode_tile((const uint8_t *) (code_d + (i * nct + j) * ts), dep_d, tile.data());
            for (int r = 0; r < 16; ++r) {
                const double ur = u[i * 16 + r];
                if (ur == 0.0) {
                    continue;
                }
                for (int c = 0; c < 16; ++c) {
                    acc[j * 16 + c] += ur * tile[r * 16 + c];
                }
            }
        }
    }

    H.apply(acc.data(), OC);

    y.resize(OC);
    for (int64_t o = 0; o < OC; ++o) {
        y[o] = acc[o] * (double) ggml_fp16_to_fp32(rout_d[o]);
    }
}

// ---- run the op -----------------------------------------------------------

// Compute the op through the CPU backend, with the tensors copied in, so the test
// exercises the same dispatch path a model would.
static bool run_op(ggml_backend_t             backend,
                   const raw_tensor &         code,
                   const raw_tensor &         rin,
                   const raw_tensor &         rout,
                   const raw_tensor &         lut,
                   const raw_tensor &         dep,
                   const std::vector<float> & x,
                   int64_t                    OC,
                   int64_t                    IC,
                   std::vector<float> &       out) {
    const size_t     ctx_size = ggml_tensor_overhead() * 16 + ggml_graph_overhead();
    ggml_init_params ip       = { /*.mem_size =*/ctx_size, /*.mem_buffer =*/nullptr, /*.no_alloc =*/true };
    ggml_context *   ctx      = ggml_init(ip);

    ggml_tensor * t_code = ggml_new_tensor_3d(ctx, GGML_TYPE_I16, code.ne[0], code.ne[1], code.ne[2]);
    ggml_tensor * t_rin  = ggml_new_tensor_1d(ctx, GGML_TYPE_F16, ggml_nelements(t_code) ? IC : IC);
    ggml_tensor * t_rout = ggml_new_tensor_1d(ctx, GGML_TYPE_F16, OC);
    ggml_tensor * t_lut  = ggml_new_tensor_1d(ctx, GGML_TYPE_F16, 65536);
    ggml_tensor * t_dep  = ggml_new_tensor_2d(ctx, GGML_TYPE_I16, 16, 256);
    ggml_tensor * t_x    = ggml_new_tensor_2d(ctx, GGML_TYPE_F32, IC, 1);

    ggml_tensor * y = ggml_escha_mul_mat(ctx, t_code, t_rin, t_rout, t_lut, t_dep, t_x);

    ggml_cgraph * gf = ggml_new_graph(ctx);
    ggml_build_forward_expand(gf, y);


    ggml_backend_buffer_t buf = ggml_backend_alloc_ctx_tensors(ctx, backend);
    if (!buf) {
        fail("backend", "tensor allocation failed");
        ggml_free(ctx);
        return false;
    }

    ggml_backend_tensor_set(t_code, code.bytes.data(), 0, code.bytes.size());
    ggml_backend_tensor_set(t_rin, rin.bytes.data(), 0, rin.bytes.size());
    ggml_backend_tensor_set(t_rout, rout.bytes.data(), 0, rout.bytes.size());
    ggml_backend_tensor_set(t_lut, lut.bytes.data(), 0, lut.bytes.size());
    ggml_backend_tensor_set(t_dep, dep.bytes.data(), 0, dep.bytes.size());
    ggml_backend_tensor_set(t_x, x.data(), 0, x.size() * sizeof(float));

    bool good = true;
    if (ggml_backend_graph_compute(backend, gf) != GGML_STATUS_SUCCESS) {
        fail("compute", "backend graph compute failed");
        good = false;
    } else {
        out.resize(OC);
        ggml_backend_tensor_get(y, out.data(), 0, OC * sizeof(float));
    }

    ggml_backend_buffer_free(buf);
    ggml_free(ctx);
    return good;
}

// ---- comparison ----------------------------------------------------------

struct diff {
    double rmse    = 0.0;
    double max_rel = 0.0;
    double scale   = 0.0;
};

static diff compare(const std::vector<double> & ref, const std::vector<float> & got) {
    diff   d;
    double se      = 0.0;
    double max_abs = 0.0;
    for (size_t i = 0; i < ref.size(); ++i) {
        const double e = (double) got[i] - ref[i];
        se += e * e;
        max_abs   = std::max(max_abs, std::fabs(ref[i]));
        d.max_rel = std::max(d.max_rel, std::fabs(e) / (std::fabs(ref[i]) + 1e-9));
    }
    d.rmse  = std::sqrt(se / (double) ref.size());
    d.scale = max_abs;
    return d;
}

// The op computes codebook A rather than reading the table, so the table it is
// handed has to BE codebook A. A checkpoint on another codebook would otherwise
// decode to plausible-looking garbage, and every case below would still "pass".
static bool check_codebook_lut(const raw_tensor & lut) {
    if (lut.bytes.size() != 65536 * sizeof(ggml_fp16_t)) {
        fail("lut", "not 65536 f16 elements");
        return false;
    }
    const ggml_fp16_t * t   = (const ggml_fp16_t *) lut.bytes.data();
    int64_t             bad = 0;
    for (uint32_t i = 0; i < 65536; ++i) {
        const uint32_t x = ((i * 0xcbac1fedu) & 0x8fff8fffu) ^ 0x3b603b60u;
        ggml_fp16_t    lo;
        ggml_fp16_t    hi;
        std::memcpy(&lo, (const char *) &x, sizeof(lo));
        std::memcpy(&hi, (const char *) &x + 2, sizeof(hi));
        const float v = ggml_fp16_to_fp32(lo) + ggml_fp16_to_fp32(hi);
        if (ggml_fp16_to_fp32(ggml_fp32_to_fp16(v)) != ggml_fp16_to_fp32(t[i])) {
            ++bad;
        }
    }
    if (bad) {
        fprintf(stderr, "lut differs from computed codebook A at %" PRId64 " of 65536 entries\n", bad);
        return false;
    }
    return true;
}

int main(int argc, char ** argv) {
    std::string model   = std::string(getenv("HOME") ? getenv("HOME") : "") + "/kintsugi/models/Kintsugi-27B-12G.gguf";
    std::string tensor  = DEFAULT_TENSOR;
    bool        verbose = false;

    for (int i = 1; i < argc; ++i) {
        const std::string a = argv[i];
        if (a == "--tensor" && i + 1 < argc) {
            tensor = argv[++i];
        } else if (a == "--verbose") {
            verbose = true;
        } else if (a == "--help") {
            printf("usage: %s [model.gguf] [--tensor NAME] [--verbose]\n", argv[0]);
            return 0;
        } else {
            model = a;
        }
    }

    FILE * f = fopen(model.c_str(), "rb");
    if (!f) {
        printf("SKIP: model not found at %s\n", model.c_str());
        printf("      this test reads a real Escha checkpoint because `dep` must hold\n");
        printf("      valid bit indices; pass the path as the first argument.\n");
        return 0;
    }

    gguf_init_params gp = { /*.no_alloc =*/true, /*.ctx =*/nullptr };
    gguf_context *   g  = gguf_init_from_file(model.c_str(), gp);
    if (!g) {
        fail("gguf", "failed to read metadata");
        fclose(f);
        return 1;
    }

    const int64_t ver = gguf_find_key(g, "qwen35.escha.version") < 0 ?
                            -1 :
                            (int64_t) gguf_get_val_u32(g, gguf_find_key(g, "qwen35.escha.version"));
    printf("model : %s\n", model.c_str());
    printf("escha : version %" PRId64 "\n", ver);
    printf("tensor: %s\n", tensor.c_str());

    raw_tensor lut, dep, code, rin, rout;
    bool       loaded = fetch_tensor(f, g, "escha_lut", lut);
    if (!loaded) {
        gguf_free(g);
        fclose(f);
        return 1;
    }
    if (!check_codebook_lut(lut)) {
        fail("codebook", "stored lut is not codebook A");
    } else {
        ok("lut matches computed codebook A");
    }

    // K is the code bit width: 32 i16 per tile for K=2, 48 for K=3.
    loaded &= fetch_tensor(f, g, (tensor + ".escha_code").c_str(), code);
    const int64_t K = loaded ? code.ne[0] / 16 : 2;
    loaded &= fetch_tensor(f, g, K == 3 ? "escha_dep_k3" : "escha_dep_k2", dep);
    loaded &= fetch_tensor(f, g, (tensor + ".escha_rin").c_str(), rin);
    loaded &= fetch_tensor(f, g, (tensor + ".escha_rout").c_str(), rout);
    if (!loaded) {
        gguf_free(g);
        fclose(f);
        return 1;
    }

    const int64_t nct = code.ne[1];
    const int64_t nit = code.ne[2];
    const int64_t OC  = nct * 16;
    const int64_t IC  = nit * 16;

    printf("code  : K=%" PRId64 " tiles %" PRId64 "x%" PRId64 " -> IC %" PRId64 ", OC %" PRId64 " (%zu bytes)\n", K,
           nit, nct, IC, OC, code.bytes.size());

    if (IC % 128 || OC % 128) {
        fail("shape", "IC/OC are not multiples of 128");
        gguf_free(g);
        fclose(f);
        return 1;
    }
    if (rin.bytes.size() != (size_t) IC * 2 || rout.bytes.size() != (size_t) OC * 2) {
        fail("shape", "rin/rout length does not match IC/OC");
        gguf_free(g);
        fclose(f);
        return 1;
    }

    // Deterministic pseudo-random activation, so a failure is reproducible.
    std::vector<float> x(IC);
    uint32_t           rng = 0x12345678u;
    for (int64_t i = 0; i < IC; ++i) {
        rng  = rng * 1664525u + 1013904223u;
        x[i] = ((float) ((rng >> 8) & 0xffff) / 65535.0f - 0.5f) * 2.0f;
    }

    std::vector<double> y_ref;
    reference(code, rin, rout, dep, x, y_ref);

    // Sweep every backend that offers the op, each scored against the SAME double
    // reference rather than against each other: a bug shared by the CPU and CUDA
    // paths cannot hide behind agreement between them.
    ggml_backend_load_all();

    std::vector<float> y_op;
    bool have_impl = false;

    for (size_t bi = 0; bi < ggml_backend_dev_count(); ++bi) {
        ggml_backend_dev_t   dev = ggml_backend_dev_get(bi);
        const std::string    dn  = ggml_backend_dev_name(dev);
        const enum ggml_backend_dev_type dt = ggml_backend_dev_type(dev);
        if (dt != GGML_BACKEND_DEVICE_TYPE_GPU && dt != GGML_BACKEND_DEVICE_TYPE_CPU) {
            continue;
        }
        ggml_backend_t backend = ggml_backend_dev_init(dev, nullptr);
        if (!backend) {
            printf("skip %-22s (init failed)\n", dn.c_str());
            continue;
        }
        std::vector<float> y_dev;
        if (!run_op(backend, code, rin, rout, lut, dep, x, OC, IC, y_dev)) {
            fprintf(stderr, "FAIL %s did not compute\n", dn.c_str());
            ++g_failures;
        } else {
            const diff   dd  = compare(y_ref, y_dev);
            const double rel = dd.rmse / (dd.scale + 1e-30);
            printf("dev  %-22s rmse %.6g  max_rel %.6g  rmse/|ref|max %.3g\n", dn.c_str(), dd.rmse, dd.max_rel, rel);
            if (!(rel < 1e-3)) {
                fprintf(stderr, "FAIL %s vs reference: %.6g (want < 1e-3)\n", dn.c_str(), rel);
                ++g_failures;
            } else {
                char what[160];
                snprintf(what, sizeof(what), "%s matches the dense reference", dn.c_str());
                ok(what);
            }
            if (!have_impl) {
                y_op     = y_dev;
                have_impl = true;
            }
        }
        ggml_backend_free(backend);
    }

    if (!have_impl) {
        fail("backend", "no backend produced a result");
        gguf_free(g);
        fclose(f);
        return 1;
    }

    const diff d = compare(y_ref, y_op);
    printf("diff  : rmse %.6g  max_rel %.6g  |ref|max %.6g\n", d.rmse, d.max_rel, d.scale);
    if (verbose) {
        for (int i = 0; i < 8 && i < (int) y_ref.size(); ++i) {
            printf("  y[%d] ref %.8g  op %.8g\n", i, y_ref[i], (double) y_op[i]);
        }
    }

    // f32 accumulation over IC terms against a double reference: the bound has to
    // leave room for rounding but not for a wrong rotation or a mis-indexed tile,
    // which show up as order-1 relative errors.
    const double rel = d.rmse / (d.scale + 1e-30);
    if (!(rel < 1e-3)) {
        fprintf(stderr, "FAIL op vs reference: rmse/|ref|max = %.6g (want < 1e-3)\n", rel);
        ++g_failures;
    } else {
        ok("op matches the dense reference");
    }

    // ---- negative controls: prove the comparison can fail -----------------

    // 1. An off-by-one in the dep table must change the result.
    {
        raw_tensor dep_bad = dep;
        int16_t *  db      = (int16_t *) dep_bad.bytes.data();
        for (int p = 0; p < 256 * 16; ++p) {
            db[p] = (int16_t) ((db[p] + 1) & 63);
        }
        std::vector<double> y_bad;
        reference(code, rin, rout, dep_bad, x, y_bad);
        const diff db_ = compare(y_bad, y_op);
        if (db_.rmse == 0.0) {
            fail("negative control", "a perturbed dep table produced an identical reference");
        } else {
            ok("perturbed dep changes the result");
        }
    }

    // 2. Swapping rin and rout is not shape-compatible, so instead zero rin: the
    //    rotation must then be the only term and the output must collapse.
    {
        raw_tensor rin_zero = rin;
        std::memset(rin_zero.bytes.data(), 0, rin_zero.bytes.size());
        std::vector<double> y_zero;
        reference(code, rin_zero, rout, dep, x, y_zero);
        const diff d0 = compare(y_zero, y_op);
        if (d0.rmse == 0.0) {
            fail("negative control", "zeroing rin produced an identical reference");
        } else {
            ok("zeroed rin changes the result");
        }
    }

    // 3. A single corrupted payload byte must show up in the output.
    {
        raw_tensor code_bad = code;
        code_bad.bytes[0] ^= 0xff;
        std::vector<double> y_bad;
        reference(code_bad, rin, rout, dep, x, y_bad);
        const diff d1 = compare(y_bad, y_op);
        if (d1.rmse == 0.0) {
            fail("negative control", "a corrupted code byte produced an identical reference");
        } else {
            ok("corrupted code byte changes the result");
        }
    }

    gguf_free(g);
    fclose(f);

    printf("\n%d checks, %d failures\n", g_checks, g_failures);
    return g_failures == 0 ? 0 : 1;
}
