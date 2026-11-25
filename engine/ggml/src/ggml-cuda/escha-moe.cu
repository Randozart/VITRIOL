#include "common.cuh"
#include "escha-moe.cuh"
#include "escha-gemv.cuh"
#include "mmid.cuh"
#include "mma.cuh"
#include <cuda_pipeline.h>

// Fused decode + routed matmul for Escha ESCHAM experts.
//
//   y = T128(T128(x * rin) @ decode(code)) * rout
//
// Weights decode independently -- weight[p] = lut[sum_j bit(payload, dep[p][j]) << j],
// no trellis state -- so the decode is a plain gather and the whole thing is one big
// embarrassingly parallel reduction.
//
// Split into three kernels because at batch 1 there is very little natural
// parallelism: 8 slots x OC/128 column groups is 32 blocks for gate/up, which leaves
// most of the GPU idle. Slicing the IC reduction across blocks and summing the
// partials afterwards is what fills it. The partials are summed in a fixed order
// rather than with atomics so results stay bit-reproducible run to run.
//
// There are two matmul kernels, picked by batch size:
//
//   escha_matmul_partial  one row per block, reduction sliced. Right when rows are
//                         scarce, but every row decodes the weights again.
//   escha_matmul_tiled    ESCHA_ROWS rows of ONE expert per block, so a decoded
//                         weight is reused across all of them. Needs the rows
//                         grouped by expert first, which is what mm_ids_helper
//                         already does for mul_mat_id.
//
// Both write the same partial layout and share escha_finalize.

#define ESCHA_TILE     16   // decode tile is 16x16
#define ESCHA_NT      128   // threads per block, one per output column
#define ESCHA_GROUPS  (ESCHA_NT/ESCHA_TILE)
#define ESCHA_MAX_W    24   // uint32 words per payload, 48 int16 at K=3
#define ESCHA_TARGET  512   // block count we try to reach by slicing the reduction
#define ESCHA_ROWS     16   // rows of one expert per block on the batched path
// dense reuses every decoded weight across this many rows. the decode, not bandwidth, is
// the limit (see the routed path's ablation), so this is the main prefill lever: 16 -> 64
// took a perplexity pass from 47.2 s to 18.9 s. kept separate from ESCHA_ROWS so the routed
// path stays exactly as measured.
//
// But acc[R] is per thread whatever the real row count is, so a big R costs occupancy at
// batch 1 and cripples generation (R=64 measured 1.84 t/s). Generation gets its own small
// instantiation instead -- the decode work per token is the same either way, so all that
// matters there is filling the device.
#define ESCHA_ROWS_DENSE      64
#define ESCHA_ROWS_DENSE_GEN   1
#define ESCHA_ROWS_DENSE_SPEC  4   // speculative verify is 2-8 tokens; reuse decode across them
#define ESCHA_GEN_MAX_ROWS    16   // at or below this, stay off the 128-row MMA prefill tile
#define ESCHA_GEN_TARGET_MUL   4
// prefill tile for the register-tiled kernel. BM*BN = TM*TN*NT, so these four fix the
// thread count too: NT = (BM/TM)*(BN/TN). BN stays 128 -- activation traffic is
// rows*IC*(OC/BN), so a narrower BN buys reuse with global bandwidth, which is a losing trade.
#define ESCHA_BM 128
#define ESCHA_BN 128
#define ESCHA_MMA_BM 128   // tensor-core prefill tile. Accumulators per thread are
#define ESCHA_MMA_BN 128   //   BM*BN/256, so BM drives register pressure and occupancy.
#define ESCHA_TM   8
#define ESCHA_TN   8
// prefill by reconstruction (escha_mul_mat_recon); the environment can change each value
#define ESCHA_PREFILL_DEFAULT       ESCHA_PREFILL_RECON
#define ESCHA_I8_WSCALE             32.0f   // int8 weights of the i8 path: round(w*32)
#define ESCHA_I8_MIN_ROWS           49
#define ESCHA_RECON_MIN_ROWS        448   // the decode to fp16 costs as much as the GEMM gain of about 390 rows (4070: slower at 256 rows, 1.2x at 1024)
#define ESCHA_RECON_MIB             64    // dense weight scratch: 256 is not faster, 32 is 2 % slower
                                   //     prefill: at batch 1 there are only n_ocb blocks
                                   //     before slicing (136 for the FFN), which leaves an
                                   //     82-SM device idle. Swept 1/2/4/8/16/32 -- flat from
                                   //     10.3 to 11.3 tok/s, 4 marginally best.

// Escha codebook A, the one this checkpoint uses (its config leaves "codebook" unset,
// which eschamoe.py defaults to cbA / codebook_id 1). It is computed, not stored -- the
// same QTIP-family trick as 3INST but with its own multiplier and no addend. Recovered
// from escham_reconstruct_kernel<1, K> and checked against all 65536 entries.
// The fp16 add must stay in fp16 to match the table bit for bit.
static __device__ __forceinline__ float escha_codebook(uint32_t idx) {
    // (x & 0x8fff8fff) ^ 0x3b603b60 is one 3-input logic op; spelling it as lop3 stops
    // ptxas from splitting it across the two 16-bit lanes. immLut 0x6a = (a & b) ^ c.
    uint32_t x = idx*0xcbac1fedu;
    asm("lop3.b32 %0, %1, %2, %3, 0x6a;"
        : "=r"(x) : "r"(x), "n"(0x8fff8fffu), "n"(0x3b603b60u));
    __half2 h;
    memcpy(&h, &x, sizeof(h));
    return __half2float(__hadd(__low2half(h), __high2half(h)));
}

// as escha_codebook, but stopping at the half. The codebook's last operation is already
// an fp16 add, so the fp16 weight is exact -- only the ACTIVATIONS lose precision below.
static __device__ __forceinline__ half escha_codebook_h(uint32_t idx) {
    uint32_t x = idx*0xcbac1fedu;
    asm("lop3.b32 %0, %1, %2, %3, 0x6a;"
        : "=r"(x) : "r"(x), "n"(0x8fff8fffu), "n"(0x3b603b60u));
    __half2 h;
    memcpy(&h, &x, sizeof(h));
    return __hadd(__low2half(h), __high2half(h));
}

// in-place normalized Sylvester-Hadamard over each block of 128
static __device__ __forceinline__ void escha_hadamard_128(float * v, int n, int tid, int nt) {
    for (int len = 1; len < 128; len <<= 1) {
        for (int idx = tid; idx < (n/128)*64; idx += nt) {
            const int blk = idx / 64;
            const int j   = idx % 64;
            const int i   = (j / len)*(2*len) + (j % len);

            float * b = v + blk*128 + i;
            const float a0 = b[0];
            const float a1 = b[len];

            b[0]   = a0 + a1;
            b[len] = a0 - a1;
        }
        __syncthreads();
    }

    const float scale = rsqrtf(128.0f);
    for (int i = tid; i < n; i += nt) {
        v[i] *= scale;
    }
    __syncthreads();
}

// u[row] = T128(x[row] * rin[expert]) -- hoisted out of the matmul so the column
// blocks do not each redo it
static __global__ void escha_rotate_in(
        const half    * __restrict__ rin,
        const float   * __restrict__ x,
        const int32_t * __restrict__ ids,
        float         * __restrict__ u,
        const int IC, const int n_x, const int n_ids,
        const int64_t nb_x1, const int64_t nb_x2,
        const int64_t nb_i0, const int64_t nb_i1) {
    extern __shared__ float s_u[];

    const int tid = threadIdx.x;
    const int row = blockIdx.x;
    const int it  = row / n_ids;
    const int is  = row % n_ids;

    const int32_t e = *(const int32_t *)((const char *) ids + is*nb_i0 + it*nb_i1);

    const half  * rin_e = rin + (int64_t) e*IC;
    const float * x_row = (const float *)((const char *) x + (int64_t)(is % n_x)*nb_x1 + it*nb_x2);

    for (int i = tid; i < IC; i += blockDim.x) {
        s_u[i] = x_row[i]*__half2float(rin_e[i]);
    }
    __syncthreads();

    escha_hadamard_128(s_u, IC, tid, blockDim.x);

    float * dst = u + (int64_t) row*IC;
    for (int i = tid; i < IC; i += blockDim.x) {
        dst[i] = s_u[i];
    }
}

// partial[slice][row][c] = sum over this slice's input tiles of u . decode(code)
template <int K>
static __global__ void escha_matmul_partial(
        const int16_t * __restrict__ code,
        const half    * __restrict__ lut,
        const int16_t * __restrict__ dep,
        const float   * __restrict__ u,
        const int32_t * __restrict__ ids,
        float         * __restrict__ partial,
        const int IC, const int OC, const int n_ids, const int n_rows, const int n_slices,
        const int64_t nb_i0, const int64_t nb_i1) {
    extern __shared__ char s_raw[];

    // dep is stored transposed and packed two entries per word: [b/2][r][cc]. that makes
    // the 16 threads of a group read 16 consecutive words, which is conflict-free -- the
    // natural [p][b] layout puts them 32 bytes apart and costs an 8-way bank conflict
    uint32_t * s_dep = (uint32_t *) s_raw;                 // [8][16][16]
    uint32_t * s_pay = s_dep + 8*256;                      // [ESCHA_GROUPS][ESCHA_MAX_W]

    const int nit  = IC/ESCHA_TILE;
    const int nct  = OC/ESCHA_TILE;
    const int n_wd = (16*K)/2;

    const int tid   = threadIdx.x;
    const int row   = blockIdx.x;
    const int ocb   = blockIdx.y;
    const int slice = blockIdx.z;

    const int it = row / n_ids;
    const int is = row % n_ids;

    const int32_t e = *(const int32_t *)((const char *) ids + is*nb_i0 + it*nb_i1);
    const int16_t * code_e = code + (int64_t) e*nit*nct*(16*K);

    for (int j = tid; j < 8*256; j += ESCHA_NT) {
        const int b2 = j / 256;
        const int p  = j % 256;
        s_dep[j] = (uint32_t) (uint16_t) dep[p*16 + 2*b2]
                 | ((uint32_t) (uint16_t) dep[p*16 + 2*b2 + 1] << 16);
    }
    __syncthreads();

    const int grp = tid / ESCHA_TILE;
    const int cc  = tid % ESCHA_TILE;
    const int tj  = ocb*ESCHA_GROUPS + grp;

    const int per   = nit/n_slices;
    const int ti0   = slice*per;
    const float * u_row = u + (int64_t) row*IC;

    uint32_t * pay = s_pay + grp*ESCHA_MAX_W;
    float sum = 0.0f;

    for (int ti = ti0; ti < ti0 + per; ++ti) {
        const uint32_t * src = (const uint32_t *)(code_e + (int64_t)(ti*nct + tj)*(16*K));
        for (int w = cc; w < n_wd; w += ESCHA_TILE) {
            pay[w] = src[w];
        }
        __syncwarp();

        const float * uu = u_row + ti*ESCHA_TILE;

        #pragma unroll 4
        for (int r = 0; r < ESCHA_TILE; ++r) {
            const uint32_t * d = s_dep + r*ESCHA_TILE + cc;

            uint32_t idx = 0;
            #pragma unroll
            for (int b2 = 0; b2 < 8; ++b2) {
                const uint32_t dd = d[b2*256];
                const int d0 = dd & 0xffff;
                const int d1 = dd >> 16;

                idx |= ((pay[d0 >> 5] >> (d0 & 31)) & 1u) << (2*b2);
                idx |= ((pay[d1 >> 5] >> (d1 & 31)) & 1u) << (2*b2 + 1);
            }

            sum += uu[r]*escha_codebook(idx);
        }
        __syncwarp();
    }

    partial[((int64_t) slice*n_rows + row)*OC + ocb*ESCHA_NT + tid] = sum;
}

