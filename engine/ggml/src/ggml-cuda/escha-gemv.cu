#include "escha-gemv.cuh"
#include "unary.cuh"

// Small-batch path of ggml_escha_mul_mat: the GEMV kernel of exllamav3 (exl3/quant/exl3_gemv_kernel.cuh, MIT, Copyright (c) 2025 Turboderp).
//
//   y = T128(T128(x * rin) @ decode(code)) * rout
//
// The escha code tensor has the layout of the exllamav3 trellis tensor [k/16][n/16][16K], so the kernel reads it as it is.
// exllamav3 runs the three stages in one cooperative launch with grid.sync. A cooperative launch does not work in a CUDA graph capture, so the stages are plain kernels here.

#if !defined(GGML_USE_HIP) && !defined(GGML_USE_MUSA)

// The exllamav3 headers take these from their util.cuh and util.h, which include PyTorch headers.
#define CEIL_DIVIDE(x, size) (((x) + (size) - 1) / (size))

typedef struct __align__(8) half4 {
    half2 x;
    half2 y;
} half4;

union half2_uint32 {
    uint32_t as_uint32;
    half2 as_half2;
    __device__ half2_uint32(uint32_t val) : as_uint32(val) {}
};

union half_uint16 {
    uint16_t as_uint16;
    half as_half;
    __device__ half_uint16(uint16_t val) : as_uint16(val) {}
};

#include "exl3/quant/exl3_gemv_kernel.cuh"

#define ESCHA_GEMV_RS 0.088388347648f   // 1/sqrt(128), the fp32 constant of the format

// xh[row] = f16(T128(x[row] * rin)), one warp per 128 channels. fp32 until the last step, as in the escha reference.
static __global__ void escha_gemv_had_in(
        const float * __restrict__ x,
        const half  * __restrict__ rin,
        half        * __restrict__ xh,
        const int IC, const int n_rows, const int ne1,
        const int64_t nb_x1, const int64_t nb_x2) {
    const int nb = IC/128;
    const int w  = blockIdx.x*(blockDim.x/32) + threadIdx.x/32;
    if (w >= n_rows*nb) {
        return;
    }
    const int row = w/nb;
    const int blk = w%nb;

    const float * x_row = (const float *)((const char *) x + (int64_t)(row%ne1)*nb_x1 + (int64_t)(row/ne1)*nb_x2);
    had_fh_r_128_inner<true, false>(x_row + blk*128, xh + (int64_t) row*IC + blk*128, rin + blk*128, ESCHA_GEMV_RS);
}

// dst[row] = T128(mid[row]) * rout, one warp per 128 columns
// mid can be the memory of dst: a lane reads its 4 values before it writes them
static __global__ void escha_gemv_had_out(
        const float * mid,
        const half  * __restrict__ rout,
        float       * dst,
        const int OC, const int n_rows, const int ne1,
        const int64_t nb_d1, const int64_t nb_d2) {
    const int nb = OC/128;
    const int w  = blockIdx.x*(blockDim.x/32) + threadIdx.x/32;
    if (w >= n_rows*nb) {
        return;
    }
    const int row = w/nb;
    const int blk = w%nb;

    float * dst_row = (float *)((char *) dst + (int64_t)(row%ne1)*nb_d1 + (int64_t)(row/ne1)*nb_d2);
    had_ff_r_128_inner<false, true>(mid + (int64_t) row*OC + blk*128, dst_row + blk*128, rout + blk*128, ESCHA_GEMV_RS);
}