// one work item per block of the tiled kernel: up to ESCHA_ROWS compact rows of expert e.
// order does not matter -- items write disjoint output rows -- so an atomic counter is enough
static __global__ void escha_build_work(
        const int32_t * __restrict__ bounds,
        int4          * __restrict__ work,
        int32_t       * __restrict__ n_work,
        const int n_expert) {
    for (int e = blockIdx.x*blockDim.x + threadIdx.x; e < n_expert; e += gridDim.x*blockDim.x) {
        const int lo = bounds[e];
        const int hi = bounds[e + 1];
        for (int s = lo; s < hi; s += ESCHA_ROWS) {
            work[atomicAdd(n_work, 1)] = make_int4(e, s, min(ESCHA_ROWS, hi - s), 0);
        }
    }
}

// partial[row][c] = u[row] . decode(code), for R rows sharing one expert
template <int K, int R>
static __global__ void escha_matmul_tiled(
        const int16_t * __restrict__ code,
        const half    * __restrict__ lut,
        const int16_t * __restrict__ dep,
        const float   * __restrict__ u,
        const int32_t * __restrict__ ids_dst,
        const int4    * __restrict__ work,
        const int32_t * __restrict__ n_work,
        float         * __restrict__ partial,
        const int IC, const int OC) {
    extern __shared__ char s_raw[];

    uint32_t * s_dep = (uint32_t *) s_raw;                             // [8][16][16]
    uint32_t * s_pay = s_dep + 8*256;                                  // [ESCHA_GROUPS][ESCHA_MAX_W]
    float    * s_u   = (float *)(s_pay + ESCHA_GROUPS*ESCHA_MAX_W);    // [R][16]

    // the grid is sized to an upper bound, so the tail blocks have nothing to do.
    // uniform across the block, so the syncs below are still safe
    if (blockIdx.x >= (unsigned) *n_work) {
        return;
    }

    const int4 w = work[blockIdx.x];
    const int e     = w.x;
    const int start = w.y;
    const int nrow  = w.z;

    const int nit  = IC/ESCHA_TILE;
    const int nct  = OC/ESCHA_TILE;
    const int n_wd = (16*K)/2;

    const int tid = threadIdx.x;

    for (int j = tid; j < 8*256; j += ESCHA_NT) {
        const int b2 = j / 256;
        const int p  = j % 256;
        s_dep[j] = (uint32_t) (uint16_t) dep[p*16 + 2*b2]
                 | ((uint32_t) (uint16_t) dep[p*16 + 2*b2 + 1] << 16);
    }

    const int grp = tid / ESCHA_TILE;
    const int cc  = tid % ESCHA_TILE;
    const int tj  = blockIdx.y*ESCHA_GROUPS + grp;

    const int16_t * code_e = code + (int64_t) e*nit*nct*(16*K);
    uint32_t * pay = s_pay + grp*ESCHA_MAX_W;

    float acc[R];
#pragma unroll
    for (int m = 0; m < R; ++m) {
        acc[m] = 0.0f;
    }
    __syncthreads();

    for (int ti = 0; ti < nit; ++ti) {
        // the whole block cooperates on one 16-wide slice of u per row, then every
        // thread reads all of it -- 16 threads of a group hit the same address, so
        // the reads broadcast instead of conflicting
        for (int j = tid; j < R*ESCHA_TILE; j += ESCHA_NT) {
            const int m = j / ESCHA_TILE;
            const int r = j % ESCHA_TILE;
            s_u[j] = m < nrow ? u[(int64_t) ids_dst[start + m]*IC + ti*ESCHA_TILE + r] : 0.0f;
        }

        const uint32_t * src = (const uint32_t *)(code_e + (int64_t)(ti*nct + tj)*(16*K));
        for (int wd = cc; wd < n_wd; wd += ESCHA_TILE) {
            pay[wd] = src[wd];
        }
        __syncthreads();

#pragma unroll 4
        for (int r = 0; r < ESCHA_TILE; ++r) {
            const uint32_t * d = s_dep + r*ESCHA_TILE + cc;

            uint32_t idx = 0;
#pragma unroll
            for (int b2 = 0; b2 < 8; ++b2) {
                const uint32_t dd = d[b2*256];
                const int d0 = dd & 0xffff;
                const int d1 = dd >> 16;

                idx |= ((pay[d0 >> 5] >> (d0 & 31)) & 1u) << (2*b2);
                idx |= ((pay[d1 >> 5] >> (d1 & 31)) & 1u) << (2*b2 + 1);
            }

            const float wv = escha_codebook(idx);
#pragma unroll
            for (int m = 0; m < R; ++m) {
                acc[m] += s_u[m*ESCHA_TILE + r]*wv;
            }
        }
        __syncthreads();
    }

    for (int m = 0; m < nrow; ++m) {
        partial[(int64_t) ids_dst[start + m]*OC + blockIdx.y*ESCHA_NT + tid] = acc[m];
    }
}

// sum the slices, rotate the 128-column group, scale by rout
static __global__ void escha_finalize(
        const half    * __restrict__ rout,
        const int32_t * __restrict__ ids,
        const float   * __restrict__ partial,
        float         * __restrict__ dst,
        const int OC, const int n_ids, const int n_rows, const int n_slices,
        const int64_t nb_i0, const int64_t nb_i1,
        const int64_t nb_d1, const int64_t nb_d2) {
    __shared__ float s_acc[ESCHA_NT];

    const int tid = threadIdx.x;
    const int row = blockIdx.x;
    const int ocb = blockIdx.y;

    const int it = row / n_ids;
    const int is = row % n_ids;

    const int32_t e = *(const int32_t *)((const char *) ids + is*nb_i0 + it*nb_i1);
    const int c = ocb*ESCHA_NT + tid;

    float sum = 0.0f;
    for (int s = 0; s < n_slices; ++s) {
        sum += partial[((int64_t) s*n_rows + row)*OC + c];
    }
    s_acc[tid] = sum;
    __syncthreads();

    escha_hadamard_128(s_acc, ESCHA_NT, tid, ESCHA_NT);

    float * dst_row = (float *)((char *) dst + is*nb_d1 + it*nb_d2);
    dst_row[c] = s_acc[tid]*__half2float(rout[(int64_t) e*OC + c]);
}

void ggml_cuda_op_escha_moe(ggml_backend_cuda_context & ctx, ggml_tensor * dst) {
    const ggml_tensor * code = dst->src[0];
    const ggml_tensor * rin  = dst->src[1];
    const ggml_tensor * rout = dst->src[2];
    const ggml_tensor * lut  = dst->src[3];
    const ggml_tensor * dep  = dst->src[4];
    const ggml_tensor * x    = dst->src[5];
    const ggml_tensor * ids  = dst->src[6];

    GGML_ASSERT(code->type == GGML_TYPE_I16 && dep->type == GGML_TYPE_I16);
    GGML_ASSERT(rin->type == GGML_TYPE_F16 && rout->type == GGML_TYPE_F16 && lut->type == GGML_TYPE_F16);
    GGML_ASSERT(x->type == GGML_TYPE_F32 && ids->type == GGML_TYPE_I32 && dst->type == GGML_TYPE_F32);

    const int K   = code->ne[0]/16;
    const int OC  = code->ne[1]*16;
    const int IC  = code->ne[2]*16;
    const int nit = IC/ESCHA_TILE;

    const int n_expert = code->ne[3];
    const int n_ids    = ids->ne[0];
    const int n_tokens = ids->ne[1];
    const int n_rows   = n_ids*n_tokens;
    const int n_ocb    = OC/ESCHA_NT;

    // reuse only pays once a block can expect a decent share of ESCHA_ROWS rows for
    // its expert. below that the sliced kernel wins, and at batch 1 it is the only
    // one that fills the device at all
    const bool tiled = n_rows >= 8*n_expert;

    cudaStream_t stream = ctx.stream();

    ggml_cuda_pool_alloc<float> u_buf(ctx.pool(), (size_t) n_rows*IC);

    escha_rotate_in<<<n_rows, 256, IC*sizeof(float), stream>>>(
        (const half *) rin->data, (const float *) x->data, (const int32_t *) ids->data,
        u_buf.get(), IC, (int) x->ne[1], n_ids,
        x->nb[1], x->nb[2], ids->nb[0], ids->nb[1]);

    const size_t smem_dep = 8*256*sizeof(uint32_t) + ESCHA_GROUPS*ESCHA_MAX_W*sizeof(uint32_t);

    if (tiled) {
        // mm_ids_helper assumes a token uses an expert at most once, which top-k routing
        // guarantees. duplicates would desync its offsets and write out of bounds
        GGML_ASSERT(ids->nb[0] == ggml_element_size(ids));

        ggml_cuda_pool_alloc<int32_t> ids_src1(ctx.pool(), n_rows);
        ggml_cuda_pool_alloc<int32_t> ids_dst(ctx.pool(), n_rows);
        ggml_cuda_pool_alloc<int32_t> bounds(ctx.pool(), n_expert + 1);

        ggml_cuda_launch_mm_ids_helper((const int32_t *) ids->data, ids_src1.get(), ids_dst.get(), bounds.get(),
            n_expert, n_tokens, n_ids, (int) x->ne[1], (int) (ids->nb[1]/ggml_element_size(ids)), 1,
            /*write_inverse =*/ false, stream);
        CUDA_CHECK(cudaGetLastError());

        // an expert can end with a part-full chunk, so the item count is bounded but not known
        const int n_work_max = (n_rows + ESCHA_ROWS - 1)/ESCHA_ROWS + n_expert;

        ggml_cuda_pool_alloc<int4>    work(ctx.pool(), n_work_max);
        ggml_cuda_pool_alloc<int32_t> n_work(ctx.pool(), 1);

        CUDA_CHECK(cudaMemsetAsync(n_work.get(), 0, sizeof(int32_t), stream));
        escha_build_work<<<1, 256, 0, stream>>>(bounds.get(), work.get(), n_work.get(), n_expert);

        ggml_cuda_pool_alloc<float> p_buf(ctx.pool(), (size_t) n_rows*OC);

        const size_t smem = smem_dep + ESCHA_ROWS*ESCHA_TILE*sizeof(float);

        auto launch = [&](auto kernel) {
            kernel<<<dim3(n_work_max, n_ocb), ESCHA_NT, smem, stream>>>(
                (const int16_t *) code->data, (const half *) lut->data, (const int16_t *) dep->data,
                u_buf.get(), ids_dst.get(), work.get(), n_work.get(), p_buf.get(), IC, OC);
        };

        switch (K) {
            case 2: launch(escha_matmul_tiled<2, ESCHA_ROWS>); break;
            case 3: launch(escha_matmul_tiled<3, ESCHA_ROWS>); break;
            default: GGML_ABORT("escha: unsupported K=%d", K);
        }

        escha_finalize<<<dim3(n_rows, n_ocb), ESCHA_NT, 0, stream>>>(
            (const half *) rout->data, (const int32_t *) ids->data, p_buf.get(), (float *) dst->data,
            OC, n_ids, n_rows, 1, ids->nb[0], ids->nb[1], dst->nb[1], dst->nb[2]);
        return;
    }

    // slice the reduction until the launch is wide enough to fill the device, but only
    // by factors that divide nit evenly
    int n_slices = 1;
    while (n_rows*n_ocb*n_slices*2 <= ESCHA_TARGET && nit % (n_slices*2) == 0) {
        n_slices *= 2;
    }

    ggml_cuda_pool_alloc<float> p_buf(ctx.pool(), (size_t) n_slices*n_rows*OC);

    const dim3 grid(n_rows, n_ocb, n_slices);

    auto launch = [&](auto kernel) {
        kernel<<<grid, ESCHA_NT, smem_dep, stream>>>(
            (const int16_t *) code->data, (const half *) lut->data, (const int16_t *) dep->data,
            u_buf.get(), (const int32_t *) ids->data, p_buf.get(),
            IC, OC, n_ids, n_rows, n_slices, ids->nb[0], ids->nb[1]);
    };

    switch (K) {
        case 2: launch(escha_matmul_partial<2>); break;
        case 3: launch(escha_matmul_partial<3>); break;
        default: GGML_ABORT("escha: unsupported K=%d", K);
    }

    escha_finalize<<<dim3(n_rows, n_ocb), ESCHA_NT, 0, stream>>>(
        (const half *) rout->data, (const int32_t *) ids->data, p_buf.get(), (float *) dst->data,
        OC, n_ids, n_rows, n_slices, ids->nb[0], ids->nb[1], dst->nb[1], dst->nb[2]);
}