// dst[row] = T128(acc[row]*scales[row]) * rout for the int32 sums of an int8 GEMM, one warp per 128 columns. acc can be the memory of dst.
// Same steps as had_ff_r_128_inner (exl3/quant/hadamard_inner.cuh) with an int32 load.
static __global__ void escha_gemv_had_out_i32(
        const int32_t * acc,
        const float   * __restrict__ scales,
        const half    * __restrict__ rout,
        float         * dst,
        const int OC, const int n_rows, const int ne1,
        const int64_t nb_d1, const int64_t nb_d2) {
    const int nb = OC/128;
    const int w  = blockIdx.x*(blockDim.x/32) + threadIdx.x/32;
    if (w >= n_rows*nb) {
        return;
    }
    const int row = w/nb;
    const int blk = w%nb;
    const int t   = threadIdx.x%32;

    const int4  a = ((const int4 *)(acc + (int64_t) row*OC + blk*128))[t];
    const float s = scales[row];

    // __fmul_rn: no FMA with the sums below, so the fused SwiGLU epilogue gives the same bits
    const float v0 = __fmul_rn((float) a.x, s);
    const float v1 = __fmul_rn((float) a.y, s);
    const float v2 = __fmul_rn((float) a.z, s);
    const float v3 = __fmul_rn((float) a.w, s);
    const float s0 = v0 + v1;
    const float d0 = v0 - v1;
    const float s1 = v2 + v3;
    const float d1 = v2 - v3;
    float4 v = make_float4(s0 + s1, d0 + d1, s0 - s1, d0 - d1);

    shuffle_had_f2x32(v.x, v.y, t);
    shuffle_had_f2x32(v.z, v.w, t);

    const half4 r = ((const half4 *)(rout + blk*128))[t];
    v.x = v.x*ESCHA_GEMV_RS*__low2float(r.x);
    v.y = v.y*ESCHA_GEMV_RS*__high2float(r.x);
    v.z = v.z*ESCHA_GEMV_RS*__low2float(r.y);
    v.w = v.w*ESCHA_GEMV_RS*__high2float(r.y);

    float * dst_row = (float *)((char *) dst + (int64_t)(row%ne1)*nb_d1 + (int64_t)(row/ne1)*nb_d2);
    ((float4 *)(dst_row + blk*128))[t] = v;
}

// T128 of the 4 values of a lane, without the 1/sqrt(128): the steps of had_ff_r_128_inner
static __device__ __forceinline__ void escha_gemv_had_lane(float4 & v, const int t) {
    const float s0 = v.x + v.y;
    const float d0 = v.x - v.y;
    const float s1 = v.z + v.w;
    const float d1 = v.z - v.w;
    v = make_float4(s0 + s1, d0 + d1, s0 - s1, d0 - d1);
    shuffle_had_f2x32(v.x, v.y, t);
    shuffle_had_f2x32(v.z, v.w, t);
}