// ===========================================================================
// dense escha (ggml_escha_mul_mat)
//
// Same codec and same rotations as the routed path above, minus the routing: one weight
// matrix, every row goes through it. That removes the ids indirection, mm_ids_helper and
// the work list -- a block owns R consecutive rows outright. The IC reduction is still
// sliced across blocks, because at batch 1 a single row would otherwise leave the device
// mostly idle.
// ===========================================================================

// u[row] = T128(x[row] * rin)
//
// Staged through shared memory in fixed chunks rather than all of IC at once: the dense
// projections go up to IC = 17408 (mlp.down), and 17408 floats is 68 KB, well past the
// 48 KB a block gets. The rotation is independent per 128-block, so any chunk that is a
// multiple of 128 splits it exactly.
#define ESCHA_ROT_CHUNK 2048   // 8 KB of shared memory

// U is float for the scalar paths and half for the tensor-core path. Emitting half here
// rather than converting during staging is bit-identical -- the same __float2half, moved
// earlier -- and it is what lets cp.async copy activations straight into shared, since
// cp.async moves bytes verbatim and cannot convert.
template <typename U>
static __global__ void escha_rotate_in_dense(
        const half  * __restrict__ rin,
        const float * __restrict__ x,
        U           * __restrict__ u,
        const int IC, const int ne1,
        const int64_t nb_x1, const int64_t nb_x2) {
    __shared__ float s_u[ESCHA_ROT_CHUNK];

    const int tid = threadIdx.x;
    const int row = blockIdx.x;

    const float * x_row = (const float *)((const char *) x + (int64_t)(row % ne1)*nb_x1
                                                           + (int64_t)(row / ne1)*nb_x2);
    U * dst = u + (int64_t) row*IC;

    for (int off = 0; off < IC; off += ESCHA_ROT_CHUNK) {
        const int n = min(ESCHA_ROT_CHUNK, IC - off);

        for (int i = tid; i < n; i += blockDim.x) {
            s_u[i] = x_row[off + i]*__half2float(rin[off + i]);
        }
        __syncthreads();

        escha_hadamard_128(s_u, n, tid, blockDim.x);

        for (int i = tid; i < n; i += blockDim.x) {
            if constexpr (sizeof(U) == sizeof(half)) {
                dst[off + i] = __float2half(s_u[i]);
            } else {
                dst[off + i] = s_u[i];
            }
        }
        __syncthreads();
    }
}

// escha's dep table is computable, so the dense kernel never reads it.
//
// The 16 bits that form a weight's codebook index are always 16 cyclically-consecutive
// positions of a bit-stream over the tile payload, where the stream visits 32-bit word 0
// first and then walks the words downwards (0, NW-1, NW-2, ...). Only the start position
// varies, and it is affine in the tile row and column:
//
//   pi(r) = (r&1) | ((r>>3)&1)<<1 | ((r>>1)&3)<<3      // bit 2 is left free for c
//   t     = pi(r) + 32*c + 4*(c>>3)
//   s     = ((32-K) - K*t) mod 256K
//
// Verified exact against both shipped tables, all 4096 entries (dep3.py). This turns
// 8 shared dep reads + 16 payload reads + ~48 bit ops per weight into two reads and a
// funnel shift, and drops the 8 KB per-block dep table that made batch 1 setup-bound.
__device__ __forceinline__ int escha_dep_pi(int r) {
    return (r & 1) | (((r >> 3) & 1) << 1) | (((r >> 1) & 3) << 3);
}

// Register-tiled variant of the dense kernel, for prefill.
//
// The column-per-thread kernel below couples decode reuse to one thread's registers: it
// accumulates R rows in acc[R], so reusing a decode more means more registers in the SAME
// thread. At R=64 that is 255 registers with spill, 2 blocks/SM, ~17% occupancy -- and it
// is why prefill decodes at ~60 G/s while the batch-1 path manages ~372 G/s.
//
// Here the decoded weights go to SHARED memory instead, so every row group in the block
// reuses them. Reuse becomes BM (rows per BLOCK) while registers stay TM*TN (the thread's
// own output tile), which decouples the two:
//
//     BM * BN = (TM * TN) * NT
//
// BN is held at 128 on purpose: total activation traffic is rows * IC * (OC/BN), so
// narrowing BN to buy reuse would multiply global reads instead (that mistake was caught
// on paper, not in silicon -- BN=16 would have cost 8x the activation bandwidth).
//
// TN must be > 1. With one column per thread every shared read feeds exactly one MAC and
// shared bandwidth becomes the new ceiling; a TMxTN tile does TM*TN MACs per TM+TN reads.
template <int K, int BM, int BN, int TM, int TN>
static __global__ void escha_matmul_dense_tiled(
        const int16_t * __restrict__ code,
        const half    * __restrict__ lut,
        const int16_t * __restrict__ dep,
        const float   * __restrict__ u,
        float         * __restrict__ partial,
        const int IC, const int OC, const int n_rows, const int n_slices) {
    constexpr int NT  = (BM/TM)*(BN/TN);   // threads per block
    constexpr int NTJ = BN/ESCHA_TILE;     // output tiles covered
    constexpr int NCX = BN/TN;             // threads across the column axis

    extern __shared__ char s_raw[];
    uint32_t * s_pay = (uint32_t *) s_raw;                          // [NTJ][ESCHA_MAX_W]
    float    * s_w   = (float *)(s_pay + NTJ*ESCHA_MAX_W);          // [16][BN]
    float    * s_u   = s_w + ESCHA_TILE*BN;                         // [2][16][BM] transposed

    GGML_UNUSED(lut);
    GGML_UNUSED(dep);

    const int NW = 8*K;
    const int NB = 32*NW;

    const int nit  = IC/ESCHA_TILE;
    const int nct  = OC/ESCHA_TILE;
    const int n_wd = (16*K)/2;

    const int tid  = threadIdx.x;
    const int cx   = tid % NCX;            // this thread's column strip
    const int ry   = tid / NCX;            // this thread's row strip
    const int row0 = blockIdx.x*BM;
    const int oc0  = blockIdx.y*BN;

    const int sl = blockIdx.z;
    const int lo = (int) (((int64_t) nit*sl)/n_slices);
    const int hi = (int) (((int64_t) nit*(sl + 1))/n_slices);

    float acc[TM*TN];
#pragma unroll
    for (int i = 0; i < TM*TN; ++i) {
        acc[i] = 0.0f;
    }

    // this thread's payload slot, fixed for the whole loop. One word per thread only:
    static_assert(NTJ*((16*K)/2) <= NT, "escha: payload needs more than one word per thread");
    const bool has_pay = tid < NTJ*n_wd;
    const int  pt = tid/n_wd, pw = tid % n_wd;
    uint32_t   ppre = 0;
    if (has_pay && lo < hi) {
        ppre = ((const uint32_t *)(code + (int64_t)(lo*nct + oc0/ESCHA_TILE + pt)*(16*K)))[pw];
    }

    // stage tile lo's activations into buffer 0 before the loop, so that inside the loop
    // the fetch for ti+1 can be issued into the OTHER buffer and overlap this tile's work
    if (lo < hi) {
        for (int j = tid; j < BM*ESCHA_TILE; j += NT) {
            const int m = j / ESCHA_TILE, r = j % ESCHA_TILE;
            const int row = row0 + m;
            s_u[r*BM + m] = row < n_rows ? u[(int64_t) row*IC + lo*ESCHA_TILE + r] : 0.0f;
        }
    }

    for (int ti = lo; ti < hi; ++ti) {
        float * su_cur = s_u + (((ti - lo) & 1)      )*(ESCHA_TILE*BM);
        float * su_nxt = s_u + (((ti - lo) & 1) ^ 1  )*(ESCHA_TILE*BM);
        // publish the payload fetched last round, then issue the next fetch before the
        // barrier, so the global latency overlaps the decode instead of stalling every warp
        if (has_pay) {
            s_pay[pt*ESCHA_MAX_W + pw] = ppre;
        }
        if (has_pay && ti + 1 < hi) {
            ppre = ((const uint32_t *)(code + (int64_t)((ti + 1)*nct + oc0/ESCHA_TILE + pt)*(16*K)))[pw];
        }
        __syncthreads();

        // next tile's activations go to the other buffer: no barrier separates them from
        // the reads of su_cur below, and the barrier at the top of the next iteration is
        // what makes them visible
        if (ti + 1 < hi) {
            for (int j = tid; j < BM*ESCHA_TILE; j += NT) {
                const int m = j / ESCHA_TILE, r = j % ESCHA_TILE;
                const int row = row0 + m;
                su_nxt[r*BM + m] = row < n_rows ? u[(int64_t) row*IC + (ti + 1)*ESCHA_TILE + r] : 0.0f;
            }
        }

        // decode this input tile's 16 x BN weights once, for the whole block
        for (int j = tid; j < ESCHA_TILE*BN; j += NT) {
            const int r = j / BN, c = j % BN;
            const uint32_t * pay = s_pay + (c/ESCHA_TILE)*ESCHA_MAX_W;
            const int ccl = c % ESCHA_TILE;

            int sp = ((32 - K) - K*(escha_dep_pi(r) + 32*ccl + 4*(ccl >> 3))) % NB;
            if (sp < 0) {
                sp += NB;
            }
            const int g0 = sp >> 5;
            const int w0 = g0 ? (NW - g0) : 0;
            const int w1 = w0 ? (w0 - 1)  : (NW - 1);

            s_w[r*BN + c] = escha_codebook(__funnelshift_r(pay[w0], pay[w1], sp & 31) & 0xffffu);
        }
        __syncthreads();

#pragma unroll
        for (int r = 0; r < ESCHA_TILE; ++r) {
            float a[TM], b[TN];
#pragma unroll
            for (int m = 0; m < TM; ++m) {
                a[m] = su_cur[r*BM + ry*TM + m];
            }
#pragma unroll
            for (int n = 0; n < TN; ++n) {
                b[n] = s_w[r*BN + cx*TN + n];
            }
#pragma unroll
            for (int m = 0; m < TM; ++m) {
#pragma unroll
                for (int n = 0; n < TN; ++n) {
                    acc[m*TN + n] += a[m]*b[n];
                }
            }
        }
        __syncthreads();
    }

#pragma unroll
    for (int m = 0; m < TM; ++m) {
        const int row = row0 + ry*TM + m;
        if (row < n_rows) {
#pragma unroll
            for (int n = 0; n < TN; ++n) {
                partial[((int64_t) sl*n_rows + row)*OC + oc0 + cx*TN + n] = acc[m*TN + n];
            }
        }
    }
}