// h[row] = silu(T128(g[row]) * rout_g) * (T128(u[row]) * rout_u), one warp per 128 columns.
// I8: g and u are the int32 sums of an int8 GEMM, sg[row] and su[row] are their fp32 factors.
// XH = false: dst[row] = h[row] (fp32; g or u can be the memory of dst).
// XH = true:  dst is [rows][OC] fp16 and gets f16(T128(h[row] * rin)), the rotated input of the next projection (rin of that projection).
template <bool I8, bool XH>
static __global__ void escha_gemv_swiglu_out(
        const void  * g,
        const void  * u,
        const float * __restrict__ sg,
        const float * __restrict__ su,
        const half  * __restrict__ rout_g,
        const half  * __restrict__ rout_u,
        const half  * __restrict__ rin,
        void        * dst,
        const int OC, const int n_rows, const int ne1,
        const int64_t nb_d1, const int64_t nb_d2) {
    const int nb = OC/128;
    const int w  = blockIdx.x*(blockDim.x/32) + threadIdx.x/32;
    if (w >= n_rows*nb) {
        return;
    }
    const int row = w/nb;
    const int blk = w%nb;
    const int t   = threadIdx.x%32;

    const int64_t off = (int64_t) row*OC + blk*128;
    float4 a, b;
    if constexpr (I8) {
        const int4  ia = ((const int4 *)((const int32_t *) g + off))[t];
        const int4  ib = ((const int4 *)((const int32_t *) u + off))[t];
        const float fa = sg[row];
        const float fb = su[row];
        a = make_float4(__fmul_rn((float) ia.x, fa), __fmul_rn((float) ia.y, fa), __fmul_rn((float) ia.z, fa), __fmul_rn((float) ia.w, fa));
        b = make_float4(__fmul_rn((float) ib.x, fb), __fmul_rn((float) ib.y, fb), __fmul_rn((float) ib.z, fb), __fmul_rn((float) ib.w, fb));
    } else {
        a = ((const float4 *)((const float *) g + off))[t];
        b = ((const float4 *)((const float *) u + off))[t];
    }

    escha_gemv_had_lane(a, t);
    escha_gemv_had_lane(b, t);

    const half4 rg = ((const half4 *)(rout_g + blk*128))[t];
    const half4 ru = ((const half4 *)(rout_u + blk*128))[t];

    float4 o;
    o.x = ggml_cuda_op_silu_single(a.x*ESCHA_GEMV_RS*__low2float(rg.x))  * (b.x*ESCHA_GEMV_RS*__low2float(ru.x));
    o.y = ggml_cuda_op_silu_single(a.y*ESCHA_GEMV_RS*__high2float(rg.x)) * (b.y*ESCHA_GEMV_RS*__high2float(ru.x));
    o.z = ggml_cuda_op_silu_single(a.z*ESCHA_GEMV_RS*__low2float(rg.y))  * (b.z*ESCHA_GEMV_RS*__low2float(ru.y));
    o.w = ggml_cuda_op_silu_single(a.w*ESCHA_GEMV_RS*__high2float(rg.y)) * (b.w*ESCHA_GEMV_RS*__high2float(ru.y));

    if constexpr (XH) {
        const half4 ri = ((const half4 *)(rin + blk*128))[t];
        o.x *= __low2float(ri.x);
        o.y *= __high2float(ri.x);
        o.z *= __low2float(ri.y);
        o.w *= __high2float(ri.y);
        escha_gemv_had_lane(o, t);
        half4 h;
        h.x = __floats2half2_rn(o.x*ESCHA_GEMV_RS, o.y*ESCHA_GEMV_RS);
        h.y = __floats2half2_rn(o.z*ESCHA_GEMV_RS, o.w*ESCHA_GEMV_RS);
        ((half4 *)((half *) dst + off))[t] = h;
    } else {
        float * dst_row = (float *)((char *) dst + (int64_t)(row%ne1)*nb_d1 + (int64_t)(row/ne1)*nb_d2);
        ((float4 *)(dst_row + blk*128))[t] = o;
    }
}