// Prefill on tensor cores. Same decode as escha_matmul_dense_tiled, but the GEMM runs on
// m16n8k16 HMMA with fp32 accumulate, which on GA102 is 71 TFLOPS against 35.6 for FP32 FMA.
//
// Two layout changes fall out of the fragment shapes, and both are cheap:
//   s_u becomes [m][k] (was [k][m]) -- which makes staging a straight contiguous copy
//   s_w becomes [n][k] (was [k][n]) -- B is the ".col" operand of mma.row.col
//
// The weights stay exact (escha_codebook_h). The ACTIVATIONS are rounded to fp16, which is
// what escha's own runtime does, and costs rel_rms ~2.1e-4 against the fp32 reference.
template <int K, int BM, int BN>
// minBlocks=2: hide reconstruct latency; BM=128 acc[2][8] should still fit.
static __global__ void __launch_bounds__(256, 2) escha_matmul_dense_tiled_mma(
        const int16_t * __restrict__ code,
        const half    * __restrict__ lut,
        const int16_t * __restrict__ dep,
        const half    * __restrict__ u,
        float         * __restrict__ partial,
        const int IC, const int OC, const int n_rows, const int n_slices) {
#ifdef TURING_MMA_AVAILABLE
    constexpr int NT   = 256;
    constexpr int NW   = NT/32;          // warps
    constexpr int WN   = 2;              // warps across the column axis
    constexpr int WM   = NW/WN;          // warps down the row axis
    constexpr int MT   = BM/16/WM;       // 16-row accumulator tiles per warp
    constexpr int NTT  = BN/8/WN;        // 8-col accumulator tiles per warp
    constexpr int NTJ  = BN/ESCHA_TILE;  // output tiles whose payload this block holds

    extern __shared__ char s_raw[];
    uint2    * s_pay = (uint2 *) s_raw;                               // [NTJ][ESCHA_MAX_W] pairs
    half     * s_u   = (half *)(s_pay + NTJ*ESCHA_MAX_W);             // [2][BM][16]
    half     * s_w   = s_u + 2*BM*ESCHA_TILE;                         // [BN][16]

    GGML_UNUSED(lut);
    GGML_UNUSED(dep);

    const int NWD = 8*K;
    const int NB  = 32*NWD;

    const int nit  = IC/ESCHA_TILE;
    const int nct  = OC/ESCHA_TILE;
    const int n_wd = (16*K)/2;

    const int lane = threadIdx.x;          // must stay the lane: mma.cuh indexes on it
    const int warp = threadIdx.y;
    const int tid  = warp*32 + lane;        // flat id, for the layout-agnostic staging loops
    const int row0 = blockIdx.x*BM;
    const int oc0  = blockIdx.y*BN;

    const int sl = blockIdx.z;
    const int lo = (int) (((int64_t) nit*sl)/n_slices);
    const int hi = (int) (((int64_t) nit*(sl + 1))/n_slices);

    const int wm   = warp / WN;
    const int wn   = warp % WN;

    constexpr int DPT = (ESCHA_TILE*BN)/NT;   // weights this thread decodes per tile
    static_assert(NT % ESCHA_TILE == 0,        "escha: r would not be thread-invariant");
    static_assert((ESCHA_TILE*BN) % NT == 0,   "escha: ragged decode assignment");
    static_assert(NT/ESCHA_TILE <= ESCHA_TILE, "escha: ccl would not be thread-invariant");

    // cp.async moves 16 bytes = 8 halves per thread; BM*ESCHA_TILE halves is exactly
    // NT*8 at BM=128/NT=256, so every thread issues one copy and none loops.
    constexpr int CPB = 16;                              // bytes per thread per tile
    static_assert(BM*ESCHA_TILE*sizeof(half) == NT*CPB,  "escha: activation copy is ragged");
    const int cp_m  = tid / (ESCHA_TILE*sizeof(half)/CPB);   // row this thread copies into
    const int cp_h  = (tid % (ESCHA_TILE*sizeof(half)/CPB))*(CPB/sizeof(half));

    const int dr   = tid % ESCHA_TILE;         // this thread's r, every k, every tile
    const int dccl = tid / ESCHA_TILE;         // and its column within the 16-wide tile
    int dsp = ((32 - K) - K*(escha_dep_pi(dr) + 32*dccl + 4*(dccl >> 3))) % NB;
    if (dsp < 0) {
        dsp += NB;
    }
    const int dg0 = dsp >> 5;
    const int dw0 = dg0 ? (NWD - dg0) : 0;
    const int dw1 = dw0 ? (dw0 - 1)   : (NWD - 1);
    const int dsh = dsp & 31;

    typedef ggml_cuda_mma::tile<16, 8, float> tile_c;
    typedef ggml_cuda_mma::tile<16, 8, half2> tile_a;
    typedef ggml_cuda_mma::tile<8,  8, half2> tile_b;

    tile_c acc[MT][NTT];

    // one payload word per thread, held a tile ahead
    static_assert(NTJ*((16*K)/2) <= NT, "escha: payload needs more than one word per thread");
    const bool has_pay = tid < NTJ*n_wd;
    const int  pt = tid/n_wd, pw = tid % n_wd;
    uint32_t   ppre = 0;
    if (has_pay && lo < hi) {
        ppre = ((const uint32_t *)(code + (int64_t)(lo*nct + oc0/ESCHA_TILE + pt)*(16*K)))[pw];
    }

    // activations for tile lo into buffer 0
    if (lo < hi) {
        {
            const int row = row0 + cp_m;
            const int src_row = row < n_rows ? row : 0;
            __pipeline_memcpy_async(s_u + cp_m*ESCHA_TILE + cp_h,
                                    u + (int64_t) src_row*IC + lo*ESCHA_TILE + cp_h,
                                    CPB, row < n_rows ? 0 : CPB);
        }
        __pipeline_commit();
    }

    for (int ti = lo; ti < hi; ++ti) {
        half * su_cur = s_u + (((ti - lo) & 1)     )*(BM*ESCHA_TILE);
        half * su_nxt = s_u + (((ti - lo) & 1) ^ 1 )*(BM*ESCHA_TILE);

        if (has_pay) {
            // word pw is the high half of pair pw and the low half of pair pw+1
            s_pay[pt*ESCHA_MAX_W + pw].y = ppre;
            s_pay[pt*ESCHA_MAX_W + (pw + 1 == NWD ? 0 : pw + 1)].x = ppre;
        }
        if (has_pay && ti + 1 < hi) {
            ppre = ((const uint32_t *)(code + (int64_t)((ti + 1)*nct + oc0/ESCHA_TILE + pt)*(16*K)))[pw];
        }
        // the copy for THIS tile was committed last round; drain it before the barrier
        // that publishes s_pay, so su_cur is visible to every warp below
        __pipeline_wait_prior(0);
        __syncthreads();

        if (ti + 1 < hi) {
            const int row = row0 + cp_m;
            const int src_row = row < n_rows ? row : 0;
            __pipeline_memcpy_async(su_nxt + cp_m*ESCHA_TILE + cp_h,
                                    u + (int64_t) src_row*IC + (ti + 1)*ESCHA_TILE + cp_h,
                                    CPB, row < n_rows ? 0 : CPB);
            __pipeline_commit();
        }

        // decode into [n][k]. All the addressing is precomputed above; iteration k reads
        // payload tile k and writes column dccl + 16k, so this is just load, shift, codebook.
#pragma unroll
        for (int k = 0; k < DPT; ++k) {
            const uint2 * pay = s_pay + k*ESCHA_MAX_W;
            const int c = dccl + ESCHA_TILE*k;
            s_w[c*ESCHA_TILE + dr] =
                escha_codebook_h(__funnelshift_r(pay[dw0].y, pay[dw0].x, dsh) & 0xffffu);
        }
        __syncthreads();

        {
            const half2 * su2 = (const half2 *) su_cur;   // [BM][8] half2
            const half2 * sw2 = (const half2 *) s_w;      // [BN][8] half2

            tile_a A[MT];
            tile_b B[NTT];
#pragma unroll
            for (int i = 0; i < MT; ++i) {
                ggml_cuda_mma::load_ldmatrix(A[i], su2 + (size_t)(wm*(16*MT) + i*16)*8, 8);
            }
#pragma unroll
            for (int j = 0; j < NTT; ++j) {
                ggml_cuda_mma::load_ldmatrix(B[j], sw2 + (size_t)(wn*(8*NTT) + j*8)*8, 8);
            }
#pragma unroll
            for (int i = 0; i < MT; ++i) {
#pragma unroll
                for (int j = 0; j < NTT; ++j) {
                    ggml_cuda_mma::mma(acc[i][j], A[i], B[j]);
                }
            }
        }
        __syncthreads();
    }

#pragma unroll
    for (int i = 0; i < MT; ++i) {
#pragma unroll
        for (int j = 0; j < NTT; ++j) {
#pragma unroll
            for (int l = 0; l < tile_c::ne; ++l) {
                const int m   = wm*(16*MT) + i*16 + tile_c::get_i(l);
                const int n   = wn*(8*NTT) + j*8  + tile_c::get_j(l);
                const int row = row0 + m;
                if (row < n_rows) {
                    partial[((int64_t) sl*n_rows + row)*OC + oc0 + n] = acc[i][j].x[l];
                }
            }
        }
    }
#else
    GGML_UNUSED_VARS(code, lut, dep, u, partial, IC, OC, n_rows, n_slices);
    NO_DEVICE_CODE;
#endif // TURING_MMA_AVAILABLE
}