// mid = xh @ decode(code): the main loop of exl3_gemv_kernel (shuffle extraction, mcg codebook, fp16 accumulate folded to fp32).
// A warp has WNT adjacent n-tiles and 1/S of k. A block has G column groups and S k-splits.
//   FUSE = false: G = 1, one block per column group as in exllamav3. C is mid [rows][OC].
//   FUSE = true:  G*WNT*16 = 128. The block has one Hadamard block of the output, it applies T128 and rout and writes dst.
// MMODE 0 is rows == 1, MMODE 1 is 2 <= rows <= 8.
template <int bits, int MMODE, int WNT, int S, bool FUSE>
static __global__ void __launch_bounds__(32*S*(FUSE ? 8/WNT : 1), S*(FUSE ? 8/WNT : 1) <= 16 ? 2 : 1) escha_gemv_kernel(
        const half     * __restrict__ A,
        const uint16_t * __restrict__ B,
        const half     * __restrict__ rout,
        float          * __restrict__ C,
        const int size_m, const int size_k, const int size_n, const int ne1,
        const int64_t nb_d1, const int64_t nb_d2, const int row0) {
#ifdef AMPERE_MMA_AVAILABLE
    constexpr int cb   = 1;   // mcg
    constexpr int G    = FUSE ? 8/WNT : 1;
    constexpr int PF   = WNT == 2 ? 4 : 2;   // prefetch ring depth
    constexpr int FOLD = WNT == 2 ? 4 : 2;   // fp16->fp32 fold cadence (divides PF)
    constexpr int THREADS = 32*S*G;
    constexpr int ROWS  = MMODE == 0 ? 1 : EXL3_GEMV_MAX_M;
    constexpr int COLS  = WNT*16;
    constexpr int BCOLS = G*COLS;

    constexpr int TWORDS = 8*bits;                               // uint32 per 16x16 tile
    constexpr bool TWO_PER_LOAD = bits == 2;                     // two tiles per warp load
    constexpr int LOADS   = TWO_PER_LOAD ? WNT/2 : WNT;          // warp loads per k-slice
    constexpr int LSTRIDE = TWO_PER_LOAD ? 2*TWORDS : TWORDS;    // uint32 per load (lanes < LSTRIDE load)
    static_assert(bits == 2 || bits == 3, "2 and 3 bpw only");
    static_assert(WNT == 2 || WNT == 4, "2 or 4 n-tiles per warp");
    static_assert(!FUSE || S*G >= ROWS, "the output transform needs one warp per row");

    const int warp = threadIdx.x/32;
    const int lane = threadIdx.x%32;
    const int grp  = warp/S;
    const int ksp  = warp%S;

    const int ntiles  = size_n/16;
    const int kslices = size_k/16;

    const int chunk = CEIL_DIVIDE(kslices, S);
    const int ks0 = ksp*chunk;
    const int myn = max(0, min(chunk, kslices - ks0));

    const uint32_t * B32 = (const uint32_t *) B;
    const size_t slice_stride = (size_t) ntiles*TWORDS;   // uint32 per k-slice row
    const half2 * A2 = (const half2 *) A;
    const half2 hzero = __half2half2(__ushort_as_half(0));

    // A fragment row indices for this lane
    const int r0 = lane >> 2;
    const size_t a_row0 = (size_t) r0*(size_k/2);
    const bool r0_ok = MMODE == 0 ? lane < 4 : r0 < size_m;

    // per-lane extraction constants (see dq8_aligned_2bits / dq8<3, cb, 4> in exl3_dq.cuh)
    int x_src_a = 0, x_src_b = 0;
    [[maybe_unused]] int x_s2 = 0;
    if constexpr (bits == 2) {
        const int i1 = lane >> 1;
        x_src_b = i1;
        x_src_a = (i1 + 15) & 15;
    } else {
        const int t_offset = lane << 3;
        const int b1 = (t_offset + 257)*3;
        const int b2 = b1 + 21;
        const int i0 = (b1 - 16)/32;
        const int i2 = (b2 - 1)/32;
        x_s2 = (i2 + 1)*32 - b2;
        x_src_a = i0%24;
        x_src_b = i2%24;
    }

    __shared__ __align__(16) float sh_red[S][ROWS][BCOLS];

    const int group = blockIdx.x*G + grp;
    const uint32_t * bp = B32 + (size_t) ks0*slice_stride + group*WNT*TWORDS + lane;

    // prefetch ring (indices must be compile-time or pf lands in local memory)
    auto ld_b = [&] (int i, int l) -> uint32_t {
        if constexpr (LSTRIDE < 32) {
            return lane < LSTRIDE ? __ldcs(bp + (size_t) i*slice_stride + l*LSTRIDE) : 0;
        } else {
            return __ldcs(bp + (size_t) i*slice_stride + l*LSTRIDE);
        }
    };

    uint32_t pf[PF][LOADS];
#pragma unroll
    for (int d = 0; d < PF; ++d) {
        if (d < myn) {
#pragma unroll
            for (int l = 0; l < LOADS; ++l) {
                pf[d][l] = ld_b(d, l);
            }
        }
    }

    FragC_h ch[WNT][2] = {};
    float2 acc0[WNT][2] = {};

    for (int ib = 0; ib < myn; ib += PF) {
#pragma unroll
        for (int d = 0; d < PF; ++d) {
            const int i = ib + d;
            if (i >= myn) {
                break;
            }

            uint32_t bw[LOADS];
#pragma unroll
            for (int l = 0; l < LOADS; ++l) {
                bw[l] = pf[d][l];
            }

            if (i + PF < myn) {
#pragma unroll
                for (int l = 0; l < LOADS; ++l) {
                    pf[d][l] = ld_b(i + PF, l);
                }
            }

            // A fragment: lane covers row lane/4, k pairs (2(lane%4), +1) and (+8, +9)
            const size_t a_col = (size_t) (ks0 + i)*8 + (lane & 3);
            FragB a01, a23;
            a01[0] = r0_ok ? A2[a_row0 + a_col] : hzero;
            a23[0] = r0_ok ? A2[a_row0 + a_col + 4] : hzero;
            a01[1] = hzero;
            a23[1] = hzero;

#pragma unroll
            for (int t = 0; t < WNT; ++t) {
                FragB f0, f1;
                if constexpr (bits == 2) {
                    // two tiles per loaded word group: tile t lives in lanes (t&1)*16 .. +15
                    const uint32_t w = bw[t >> 1];
                    const int base = (t & 1) << 4;
                    const uint32_t bwv = __shfl_sync(0xffffffffu, w, base + x_src_b);
                    const uint32_t awv = __shfl_sync(0xffffffffu, w, base + x_src_a);
                    exl3_gemv_ns::dq8_regs_2bits<cb>(awv, bwv, lane << 3, f0, f1);
                } else {
                    const uint32_t awv = __shfl_sync(0xffffffffu, bw[t], x_src_a);
                    const uint32_t bwv = __shfl_sync(0xffffffffu, bw[t], x_src_b);
                    exl3_gemv_ns::dq8_regs_3bits<cb>(awv, bwv, x_s2, f0, f1);
                }

                exl3_gemv_ns::mma_ab_h(a01, a23, f0, ch[t][0]);
                exl3_gemv_ns::mma_ab_h(a01, a23, f1, ch[t][1]);
            }

            if ((d + 1)%FOLD == 0 || i + 1 == myn) {
#pragma unroll
                for (int t = 0; t < WNT; ++t) {
#pragma unroll
                    for (int f = 0; f < 2; ++f) {
                        acc0[t][f].x += __low2float(ch[t][f][0]);
                        acc0[t][f].y += __high2float(ch[t][f][0]);
                        ch[t][f][0] = hzero;
                    }
                }
            }
        }
    }

    // reduction over the k splits. Lane l holds row l/4, cols tile*16 + frag*8 + 2*(l%4) (+1)
    {
        const int c0 = grp*COLS + 2*(lane & 3);
        const bool store0 = MMODE == 0 ? lane < 4 : r0 < ROWS;
        const int sr0 = MMODE == 0 ? 0 : r0;
        if (store0) {
#pragma unroll
            for (int t = 0; t < WNT; ++t) {
#pragma unroll
                for (int f = 0; f < 2; ++f) {
                    const int col = t*16 + f*8 + c0;
                    sh_red[ksp][sr0][col + 0] = acc0[t][f].x;
                    sh_red[ksp][sr0][col + 1] = acc0[t][f].y;
                }
            }
        }
    }
    __syncthreads();

    // fixed order, so the result is the same from run to run
    const int rows_out = MMODE == 0 ? 1 : min(size_m, ROWS);
    for (int idx = threadIdx.x; idx < BCOLS*rows_out; idx += THREADS) {
        const int r = idx/BCOLS;
        const int c = idx%BCOLS;
        float sum = 0.0f;
#pragma unroll
        for (int j = 0; j < S; ++j) {
            sum += sh_red[j][r][c];
        }
        if constexpr (FUSE) {
            sh_red[0][r][c] = sum;
        } else {
            C[(size_t) r*size_n + blockIdx.x*BCOLS + c] = sum;
        }
    }

    if constexpr (FUSE) {
        __syncthreads();
        // warp r does row r
        if (warp < rows_out) {
            const int orow = row0 + warp;
            float * dst_row = (float *)((char *) C + (int64_t)(orow%ne1)*nb_d1 + (int64_t)(orow/ne1)*nb_d2);
            had_ff_r_128_inner<false, true>(sh_red[0][warp], dst_row + blockIdx.x*128, rout + blockIdx.x*128, ESCHA_GEMV_RS);
        }
    }
#else
    GGML_UNUSED_VARS(A, B, rout, C, size_m, size_k, size_n, ne1, nb_d1, nb_d2, row0);
    NO_DEVICE_CODE;
#endif // AMPERE_MMA_AVAILABLE
}