// partial[slice][row][c] = sum over this slice's input tiles of u . decode(code)
template <int K, int R>
static __global__ void escha_matmul_dense(
        const int16_t * __restrict__ code,
        const half    * __restrict__ lut,
        const int16_t * __restrict__ dep,
        const float   * __restrict__ u,
        float         * __restrict__ partial,
        const int IC, const int OC, const int n_rows, const int n_slices) {
    extern __shared__ char s_raw[];

    // every decode wants the adjacent pair (pay[w0-1], pay[w0]), so hold the payload AS
    // overlapping pairs: one aligned LDS.64 then replaces the two LDS the funnel shift needed
    uint2 * s_pay = (uint2 *) s_raw;                                   // [ESCHA_GROUPS][ESCHA_MAX_W]
    float * s_u   = (float *)(s_pay + ESCHA_GROUPS*ESCHA_MAX_W);       // [R][16]

    GGML_UNUSED(lut);
    GGML_UNUSED(dep);

    const int NW = 8*K;          // 32-bit words in a tile payload
    const int NB = 32*NW;        // bits

    const int nit  = IC/ESCHA_TILE;
    const int nct  = OC/ESCHA_TILE;
    const int n_wd = (16*K)/2;

    const int tid   = threadIdx.x;
    const int start = blockIdx.x*R;
    const int nrow  = min(R, n_rows - start);

    // this block's share of the input tiles
    const int sl  = blockIdx.z;
    const int lo  = (int) (((int64_t) nit*sl)/n_slices);
    const int hi  = (int) (((int64_t) nit*(sl + 1))/n_slices);

    const int grp = tid / ESCHA_TILE;
    const int cc  = tid % ESCHA_TILE;
    const int tj  = blockIdx.y*ESCHA_GROUPS + grp;

    uint2 * pay = s_pay + grp*ESCHA_MAX_W;

    // start position for this thread's column, before the per-row term
    int s0 = ((32 - K) - K*(32*cc + 4*(cc >> 3))) % NB;
    if (s0 < 0) {
        s0 += NB;
    }

    float acc[R];
#pragma unroll
    for (int m = 0; m < R; ++m) {
        acc[m] = 0.0f;
    }
    // At R == 1 the block reads one row, so its entire slice of u fits in shared and is
    // staged once here -- the per-tile staging below then disappears, and with it the
    // block-wide barrier that ordered it. This is what their 10,240-byte allocation is.
    if constexpr (R == 1) {
        const float * u_row = u + (int64_t) start*IC + (int64_t) lo*ESCHA_TILE;
        const int n_stage = (hi - lo)*ESCHA_TILE;
        for (int j = tid; j < n_stage; j += ESCHA_NT) {
            s_u[j] = u_row[j];
        }
    }
    // payload words this thread owns, and the tile fetched one iteration ahead
    constexpr int NPW = (8*K + ESCHA_TILE - 1)/ESCHA_TILE;
    uint32_t pre[NPW];
    if (lo < hi) {
        const uint32_t * s0p = (const uint32_t *)(code + (int64_t)(lo*nct + tj)*(16*K));
#pragma unroll
        for (int i = 0; i < NPW; ++i) {
            const int wd = cc + i*ESCHA_TILE;
            if (wd < n_wd) {
                pre[i] = s0p[wd];
            }
        }
    }
    __syncthreads();

    for (int ti = lo; ti < hi; ++ti) {
        // as in the routed kernel: the block cooperates on one 16-wide slice of u per row,
        // and the 16 threads of a group then read the same entry, so it broadcasts
        if constexpr (R != 1) {
            for (int j = tid; j < R*ESCHA_TILE; j += ESCHA_NT) {
                const int m = j / ESCHA_TILE;
                const int r = j % ESCHA_TILE;
                s_u[j] = m < nrow ? u[(int64_t)(start + m)*IC + ti*ESCHA_TILE + r] : 0.0f;
            }
        }

        // publish the tile fetched last round, then issue the next fetch immediately: the
        // LDG then overlaps this tile's decode instead of stalling in front of it, which is
        // what the LDG -> dependent STS pair at the top of the loop was costing
#pragma unroll
        for (int i = 0; i < NPW; ++i) {
            const int wd = cc + i*ESCHA_TILE;
            if (wd < n_wd) {
                // word wd is the high half of pair wd and the low half of pair wd+1
                pay[wd].y = pre[i];
                pay[wd + 1 == NW ? 0 : wd + 1].x = pre[i];
            }
        }
        if (ti + 1 < hi) {
            const uint32_t * nxt = (const uint32_t *)(code + (int64_t)((ti + 1)*nct + tj)*(16*K));
#pragma unroll
            for (int i = 0; i < NPW; ++i) {
                const int wd = cc + i*ESCHA_TILE;
                if (wd < n_wd) {
                    pre[i] = nxt[wd];
                }
            }
        }
        if constexpr (R == 1) { __syncwarp(); } else { __syncthreads(); }

        const float * uu = s_u + (ti - lo)*ESCHA_TILE;

        // generation unrolls fully so pi(r) folds to a compile-time constant; prefill keeps
        // a partial unroll, where acc[R] already claims the registers
#pragma unroll (R <= 8 ? 16 : 4)
        for (int r = 0; r < ESCHA_TILE; ++r) {
            // K*pi(r) <= 81 < NB, so one conditional add restores the range
            int sp = s0 - K*escha_dep_pi(r);
            if (sp < 0) {
                sp += NB;
            }

            const int g0 = sp >> 5;
            const int w0 = g0 ? (NW - g0) : 0;
            const int w1 = w0 ? (w0 - 1)  : (NW - 1);

            const uint2 p = pay[w0];
            GGML_UNUSED(w1);
            const uint32_t idx = __funnelshift_r(p.y, p.x, sp & 31) & 0xffffu;

            const float wv = escha_codebook(idx);
            if constexpr (R == 1) {
                acc[0] += uu[r]*wv;
            } else {
#pragma unroll
                for (int m = 0; m < R; ++m) {
                    acc[m] += s_u[m*ESCHA_TILE + r]*wv;
                }
            }
        }
        if constexpr (R == 1) { __syncwarp(); } else { __syncthreads(); }
    }

    for (int m = 0; m < nrow; ++m) {
        partial[((int64_t) sl*n_rows + start + m)*OC + blockIdx.y*ESCHA_NT + tid] = acc[m];
    }
}

// sum the slices in a fixed order (so the result is reproducible), rotate the
// 128-column group, scale by rout
// partial and dst can be the same memory (recon path, 1 slice): a block reads its 128 values before it writes them
static __global__ void escha_finalize_dense(
        const half  * __restrict__ rout,
        const float * partial,
        float       * dst,
        const int OC, const int ne1, const int n_rows, const int n_slices,
        const int64_t nb_d1, const int64_t nb_d2) {
    __shared__ float s_acc[ESCHA_NT];

    const int tid = threadIdx.x;
    const int row = blockIdx.x;
    const int c   = blockIdx.y*ESCHA_NT + tid;

    float sum = 0.0f;
    for (int s = 0; s < n_slices; ++s) {
        sum += partial[((int64_t) s*n_rows + row)*OC + c];
    }
    s_acc[tid] = sum;
    __syncthreads();

    escha_hadamard_128(s_acc, ESCHA_NT, tid, ESCHA_NT);

    float * dst_row = (float *)((char *) dst + (int64_t)(row % ne1)*nb_d1
                                             + (int64_t)(row / ne1)*nb_d2);
    dst_row[c] = s_acc[tid]*__half2float(rout[c]);
}

// Prefill by reconstruction (the exllamav3 route for large batches): decode the projection to dense fp16 once for a micro-batch, then one tensor-core GEMM.
// The tiled MMA kernel above decodes the weights again for each 128-row block.

// w[c][k] = decode(code) for the output columns [oc0, oc0 + gridDim.x*128), one row of IC values for each column.
// A block has 8 output tiles and a slice of the input tiles. A thread has one column: it decodes the 16 weights of that column in a tile and writes them as one contiguous run.
// W = half: the exact weights. W = int8_t: round(w*ESCHA_I8_WSCALE), for the int8 path (the codebook is inside +-4).
template <int K, typename W>
static __global__ void escha_reconstruct_dense(
        const int16_t * __restrict__ code,
        W             * __restrict__ w,
        const int IC, const int OC, const int oc0, const int n_slices) {
    __shared__ uint2 s_pay[ESCHA_GROUPS*ESCHA_MAX_W];

    const int NW = 8*K;
    const int NB = 32*NW;

    const int nit  = IC/ESCHA_TILE;
    const int nct  = OC/ESCHA_TILE;
    const int n_wd = (16*K)/2;

    const int tid = threadIdx.x;
    const int grp = tid / ESCHA_TILE;
    const int cc  = tid % ESCHA_TILE;
    const int tj  = oc0/ESCHA_TILE + blockIdx.x*ESCHA_GROUPS + grp;

    const int sl = blockIdx.y;
    const int lo = (int) (((int64_t) nit*sl)/n_slices);
    const int hi = (int) (((int64_t) nit*(sl + 1))/n_slices);

    uint2 * pay = s_pay + grp*ESCHA_MAX_W;

    int s0 = ((32 - K) - K*(32*cc + 4*(cc >> 3))) % NB;
    if (s0 < 0) {
        s0 += NB;
    }

    // payload words of this thread, fetched one tile ahead (as in escha_matmul_dense)
    constexpr int NPW = (8*K + ESCHA_TILE - 1)/ESCHA_TILE;
    uint32_t pre[NPW];
    if (lo < hi) {
        const uint32_t * s0p = (const uint32_t *)(code + (int64_t)(lo*nct + tj)*(16*K));
#pragma unroll
        for (int i = 0; i < NPW; ++i) {
            const int wd = cc + i*ESCHA_TILE;
            if (wd < n_wd) {
                pre[i] = s0p[wd];
            }
        }
    }

    uint4 * w_col = (uint4 *)(w + (int64_t)(blockIdx.x*ESCHA_NT + tid)*IC);

    for (int ti = lo; ti < hi; ++ti) {
#pragma unroll
        for (int i = 0; i < NPW; ++i) {
            const int wd = cc + i*ESCHA_TILE;
            if (wd < n_wd) {
                pay[wd].y = pre[i];
                pay[wd + 1 == NW ? 0 : wd + 1].x = pre[i];
            }
        }
        if (ti + 1 < hi) {
            const uint32_t * nxt = (const uint32_t *)(code + (int64_t)((ti + 1)*nct + tj)*(16*K));
#pragma unroll
            for (int i = 0; i < NPW; ++i) {
                const int wd = cc + i*ESCHA_TILE;
                if (wd < n_wd) {
                    pre[i] = nxt[wd];
                }
            }
        }
        // the 16 threads of a column group share one payload, and a warp has two whole groups
        __syncwarp();

        uint32_t v[ESCHA_TILE/2] = {};
#pragma unroll
        for (int r = 0; r < ESCHA_TILE; ++r) {
            int sp = s0 - K*escha_dep_pi(r);
            if (sp < 0) {
                sp += NB;
            }
            const int g0 = sp >> 5;
            const int w0 = g0 ? (NW - g0) : 0;

            const uint2 p = pay[w0];
            const half  h = escha_codebook_h(__funnelshift_r(p.y, p.x, sp & 31) & 0xffffu);
            if constexpr (sizeof(W) == sizeof(half)) {
                v[r/2] |= (uint32_t) __half_as_ushort(h) << (16*(r & 1));
            } else {
                const int q = max(-127, min(127, __float2int_rn(__half2float(h)*ESCHA_I8_WSCALE)));
                v[r/4] |= (uint32_t) (q & 0xff) << (8*(r & 3));
            }
        }
        if constexpr (sizeof(W) == sizeof(half)) {
            w_col[2*ti]     = make_uint4(v[0], v[1], v[2], v[3]);
            w_col[2*ti + 1] = make_uint4(v[4], v[5], v[6], v[7]);
        } else {
            w_col[ti] = make_uint4(v[0], v[1], v[2], v[3]);
        }
        __syncwarp();
    }
}

// q[row] = int8(u[row]*127/amax(row)), scales[row] = amax(row)/127/ESCHA_I8_WSCALE: acc*scales[row] is then the fp32 product
static __global__ void escha_quantize_rows_i8(
        const half * __restrict__ u,
        int8_t     * __restrict__ q,
        float      * __restrict__ scales,
        const int IC) {
    __shared__ float s_max[8];

    const int tid = threadIdx.x;
    const int row = blockIdx.x;

    const half2 * u_row = (const half2 *)(u + (int64_t) row*IC);

    float amax = 0.0f;
    for (int i = tid; i < IC/2; i += blockDim.x) {
        const float2 f = __half22float2(u_row[i]);
        amax = fmaxf(amax, fmaxf(fabsf(f.x), fabsf(f.y)));
    }
    amax = warp_reduce_max(amax);
    if (tid % 32 == 0) {
        s_max[tid/32] = amax;
    }
    __syncthreads();
#pragma unroll
    for (int i = 0; i < 8; ++i) {
        amax = fmaxf(amax, s_max[i]);
    }

    const float inv = amax > 0.0f ? 127.0f/amax : 0.0f;
    uint32_t * q_row = (uint32_t *)(q + (int64_t) row*IC);
    for (int i = tid; i < IC/4; i += blockDim.x) {
        const float2 a = __half22float2(u_row[2*i]);
        const float2 b = __half22float2(u_row[2*i + 1]);
        const int q0 = max(-127, min(127, __float2int_rn(a.x*inv)));
        const int q1 = max(-127, min(127, __float2int_rn(a.y*inv)));
        const int q2 = max(-127, min(127, __float2int_rn(b.x*inv)));
        const int q3 = max(-127, min(127, __float2int_rn(b.y*inv)));
        q_row[i] = (uint32_t) (q0 & 0xff) | (uint32_t) (q1 & 0xff) << 8 | (uint32_t) (q2 & 0xff) << 16 | (uint32_t) (q3 & 0xff) << 24;
    }
    if (tid == 0) {
        scales[row] = amax/(127.0f*ESCHA_I8_WSCALE);
    }
}

// GGML_ESCHA_PREFILL=recon|mma|i8: the prefill path for rows >= GGML_ESCHA_RECON_MIN_ROWS
//   recon: exact read. Exact weights in fp16, fp16 activations, fp32 sum.
//   mma:   exact read with the tiled kernel above (same numbers as recon).
//   i8:    fast read (int8), approximate. int8 weights round(w*32) and int8 activations (one scale for each row), int32 sum. 3.5x the GEMM rate of fp16 on an RTX 4070, 1.1 to 1.2 % error for each projection (exact read: 0.02 %).
// With i8 a token that is read in a batch of GGML_ESCHA_RECON_MIN_ROWS rows or more and the same token read alone (exact kernels) do not give the same output, so GGML_CUDA_BATCH_INVARIANT does not cover prefill against decode in this mode.
// Drafting on / off stays identical: the two read the prompt the same way, and the int8 result of a row does not change with the other rows of the batch (integer sums, one scale for each row).
enum escha_prefill_mode { ESCHA_PREFILL_MMA, ESCHA_PREFILL_RECON, ESCHA_PREFILL_I8 };

static escha_prefill_mode escha_prefill() {
    static const escha_prefill_mode mode = [] {
        const char * e = getenv("GGML_ESCHA_PREFILL");
        if (e == nullptr) {
            return ESCHA_PREFILL_DEFAULT;
        }
        return strcmp(e, "i8") == 0 ? ESCHA_PREFILL_I8 : strcmp(e, "recon") == 0 ? ESCHA_PREFILL_RECON : ESCHA_PREFILL_MMA;
    }();
    return mode;
}

static int escha_recon_min_rows() {
    static const int rows = [] {
        const char * e = getenv("GGML_ESCHA_RECON_MIN_ROWS");
        return e != nullptr ? MAX(ESCHA_GEN_MAX_ROWS + 1, atoi(e)) : ESCHA_RECON_MIN_ROWS;
    }();
    return rows;
}

// GGML_ESCHA_I8_MIN_ROWS=n: the row threshold of the int8 read (default 49: all batches that the small-batch GEMV chunks do not take).
// Its decode writes half the bytes of the fp16 decode and its GEMM is 3.5x faster, so it is faster than one block of the tiled kernel from the first row.
static int escha_i8_min_rows() {
    static const int rows = [] {
        const char * e = getenv("GGML_ESCHA_I8_MIN_ROWS");
        return MAX(ESCHA_GEN_MAX_ROWS + 1, e != nullptr ? atoi(e) : ESCHA_I8_MIN_ROWS);
    }();
    return rows;
}

// GGML_ESCHA_RECON_MIB=n: limit of the dense weight scratch. A larger projection is done in column chunks, one decode and one GEMM for each.
static size_t escha_recon_max_bytes() {
    static const size_t bytes = [] {
        const char * e = getenv("GGML_ESCHA_RECON_MIB");
        return (size_t) MAX(1, e != nullptr ? atoi(e) : ESCHA_RECON_MIB) << 20;
    }();
    return bytes;
}

// partial[row][c] = u[row] . decode(code)[c]
//   q == nullptr: u in fp16, exact weights, fp32 sum
//   q != nullptr: the int8 rows of escha_quantize_rows_i8, int8 weights, int32 sum (partial has int32 values)
static void escha_mul_mat_recon(
        ggml_backend_cuda_context & ctx, const int16_t * code, const half * u, const int8_t * q, float * partial,
        const int K, const int IC, const int OC, const int n_rows) {
    cudaStream_t stream = ctx.stream();

    const bool   i8 = q != nullptr;
    const size_t ws = i8 ? sizeof(int8_t) : sizeof(half);

    // equal chunks of whole 128-column groups
    const int64_t fit     = MAX((int64_t) 1, (int64_t) (escha_recon_max_bytes()/((size_t) IC*ws))/ESCHA_NT);
    const int64_t n_grp   = OC/ESCHA_NT;
    const int64_t n_chunk = (n_grp + fit - 1)/fit;
    const int     oc_step = (int) ((n_grp + n_chunk - 1)/n_chunk)*ESCHA_NT;

    ggml_cuda_pool_alloc<char> w_buf(ctx.pool(), (size_t) oc_step*IC*ws);

    // about 32 input tiles for each block
    const int n_slices = MAX(1, IC/ESCHA_TILE/32);

    cublasHandle_t handle = ctx.cublas_handle();

    for (int oc0 = 0; oc0 < OC; oc0 += oc_step) {
        const int n_oc = MIN(oc_step, OC - oc0);

        const dim3 grid(n_oc/ESCHA_NT, n_slices);
        switch (2*K + (i8 ? 1 : 0)) {
            case 4: escha_reconstruct_dense<2, half>  <<<grid, ESCHA_NT, 0, stream>>>(code, (half *)   w_buf.get(), IC, OC, oc0, n_slices); break;
            case 5: escha_reconstruct_dense<2, int8_t><<<grid, ESCHA_NT, 0, stream>>>(code, (int8_t *) w_buf.get(), IC, OC, oc0, n_slices); break;
            case 6: escha_reconstruct_dense<3, half>  <<<grid, ESCHA_NT, 0, stream>>>(code, (half *)   w_buf.get(), IC, OC, oc0, n_slices); break;
            case 7: escha_reconstruct_dense<3, int8_t><<<grid, ESCHA_NT, 0, stream>>>(code, (int8_t *) w_buf.get(), IC, OC, oc0, n_slices); break;
            default: GGML_ABORT("escha: unsupported K=%d", K);
        }
        CUDA_CHECK(cudaGetLastError());

        // column-major: C (n_oc x n_rows, in partial with ld OC) = W^T (n_oc x IC) * U (IC x n_rows)
        if (i8) {
            const int alpha = 1;
            const int beta  = 0;
            CUBLAS_CHECK(cublasGemmEx(handle, CUBLAS_OP_T, CUBLAS_OP_N, n_oc, n_rows, IC,
                &alpha, w_buf.get(),   CUDA_R_8I,  IC,
                        q,             CUDA_R_8I,  IC,
                &beta,  partial + oc0, CUDA_R_32I, OC,
                CUBLAS_COMPUTE_32I, CUBLAS_GEMM_DEFAULT_TENSOR_OP));
        } else {
            const float alpha = 1.0f;
            const float beta  = 0.0f;
            CUBLAS_CHECK(cublasGemmEx(handle, CUBLAS_OP_T, CUBLAS_OP_N, n_oc, n_rows, IC,
                &alpha, w_buf.get(),   CUDA_R_16F, IC,
                        u,             CUDA_R_16F, IC,
                &beta,  partial + oc0, CUDA_R_32F, OC,
                CUBLAS_COMPUTE_32F, CUBLAS_GEMM_DEFAULT_TENSOR_OP));
        }
    }
}