// the Hadamard kernels load and store 4 floats at a time
static bool escha_gemv_rows_aligned(const ggml_tensor * t) {
    return (uintptr_t) t->data%16 == 0 && t->nb[1]%16 == 0 && t->nb[2]%16 == 0;
}

// GGML_ESCHA_GEMV_MAX_ROWS=n (default 48, 8 = one chunk only): a batch of 9 to n rows runs as chunks of 8 rows.
// A chunk costs about as much as 1 row, and the tiled prefill kernel costs the same for 1 to 128 rows (about 6 chunks). A row has the same bits alone and in a chunk.
int ggml_cuda_escha_gemv_max_rows() {
    static const int max_rows = [] {
        const char * e = getenv("GGML_ESCHA_GEMV_MAX_ROWS");
        return e != nullptr ? MAX(EXL3_GEMV_MAX_M, atoi(e)) : 48;
    }();
    return max_rows;
}

bool ggml_cuda_escha_gemv(ggml_backend_cuda_context & ctx, ggml_tensor * dst) {
    const ggml_tensor * code = dst->src[0];
    const ggml_tensor * rin  = dst->src[1];
    const ggml_tensor * rout = dst->src[2];
    const ggml_tensor * x    = dst->src[5];

    static const bool enabled = [] {
        const char * e = getenv("GGML_ESCHA_GEMV");
        return e == nullptr || atoi(e) != 0;
    }();
    // for experiments: narrow and wide are the two blocks of exllamav3, exl3 is its choice between them
    enum { CFG_FUSED, CFG_NARROW, CFG_WIDE, CFG_EXL3 };
    static const int cfg_env = [] () -> int {
        const char * e = getenv("GGML_ESCHA_GEMV_CFG");
        if (e == nullptr) {
            return CFG_FUSED;
        }
        return strcmp(e, "narrow") == 0 ? CFG_NARROW : strcmp(e, "wide") == 0 ? CFG_WIDE : strcmp(e, "exl3") == 0 ? CFG_EXL3 : CFG_FUSED;
    }();

    const int K  = code->ne[0]/16;
    const int OC = code->ne[1]*16;
    const int IC = code->ne[2]*16;
    const int n_rows = x->ne[1]*x->ne[2];

    if (!enabled || n_rows < 1 || n_rows > ggml_cuda_escha_gemv_max_rows() || (K != 2 && K != 3) || IC%128 != 0 || OC%128 != 0) {
        return false;
    }
    if (!ampere_mma_available(ggml_cuda_info().devices[ctx.device].cc)) {
        return false;
    }
    if (!escha_gemv_rows_aligned(x) || !escha_gemv_rows_aligned(dst)) {
        return false;
    }

    cudaStream_t stream = ctx.stream();

    ggml_cuda_pool_alloc<half> xh(ctx.pool(), (size_t) n_rows*IC);
    escha_gemv_had_in<<<(n_rows*(IC/128) + 7)/8, 256, 0, stream>>>(
        (const float *) x->data, (const half *) rin->data, xh.get(),
        IC, n_rows, (int) x->ne[1], x->nb[1], x->nb[2]);
    CUDA_CHECK(cudaGetLastError());

    // exllamav3 on Ada (exl3_gemv_cfg): narrow up to n = 8192, wide above
    const int cfg = cfg_env != CFG_EXL3 ? cfg_env : OC <= 8192 ? CFG_NARROW : CFG_WIDE;
    const bool fuse = cfg == CFG_FUSED;
    ggml_cuda_pool_alloc<float> mid(ctx.pool());
    if (!fuse) {
        mid.alloc((size_t) n_rows*OC);
    }

    for (int row0 = 0; row0 < n_rows; row0 += EXL3_GEMV_MAX_M) {
        const int m     = MIN(EXL3_GEMV_MAX_M, n_rows - row0);
        const int mmode = m == 1 ? 0 : 1;

        auto launch = [&](auto kernel, const int grid, const int threads) {
            kernel<<<grid, threads, 0, stream>>>(
                xh.get() + (size_t) row0*IC, (const uint16_t *) code->data, (const half *) rout->data,
                fuse ? (float *) dst->data : mid.get() + (size_t) row0*OC,
                m, IC, OC, (int) x->ne[1], dst->nb[1], dst->nb[2], row0);
        };

#define ESCHA_GEMV_LAUNCH(bits, mm) \
        if (cfg == CFG_FUSED) { \
            launch(escha_gemv_kernel<bits, mm, 2, 8, true>, OC/128, 1024); \
        } else if (cfg == CFG_NARROW) { \
            launch(escha_gemv_kernel<bits, mm, 2, 16, false>, OC/32, 512); \
        } else { \
            launch(escha_gemv_kernel<bits, mm, 4, 8, false>, OC/64, 256); \
        }

        switch (2*K + mmode) {
            case 4: ESCHA_GEMV_LAUNCH(2, 0); break;
            case 5: ESCHA_GEMV_LAUNCH(2, 1); break;
            case 6: ESCHA_GEMV_LAUNCH(3, 0); break;
            case 7: ESCHA_GEMV_LAUNCH(3, 1); break;
        }
#undef ESCHA_GEMV_LAUNCH
    }
    CUDA_CHECK(cudaGetLastError());

    if (!fuse) {
        escha_gemv_had_out<<<(n_rows*(OC/128) + 7)/8, 256, 0, stream>>>(
            mid.get(), (const half *) rout->data, (float *) dst->data,
            OC, n_rows, (int) x->ne[1], dst->nb[1], dst->nb[2]);
        CUDA_CHECK(cudaGetLastError());
    }

    return true;
}