// The prefill path of a projection node for its batch: recon or i8, or mma when the batch does not take the reconstruction path
static escha_prefill_mode escha_prefill_for(ggml_backend_cuda_context & ctx, const ggml_tensor * dst) {
    const ggml_tensor * code = dst->src[0];
    const ggml_tensor * x    = dst->src[5];

    const int OC     = code->ne[1]*16;
    const int n_rows = x->ne[1]*x->ne[2];

    const bool want_i8 = escha_prefill() == ESCHA_PREFILL_I8;

    if (n_rows <= ESCHA_GEN_MAX_ROWS || n_rows < (want_i8 ? escha_i8_min_rows() : escha_recon_min_rows())
            || ggml_cuda_info().devices[ctx.device].cc < GGML_CUDA_CC_TURING
            || OC % ESCHA_MMA_BN != 0
            || getenv("ESCHA_NO_MMA") != nullptr) {
        return ESCHA_PREFILL_MMA;
    }
    if (want_i8) {
        // the int8 path needs the warp kernel for the output rotation (16-byte aligned rows)
        const bool rows_aligned = (uintptr_t) dst->data % 16 == 0 && dst->nb[1] % 16 == 0 && dst->nb[2] % 16 == 0;
        if (rows_aligned) {
            return ESCHA_PREFILL_I8;
        }
        return n_rows >= escha_recon_min_rows() ? ESCHA_PREFILL_RECON : ESCHA_PREFILL_MMA;
    }
    return escha_prefill();
}

// Input rotation, decode and GEMM of one projection node: partial[row][c] = xh[row] . decode(code)[c].
// i8: the rows are quantized first, partial has int32 sums and scales[row] is their fp32 factor.
// xh_ready: the rotated input in fp16 when a fused epilogue made it already.
static void escha_recon_stage(ggml_backend_cuda_context & ctx, const ggml_tensor * dst, const bool i8, float * partial, float * scales,
                              const half * xh_ready = nullptr) {
    const ggml_tensor * code = dst->src[0];
    const ggml_tensor * rin  = dst->src[1];
    const ggml_tensor * x    = dst->src[5];

    const int K      = code->ne[0]/16;
    const int OC     = code->ne[1]*16;
    const int IC     = code->ne[2]*16;
    const int n_rows = x->ne[1]*x->ne[2];

    cudaStream_t stream = ctx.stream();

    // one warp for each 128 channels (escha-gemv.cu); the block kernel is the fallback
    ggml_cuda_pool_alloc<half> u_buf(ctx.pool());
    const half * xh = xh_ready;
    if (xh == nullptr) {
        xh = u_buf.alloc((size_t) n_rows*IC);
        if (!ggml_cuda_escha_had_in(ctx, x, rin, u_buf.get())) {
            escha_rotate_in_dense<half><<<n_rows, 256, 0, stream>>>(
                (const half *) rin->data, (const float *) x->data, u_buf.get(),
                IC, (int) x->ne[1], x->nb[1], x->nb[2]);
            CUDA_CHECK(cudaGetLastError());
        }
    }

    if (i8) {
        ggml_cuda_pool_alloc<int8_t> q_buf(ctx.pool(), (size_t) n_rows*IC);
        escha_quantize_rows_i8<<<n_rows, 256, 0, stream>>>(xh, q_buf.get(), scales, IC);
        CUDA_CHECK(cudaGetLastError());
        escha_mul_mat_recon(ctx, (const int16_t *) code->data, nullptr, q_buf.get(), partial, K, IC, OC, n_rows);
    } else {
        escha_mul_mat_recon(ctx, (const int16_t *) code->data, xh, nullptr, partial, K, IC, OC, n_rows);
    }
}

// One projection node on the recon / i8 path: the stage above, then the output rotation into dst
static void escha_recon_node(ggml_backend_cuda_context & ctx, ggml_tensor * dst, const bool i8, const half * xh_ready = nullptr) {
    const ggml_tensor * rout = dst->src[2];
    const ggml_tensor * x    = dst->src[5];

    const int OC     = dst->src[0]->ne[1]*16;
    const int n_rows = x->ne[1]*x->ne[2];

    // the GEMM writes into dst when its rows are contiguous, and the output rotation then works in place
    const bool in_place = dst->nb[1] == (size_t) OC*sizeof(float) && dst->nb[2] == dst->nb[1]*dst->ne[1];

    ggml_cuda_pool_alloc<float> p_buf(ctx.pool());
    ggml_cuda_pool_alloc<float> s_buf(ctx.pool());
    float * partial = in_place ? (float *) dst->data : p_buf.alloc((size_t) n_rows*OC);
    if (i8) {
        s_buf.alloc((size_t) n_rows);
    }

    escha_recon_stage(ctx, dst, i8, partial, s_buf.get(), xh_ready);

    if (i8) {
        const bool done = ggml_cuda_escha_had_out_i32(ctx, (const int32_t *) partial, s_buf.get(), rout, dst);
        GGML_ASSERT(done);
    } else if (!ggml_cuda_escha_had_out(ctx, partial, rout, dst)) {
        escha_finalize_dense<<<dim3(n_rows, OC/ESCHA_NT), ESCHA_NT, 0, ctx.stream()>>>(
            (const half *) rout->data, partial, (float *) dst->data,
            OC, (int) x->ne[1], n_rows, 1, dst->nb[1], dst->nb[2]);
        CUDA_CHECK(cudaGetLastError());
    }
}

static bool escha_ranges_overlap(const void * a, const void * b, const size_t bytes) {
    const uintptr_t pa = (uintptr_t) a;
    const uintptr_t pb = (uintptr_t) b;
    return pa < pb + bytes && pb < pa + bytes;
}

// mul = silu(gate(x)) * up(x) for two projection nodes that take the recon / i8 path: the two GEMMs, then one epilogue kernel for both output rotations, SiLU and the product.
// Without it the two rotated outputs, the SiLU output and the product are 4 fp32 passes over [rows][OC]. Returns false if it does not take the nodes.
// down != nullptr: the projection that has mul as its input is done too. The epilogue then writes the rotated fp16 input of down, and mul gets no data.
// GGML_ESCHA_FUSE_SWIGLU=0: no fused epilogue (each node runs alone). GGML_ESCHA_FUSE_SWIGLU=1: the form without down only.
bool ggml_cuda_op_escha_swiglu(ggml_backend_cuda_context & ctx, ggml_tensor * gate, ggml_tensor * up, ggml_tensor * mul, ggml_tensor * down) {
    static const int level = getenv("GGML_ESCHA_FUSE_SWIGLU") == nullptr ? 2 : atoi(getenv("GGML_ESCHA_FUSE_SWIGLU"));
    if (level <= 0 || (down != nullptr && level < 2)) {
        return false;
    }

    const int OC     = gate->src[0]->ne[1]*16;
    const int n_rows = gate->src[5]->ne[1]*gate->src[5]->ne[2];

    const escha_prefill_mode mode = escha_prefill_for(ctx, gate);
    if (mode == ESCHA_PREFILL_MMA || escha_prefill_for(ctx, up) != mode
            || n_rows <= ggml_cuda_escha_gemv_max_rows()
            || up->src[0]->ne[1]*16 != OC || up->src[5]->ne[1]*up->src[5]->ne[2] != n_rows
            || gate->type != GGML_TYPE_F32 || up->type != GGML_TYPE_F32 || mul->type != GGML_TYPE_F32
            || !ggml_are_same_shape(gate, mul) || !ggml_are_same_shape(up, mul)) {
        return false;
    }

    // the epilogue reads and writes whole rows of 128-column groups, 16 bytes for each lane
    auto rows_ok = [&](const ggml_tensor * t) {
        return (uintptr_t) t->data % 16 == 0 && t->nb[1] == (size_t) OC*sizeof(float) && t->nb[2] == t->nb[1]*t->ne[1];
    };
    if (down == nullptr && !rows_ok(mul)) {
        return false;
    }
    if (down != nullptr && (down->src[5] != mul || down->src[0]->ne[2]*16 != OC || escha_prefill_for(ctx, down) != mode)) {
        return false;
    }

    const bool   i8    = mode == ESCHA_PREFILL_I8;
    const size_t bytes = (size_t) n_rows*OC*sizeof(float);

    // The sums go into the buffers of the gate and up nodes. The graph allocator can give the memory of a node to a later node of the group:
    // a buffer is used only if it is the buffer of mul (in place, row by row) or apart from it, and the two are apart from each other.
    // With down, mul gets no data and down is written after the epilogue has read the sums.
    auto usable = [&](const ggml_tensor * t) {
        return rows_ok(t) && (down != nullptr || t->data == mul->data || !escha_ranges_overlap(t->data, mul->data, bytes));
    };

    ggml_cuda_pool_alloc<float> pg_buf(ctx.pool());
    ggml_cuda_pool_alloc<float> pu_buf(ctx.pool());
    ggml_cuda_pool_alloc<float> sg_buf(ctx.pool());
    ggml_cuda_pool_alloc<float> su_buf(ctx.pool());

    float * part_g = usable(gate) ? (float *) gate->data : pg_buf.alloc((size_t) n_rows*OC);
    float * part_u = usable(up) && !escha_ranges_overlap(up->data, part_g, bytes) ? (float *) up->data : pu_buf.alloc((size_t) n_rows*OC);
    if (i8) {
        sg_buf.alloc((size_t) n_rows);
        su_buf.alloc((size_t) n_rows);
    }

    escha_recon_stage(ctx, gate, i8, part_g, sg_buf.get());
    escha_recon_stage(ctx, up,   i8, part_u, su_buf.get());

    if (down != nullptr) {
        ggml_cuda_pool_alloc<half> xh_buf(ctx.pool(), (size_t) n_rows*OC);
        const bool done = ggml_cuda_escha_swiglu_xh(ctx, part_g, part_u, sg_buf.get(), su_buf.get(), gate->src[2], up->src[2], down->src[1], xh_buf.get(), OC, n_rows);
        GGML_ASSERT(done);
        escha_recon_node(ctx, down, i8, xh_buf.get());
        return true;
    }

    const bool done = ggml_cuda_escha_swiglu_out(ctx, part_g, part_u, sg_buf.get(), su_buf.get(), gate->src[2], up->src[2], mul);
    GGML_ASSERT(done);
    return true;
}