bool ggml_cuda_escha_had_in(ggml_backend_cuda_context & ctx, const ggml_tensor * x, const ggml_tensor * rin, half * xh) {
    if (!escha_gemv_rows_aligned(x) || (uintptr_t) xh%16 != 0) {
        return false;
    }
    const int IC     = x->ne[0];
    const int n_rows = x->ne[1]*x->ne[2];
    escha_gemv_had_in<<<(n_rows*(IC/128) + 7)/8, 256, 0, ctx.stream()>>>(
        (const float *) x->data, (const half *) rin->data, xh,
        IC, n_rows, (int) x->ne[1], x->nb[1], x->nb[2]);
    CUDA_CHECK(cudaGetLastError());
    return true;
}

bool ggml_cuda_escha_had_out(ggml_backend_cuda_context & ctx, const float * mid, const ggml_tensor * rout, ggml_tensor * dst) {
    if (!escha_gemv_rows_aligned(dst) || (uintptr_t) mid%16 != 0) {
        return false;
    }
    const int OC     = dst->ne[0];
    const int n_rows = dst->ne[1]*dst->ne[2];
    escha_gemv_had_out<<<(n_rows*(OC/128) + 7)/8, 256, 0, ctx.stream()>>>(
        mid, (const half *) rout->data, (float *) dst->data,
        OC, n_rows, (int) dst->ne[1], dst->nb[1], dst->nb[2]);
    CUDA_CHECK(cudaGetLastError());
    return true;
}

bool ggml_cuda_escha_had_out_i32(ggml_backend_cuda_context & ctx, const int32_t * acc, const float * scales, const ggml_tensor * rout, ggml_tensor * dst) {
    if (!escha_gemv_rows_aligned(dst) || (uintptr_t) acc%16 != 0) {
        return false;
    }
    const int OC     = dst->ne[0];
    const int n_rows = dst->ne[1]*dst->ne[2];
    escha_gemv_had_out_i32<<<(n_rows*(OC/128) + 7)/8, 256, 0, ctx.stream()>>>(
        acc, scales, (const half *) rout->data, (float *) dst->data,
        OC, n_rows, (int) dst->ne[1], dst->nb[1], dst->nb[2]);
    CUDA_CHECK(cudaGetLastError());
    return true;
}

bool ggml_cuda_escha_swiglu_out(ggml_backend_cuda_context & ctx, const void * g, const void * u, const float * sg, const float * su,
                                const ggml_tensor * rout_g, const ggml_tensor * rout_u, ggml_tensor * dst) {
    if (!escha_gemv_rows_aligned(dst) || (uintptr_t) g%16 != 0 || (uintptr_t) u%16 != 0) {
        return false;
    }
    const int OC     = dst->ne[0];
    const int n_rows = dst->ne[1]*dst->ne[2];
    const int grid   = (n_rows*(OC/128) + 7)/8;
    if (sg != nullptr) {
        escha_gemv_swiglu_out<true, false><<<grid, 256, 0, ctx.stream()>>>(
            g, u, sg, su, (const half *) rout_g->data, (const half *) rout_u->data, nullptr, dst->data,
            OC, n_rows, (int) dst->ne[1], dst->nb[1], dst->nb[2]);
    } else {
        escha_gemv_swiglu_out<false, false><<<grid, 256, 0, ctx.stream()>>>(
            g, u, sg, su, (const half *) rout_g->data, (const half *) rout_u->data, nullptr, dst->data,
            OC, n_rows, (int) dst->ne[1], dst->nb[1], dst->nb[2]);
    }
    CUDA_CHECK(cudaGetLastError());
    return true;
}