void ggml_cuda_op_escha_mul_mat(ggml_backend_cuda_context & ctx, ggml_tensor * dst) {
    const ggml_tensor * code = dst->src[0];
    const ggml_tensor * rin  = dst->src[1];
    const ggml_tensor * rout = dst->src[2];
    const ggml_tensor * lut  = dst->src[3];
    const ggml_tensor * dep  = dst->src[4];
    const ggml_tensor * x    = dst->src[5];

    GGML_ASSERT(code->type == GGML_TYPE_I16 && dep->type == GGML_TYPE_I16);
    GGML_ASSERT(rin->type == GGML_TYPE_F16 && rout->type == GGML_TYPE_F16 && lut->type == GGML_TYPE_F16);
    GGML_ASSERT(x->type == GGML_TYPE_F32 && dst->type == GGML_TYPE_F32);

    // rows <= 8 on tensor cores, GGML_ESCHA_GEMV=0 keeps the fp32 kernels below
    if (ggml_cuda_escha_gemv(ctx, dst)) {
        return;
    }

    const int K   = code->ne[0]/16;
    const int OC  = code->ne[1]*16;
    const int IC  = code->ne[2]*16;
    const int nit = IC/ESCHA_TILE;

    const int n_rows = x->ne[1]*x->ne[2];
    const int n_ocb  = OC/ESCHA_NT;

    cudaStream_t stream = ctx.stream();

    // prefill by reconstruction (exact read) or its int8 form (fast read)
    const escha_prefill_mode mode = escha_prefill_for(ctx, dst);
    if (mode != ESCHA_PREFILL_MMA) {
        escha_recon_node(ctx, dst, mode == ESCHA_PREFILL_I8);
        return;
    }

    // The tensor-core path wants its activations already in fp16 so cp.async can move them
    // verbatim, so the rotation has to know its consumer before it runs.
    // n_rows==1 is decode; 2..ESCHA_GEN_MAX_ROWS is speculative verify (reuse decoded
    // weights across the small batch instead of running the R=1 kernel 4x).
    const bool decode1 = n_rows == 1;
    const bool specb   = n_rows > 1 && n_rows <= ESCHA_GEN_MAX_ROWS;
    const bool gen     = decode1 || specb;
    const bool use_mma = !gen
                      && ggml_cuda_info().devices[ctx.device].cc >= GGML_CUDA_CC_TURING
                      && OC % ESCHA_MMA_BN == 0
                      && getenv("ESCHA_NO_MMA") == nullptr;

    ggml_cuda_pool_alloc<char> u_buf(ctx.pool(),
        (size_t) n_rows*IC*(use_mma ? sizeof(half) : sizeof(float)));

    if (use_mma) {
        escha_rotate_in_dense<half><<<n_rows, 256, 0, stream>>>(
            (const half *) rin->data, (const float *) x->data, (half *) u_buf.get(),
            IC, (int) x->ne[1], x->nb[1], x->nb[2]);
    } else {
        escha_rotate_in_dense<float><<<n_rows, 256, 0, stream>>>(
            (const half *) rin->data, (const float *) x->data, (float *) u_buf.get(),
            IC, (int) x->ne[1], x->nb[1], x->nb[2]);
    }
    CUDA_CHECK(cudaGetLastError());

    // slice the IC reduction only as far as it takes to fill the device: at batch 1 the
    // natural grid is just n_ocb blocks, but a long prompt already has plenty of rows
    const int  R   = decode1 ? ESCHA_ROWS_DENSE_GEN : (specb ? ESCHA_ROWS_DENSE_SPEC : ESCHA_ROWS_DENSE);

    const int n_rb = (n_rows + R - 1)/R;
    // the tiled prefill kernel blocks over BM rows x BN columns instead
    const int n_tb = (n_rows + ESCHA_BM - 1)/ESCHA_BM;
    const int n_cb = OC/ESCHA_BN;
    // batch 1 has only n_ocb blocks before slicing (136 for the FFN), which leaves an 82-SM
    // device mostly idle, so generation slices the reduction much harder than prefill
    const int target = gen ? ESCHA_GEN_TARGET_MUL*ESCHA_TARGET : ESCHA_TARGET;
    // batch-invariant mode: 2 to ESCHA_GEN_MAX_ROWS rows get the slice count of 1 row, so the fp32 sum of a row has the same order alone and in a batch
    // VITRIOL: ggml_cuda_batch_invariant() is a newer-upstream toggle our frozen fork
    // does not carry. Treat it as off, which is exactly the pre-feature behaviour:
    // n_rb_slices == n_rb. We make no batch-invariance claim.
    const int n_rb_slices = n_rb;
    int n_slices = target/MAX(1, gen ? n_rb_slices*n_ocb : n_tb*n_cb);
    n_slices = MIN(MAX(n_slices, 1), nit);

    ggml_cuda_pool_alloc<float> p_buf(ctx.pool(), (size_t) n_slices*n_rows*OC);
    float * partial = p_buf.get();

    if (decode1) {
        // widest slice any block gets, since lo/hi split nit unevenly by at most one tile
        const int tiles_max = (nit + n_slices - 1)/n_slices;
        const size_t smem = ESCHA_GROUPS*ESCHA_MAX_W*sizeof(uint2)
                          + (size_t) tiles_max*ESCHA_TILE*sizeof(float);
        GGML_ASSERT(smem <= 48*1024 && "escha: staged u exceeds the default shared budget");
        auto launch = [&](auto kernel) {
            kernel<<<dim3(n_rb, n_ocb, n_slices), ESCHA_NT, smem, stream>>>(
                (const int16_t *) code->data, (const half *) lut->data, (const int16_t *) dep->data,
                (const float *) u_buf.get(), partial, IC, OC, n_rows, n_slices);
        };
        switch (K) {
            case 2: launch(escha_matmul_dense<2, ESCHA_ROWS_DENSE_GEN>); break;
            case 3: launch(escha_matmul_dense<3, ESCHA_ROWS_DENSE_GEN>); break;
            default: GGML_ABORT("escha: unsupported K=%d", K);
        }
    } else if (specb) {
        const size_t smem = ESCHA_GROUPS*ESCHA_MAX_W*sizeof(uint2)
                          + (size_t) ESCHA_ROWS_DENSE_SPEC*ESCHA_TILE*sizeof(float);
        auto launch = [&](auto kernel) {
            kernel<<<dim3(n_rb, n_ocb, n_slices), ESCHA_NT, smem, stream>>>(
                (const int16_t *) code->data, (const half *) lut->data, (const int16_t *) dep->data,
                (const float *) u_buf.get(), partial, IC, OC, n_rows, n_slices);
        };
        switch (K) {
            case 2: launch(escha_matmul_dense<2, ESCHA_ROWS_DENSE_SPEC>); break;
            case 3: launch(escha_matmul_dense<3, ESCHA_ROWS_DENSE_SPEC>); break;
            default: GGML_ABORT("escha: unsupported K=%d", K);
        }
    } else if (OC % ESCHA_BN != 0) {
        // the tiled kernel blocks the output axis in exact BN steps; a ragged OC would
        // silently leave the tail columns unwritten. Every projection in this checkpoint is
        // 128-aligned, but that is a property of the model, not of the format.
        const size_t smem = ESCHA_GROUPS*ESCHA_MAX_W*sizeof(uint2)
                          + (size_t) ESCHA_ROWS_DENSE*ESCHA_TILE*sizeof(float);
        auto launch = [&](auto kernel) {
            kernel<<<dim3((n_rows + ESCHA_ROWS_DENSE - 1)/ESCHA_ROWS_DENSE, n_ocb, n_slices),
                     ESCHA_NT, smem, stream>>>(
                (const int16_t *) code->data, (const half *) lut->data, (const int16_t *) dep->data,
                (const float *) u_buf.get(), partial, IC, OC, n_rows, n_slices);
        };
        switch (K) {
            case 2: launch(escha_matmul_dense<2, ESCHA_ROWS_DENSE>); break;
            case 3: launch(escha_matmul_dense<3, ESCHA_ROWS_DENSE>); break;
            default: GGML_ABORT("escha: unsupported K=%d", K);
        }
    } else if (use_mma) {
        // tensor-core prefill. Weights are exact; activations are rounded to fp16, which is
        // what escha's runtime does. ESCHA_NO_MMA=1 falls back to the fp32 FMA kernel.
        constexpr int NTJ = ESCHA_MMA_BN/ESCHA_TILE;
        const size_t smem = NTJ*ESCHA_MAX_W*sizeof(uint2)
                          + (size_t) 2*ESCHA_MMA_BM*ESCHA_TILE*sizeof(half)
                          + (size_t) ESCHA_MMA_BN*ESCHA_TILE*sizeof(half);
        const int n_tb_mma = (n_rows + ESCHA_MMA_BM - 1)/ESCHA_MMA_BM;
        const int n_cb_mma = OC/ESCHA_MMA_BN;
        auto launch = [&](auto kernel) {
            kernel<<<dim3(n_tb_mma, n_cb_mma, n_slices), dim3(32, 256/32), smem, stream>>>(
                (const int16_t *) code->data, (const half *) lut->data, (const int16_t *) dep->data,
                (const half *) u_buf.get(), partial, IC, OC, n_rows, n_slices);
        };
        switch (K) {
            case 2: launch((escha_matmul_dense_tiled_mma<2, ESCHA_MMA_BM, ESCHA_MMA_BN>)); break;
            case 3: launch((escha_matmul_dense_tiled_mma<3, ESCHA_MMA_BM, ESCHA_MMA_BN>)); break;
            default: GGML_ABORT("escha: unsupported K=%d", K);
        }
    } else {
        constexpr int NT  = (ESCHA_BM/ESCHA_TM)*(ESCHA_BN/ESCHA_TN);
        constexpr int NTJ = ESCHA_BN/ESCHA_TILE;
        const size_t smem = NTJ*ESCHA_MAX_W*sizeof(uint32_t)
                          + (size_t) ESCHA_TILE*ESCHA_BN*sizeof(float)
                          + (size_t) 2*ESCHA_TILE*ESCHA_BM*sizeof(float);
        auto launch = [&](auto kernel) {
            kernel<<<dim3(n_tb, n_cb, n_slices), NT, smem, stream>>>(
                (const int16_t *) code->data, (const half *) lut->data, (const int16_t *) dep->data,
                (const float *) u_buf.get(), partial, IC, OC, n_rows, n_slices);
        };
        switch (K) {
            case 2: launch((escha_matmul_dense_tiled<2, ESCHA_BM, ESCHA_BN, ESCHA_TM, ESCHA_TN>)); break;
            case 3: launch((escha_matmul_dense_tiled<3, ESCHA_BM, ESCHA_BN, ESCHA_TM, ESCHA_TN>)); break;
            default: GGML_ABORT("escha: unsupported K=%d", K);
        }
    }
    CUDA_CHECK(cudaGetLastError());

    escha_finalize_dense<<<dim3(n_rows, n_ocb), ESCHA_NT, 0, stream>>>(
        (const half *) rout->data, partial, (float *) dst->data,
        OC, (int) x->ne[1], n_rows, n_slices, dst->nb[1], dst->nb[2]);
    CUDA_CHECK(cudaGetLastError());
}