bool ggml_cuda_escha_swiglu_xh(ggml_backend_cuda_context & ctx, const void * g, const void * u, const float * sg, const float * su,
                               const ggml_tensor * rout_g, const ggml_tensor * rout_u, const ggml_tensor * rin, half * xh,
                               const int OC, const int n_rows) {
    if ((uintptr_t) g%16 != 0 || (uintptr_t) u%16 != 0 || (uintptr_t) xh%16 != 0) {
        return false;
    }
    const int grid = (n_rows*(OC/128) + 7)/8;
    if (sg != nullptr) {
        escha_gemv_swiglu_out<true, true><<<grid, 256, 0, ctx.stream()>>>(
            g, u, sg, su, (const half *) rout_g->data, (const half *) rout_u->data, (const half *) rin->data, xh,
            OC, n_rows, 1, 0, 0);
    } else {
        escha_gemv_swiglu_out<false, true><<<grid, 256, 0, ctx.stream()>>>(
            g, u, sg, su, (const half *) rout_g->data, (const half *) rout_u->data, (const half *) rin->data, xh,
            OC, n_rows, 1, 0, 0);
    }
    CUDA_CHECK(cudaGetLastError());
    return true;
}

#else

int ggml_cuda_escha_gemv_max_rows() {
    return 0;
}

bool ggml_cuda_escha_swiglu_out(ggml_backend_cuda_context & ctx, const void * g, const void * u, const float * sg, const float * su,
                                const ggml_tensor * rout_g, const ggml_tensor * rout_u, ggml_tensor * dst) {
    GGML_UNUSED_VARS(ctx, g, u, sg, su, rout_g, rout_u, dst);
    return false;
}

bool ggml_cuda_escha_swiglu_xh(ggml_backend_cuda_context & ctx, const void * g, const void * u, const float * sg, const float * su,
                               const ggml_tensor * rout_g, const ggml_tensor * rout_u, const ggml_tensor * rin, half * xh,
                               const int OC, const int n_rows) {
    GGML_UNUSED_VARS(ctx, g, u, sg, su, rout_g, rout_u, rin, xh, OC, n_rows);
    return false;
}

bool ggml_cuda_escha_had_out_i32(ggml_backend_cuda_context & ctx, const int32_t * acc, const float * scales, const ggml_tensor * rout, ggml_tensor * dst) {
    GGML_UNUSED_VARS(ctx, acc, scales, rout, dst);
    return false;
}

bool ggml_cuda_escha_gemv(ggml_backend_cuda_context & ctx, ggml_tensor * dst) {
    GGML_UNUSED_VARS(ctx, dst);
    return false;
}

bool ggml_cuda_escha_had_in(ggml_backend_cuda_context & ctx, const ggml_tensor * x, const ggml_tensor * rin, half * xh) {
    GGML_UNUSED_VARS(ctx, x, rin, xh);
    return false;
}

bool ggml_cuda_escha_had_out(ggml_backend_cuda_context & ctx, const float * mid, const ggml_tensor * rout, ggml_tensor * dst) {
    GGML_UNUSED_VARS(ctx, mid, rout, dst);
    return false;
}

#endif // !defined(GGML_USE_HIP) && !defined(GGML_USE_MUSA)
