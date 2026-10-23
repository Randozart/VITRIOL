#include "gated_delta_net.cuh"
#include "ggml-cuda/common.cuh"

template <int S_v, bool KDA, bool keep_rs_t>
__global__ void __launch_bounds__((ggml_cuda_get_physical_warp_size() < S_v ? ggml_cuda_get_physical_warp_size() : S_v) * 4, 2)
gated_delta_net_cuda(const float * q,
                                     const float * k,
                                     const float * v,
                                     const float * g,
                                     const float * beta,
                                     const float * curr_state,
                                     float *       dst,
                                     float *       state,
                                     int64_t       H,
                                     int64_t       n_tokens,
                                     int64_t       n_seqs,
                                     int64_t       sq1,
                                     int64_t       sq2,
                                     int64_t       sq3,
                                     int64_t       sv1,
                                     int64_t       sv2,
                                     int64_t       sv3,
                                     int64_t       sb1,
                                     int64_t       sb2,
                                     int64_t       sb3,
                                     const uint3   neqk1_magic,
                                     const uint3   rq3_magic,
                                     float         scale,
                                     int64_t       state_slot_stride,
                                     int           K) {
    const uint32_t h_idx    = blockIdx.x;
    const uint32_t sequence = blockIdx.y;
    // each warp owns one column, using warp-level primitives to reduce across rows
    const int      lane     = threadIdx.x;
    const int      col      = blockIdx.z * blockDim.y + threadIdx.y;

    const uint32_t iq1 = fastmodulo(h_idx, neqk1_magic);
    const uint32_t iq3 = fastdiv(sequence, rq3_magic);

    float *       attn_data        = dst;

    // input state holds s0 only: [S_v, S_v, H, n_seqs] — seq stride is D = H * S_v * S_v.
    // output state layout (per-slot D * n_seqs) — same per-(seq,head) offset as before.
    const int64_t state_in_offset      = sequence * H * S_v * S_v + h_idx * S_v * S_v;
    const int64_t state_out_offset     = (sequence * H + h_idx) * S_v * S_v;
    state += state_out_offset;
    curr_state += state_in_offset + col * S_v;
    attn_data += (sequence * n_tokens * H + h_idx) * S_v;

    constexpr int warp_size = ggml_cuda_get_physical_warp_size() < S_v ? ggml_cuda_get_physical_warp_size() : S_v;
    static_assert(S_v % warp_size == 0, "S_v must be a multiple of warp_size");
    constexpr int rows_per_lane = (S_v + warp_size - 1) / warp_size;
    float         s_shard[rows_per_lane];
    // state is stored transposed: M[col][i] = S[i][col], row col is contiguous

    ggml_cuda_pdl_sync();
#pragma unroll
    for (int r = 0; r < rows_per_lane; r++) {
        const int i = r * warp_size + lane;
        s_shard[r]  = curr_state[i];
    }

    for (int t = 0; t < n_tokens; t++) {
        const float * q_t = q + iq3 * sq3 + t * sq2 + iq1 * sq1;
        const float * k_t = k + iq3 * sq3 + t * sq2 + iq1 * sq1;
        const float * v_t = v + sequence * sv3 + t * sv2 + h_idx * sv1;

        const int64_t gb_offset = sequence * sb3 + t * sb2 + h_idx * sb1;
        const float * beta_t = beta + gb_offset;
        const float * g_t    = g    + gb_offset * (KDA ? S_v : 1);

        const float beta_val = *beta_t;

        // Cache k and q in registers
        float k_reg[rows_per_lane];
        float q_reg[rows_per_lane];
#pragma unroll
        for (int r = 0; r < rows_per_lane; r++) {
            const int i = r * warp_size + lane;
            k_reg[r] = k_t[i];
            q_reg[r] = q_t[i];
        }

        if constexpr (!KDA) {
            const float g_val = expf(*g_t);

            // kv[col] = (S^T @ k)[col] = sum_i S[i][col] * k[i]
            float kv_shard = 0.0f;
#pragma unroll
            for (int r = 0; r < rows_per_lane; r++) {
                kv_shard += s_shard[r] * k_reg[r];
            }
            float kv_col = warp_reduce_sum<warp_size>(kv_shard);

            // delta[col] = (v[col] - g * kv[col]) * beta
            float delta_col = (v_t[col] - g_val * kv_col) * beta_val;

            // fused: S[i][col] = g * S[i][col] + k[i] * delta[col]
            // attn[col] = (S^T @ q)[col] = sum_i S[i][col] * q[i]
            float attn_partial = 0.0f;
#pragma unroll
            for (int r = 0; r < rows_per_lane; r++) {
                s_shard[r]  = g_val * s_shard[r] + k_reg[r] * delta_col;
                attn_partial += s_shard[r] * q_reg[r];
            }

            float attn_col = warp_reduce_sum<warp_size>(attn_partial);

            if (lane == 0) {
                attn_data[col] = attn_col * scale;
            }
        } else {
            // kv[col] = sum_i g[i] * S[i][col] * k[i]
            float kv_shard = 0.0f;
#pragma unroll
            for (int r = 0; r < rows_per_lane; r++) {
                const int i = r * warp_size + lane;
                kv_shard += expf(g_t[i]) * s_shard[r] * k_reg[r];
            }

            float kv_col = warp_reduce_sum<warp_size>(kv_shard);

            // delta[col] = (v[col] - kv[col]) * beta
            float delta_col = (v_t[col] - kv_col) * beta_val;

            // fused: S[i][col] = g[i] * S[i][col] + k[i] * delta[col]
            // attn[col] = (S^T @ q)[col] = sum_i S[i][col] * q[i]
            float attn_partial = 0.0f;
#pragma unroll
            for (int r = 0; r < rows_per_lane; r++) {
                const int i = r * warp_size + lane;
                s_shard[r]  = expf(g_t[i]) * s_shard[r] + k_reg[r] * delta_col;
                attn_partial += s_shard[r] * q_reg[r];
            }

            float attn_col = warp_reduce_sum<warp_size>(attn_partial);

            if (lane == 0) {
                attn_data[col] = attn_col * scale;
            }
        }

        attn_data += S_v * H;

        if constexpr (keep_rs_t) {
            // snapshot slot mapping: slot 0 = most recent state, slot s = s tokens back.
            // When n_tokens < K only slots 0..n_tokens-1 are written; older slots are caller-owned.
            const int target_slot = (int) n_tokens - 1 - t;
            if (target_slot >= 0 && target_slot < K) {
                float * curr_state = state + target_slot * state_slot_stride;
#pragma unroll
                for (int r = 0; r < rows_per_lane; r++) {
                    const int i = r * warp_size + lane;
                    curr_state[col * S_v + i] = s_shard[r];
                }
            }
        }
    }

    if constexpr (!keep_rs_t) {
#pragma unroll
        for (int r = 0; r < rows_per_lane; r++) {
            const int i          = r * warp_size + lane;
            state[col * S_v + i] = s_shard[r];
        }
    }
}

// PROVENANCE: inspiration — Nehanth/swarmllm (MIT), chunked Gated Delta Net
// prefill (running-product decays + C-step triangular solve); the algorithm is
// from arXiv 2412.06464 (Gated Delta Net, Sec 3.3), SwarmLLM is the verified
// implementation reference. Re-derived for CUDA against the fork's serial
// kernel; the (I+A)D=R identity is the same math, not copied code.
//
// Chunk identity (C tokens, state S_0):
//   alpha_r = expf(g_r)                  (fork's non-KDA decay)
//   gamma_r = prod_{m=0..r} alpha_m
//   Lam[r][i] = prod_{m=i+1..r} alpha_m  (i <= r; Lam[r][r] = 1)
//   (E1) A[r][i] = beta_r * Lam[r][i] * (k_r . k_i),  i < r
//   (E2) R[r] = beta_r v_r - beta_r gamma_r (S_0^T k_r)
//   (E3) (I + A) D = R, unit-lower forward substitution
//   (E4) P[r][i] = Lam[r][i] * (q_r . k_i),  i <= r
//   (E5) o_r = scale * (gamma_r (S_0^T q_r) + sum_{i<=r} P[r][i] D[i])
//   (E6) S_C = gamma_{C-1} S_0 + sum_r lamC[r] k_r D[r]^T,  lamC[r] = Lam[C-1][r]
//   (E7) snapshot S_r = gamma_r S_0 + sum_{i<=r} Lam[r][i] k_i D[i]^T
template <int S_v, int C, bool keep_rs_t>
__global__ void __launch_bounds__((ggml_cuda_get_physical_warp_size() < S_v ? ggml_cuda_get_physical_warp_size() : S_v) * 4, 2)
gated_delta_net_chunk_cuda(const float * q,
                                        const float * k,
                                        const float * v,
                                        const float * g,
                                        const float * beta,
                                        const float * curr_state,
                                        float *       dst,
                                        float *       state,
                                        int64_t       H,
                                        int64_t       n_tokens,
                                        int64_t       n_seqs,
                                        int64_t       sq1,
                                        int64_t       sq2,
                                        int64_t       sq3,
                                        int64_t       sv1,
                                        int64_t       sv2,
                                        int64_t       sv3,
                                        int64_t       sb1,
                                        int64_t       sb2,
                                        int64_t       sb3,
                                        const uint3   neqk1_magic,
                                        const uint3   rq3_magic,
                                        float         scale,
                                        int64_t       state_slot_stride,
                                        int           K) {
    const uint32_t h_idx    = blockIdx.x;
    const uint32_t sequence = blockIdx.y;
    const int      lane     = threadIdx.x;
    const int      warp     = threadIdx.y;
    const int      col      = blockIdx.z * blockDim.y + warp;

    const uint32_t iq1 = fastmodulo(h_idx, neqk1_magic);
    const uint32_t iq3 = fastdiv(sequence, rq3_magic);

    float * attn_data = dst;

    const int64_t state_in_offset  = sequence * H * S_v * S_v + h_idx * S_v * S_v;
    const int64_t state_out_offset = (sequence * H + h_idx) * S_v * S_v;
    state += state_out_offset;
    curr_state += state_in_offset + col * S_v;
    attn_data += (sequence * n_tokens * H + h_idx) * S_v;

    constexpr int warp_size = ggml_cuda_get_physical_warp_size() < S_v ? ggml_cuda_get_physical_warp_size() : S_v;
    static_assert(S_v % warp_size == 0, "S_v must be a multiple of warp_size");
    constexpr int rows_per_lane = (S_v + warp_size - 1) / warp_size;

    float s_shard[rows_per_lane];
    float k_reg[C][rows_per_lane];
    float q_reg[C][rows_per_lane];
    float v_reg[C];

    __shared__ float s_A[C][C];
    __shared__ float s_P[C][C];
    __shared__ float s_Lam[C][C];
    __shared__ float s_gamma[C];
    __shared__ float s_lamC[C];
    __shared__ float s_alpha[C];
    __shared__ float s_beta[C];

    ggml_cuda_pdl_sync();
#pragma unroll
    for (int r = 0; r < rows_per_lane; r++) {
        const int i = r * warp_size + lane;
        s_shard[r]  = curr_state[i];
    }

    /* Load per-token q, k (own rows) and v (own column) into registers. */
#pragma unroll
    for (int t = 0; t < C; t++) {
        const float * q_t = q + iq3 * sq3 + t * sq2 + iq1 * sq1;
        const float * k_t = k + iq3 * sq3 + t * sq2 + iq1 * sq1;
        const float * v_t = v + sequence * sv3 + t * sv2 + h_idx * sv1;
        v_reg[t] = v_t[col];
#pragma unroll
        for (int r = 0; r < rows_per_lane; r++) {
            const int i = r * warp_size + lane;
            k_reg[t][r] = k_t[i];
            q_reg[t][r] = q_t[i];
        }
    }

    /* Per-token alpha (decay) and beta; running products. */
    if (lane == 0 && warp == 0) {
        const int64_t gb_offset_base = sequence * sb3 + h_idx * sb1;
        float gamma = 1.0f;
        for (int t = 0; t < C; t++) {
            const int64_t gb_offset = gb_offset_base + t * sb2;
            s_alpha[t] = expf(*(g + gb_offset));
            s_beta[t]  = *(beta + gb_offset);
            gamma *= s_alpha[t];
            s_gamma[t] = gamma;
        }
        for (int r = 0; r < C; r++) {
            s_Lam[r][r] = 1.0f;
            for (int i = r - 1; i >= 0; i--) {
                s_Lam[r][i] = s_Lam[r][i + 1] * s_alpha[i + 1];
            }
        }
        for (int r = 0; r < C; r++) {
            s_lamC[r] = s_Lam[C - 1][r];
        }
    }
    __syncthreads();

    /* A[r][i] = beta_r Lam[r][i] (k_r . k_i), i < r; P[r][i] = Lam[r][i] (q_r . k_i), i <= r.
     * Each warp computes the same per-head dots; warp 0 writes them to shared. */
    {
        const int n_pairs = C * (C + 1) / 2;
        for (int p = warp; p < n_pairs; p += 4) {
            int r = 0, i = 0;
            /* map pair index to (r, i) with i <= r (upper triangle incl. diagonal) */
            for (int rr = 0; rr < C; rr++) {
                if (p < (rr + 1) * (rr + 2) / 2) {
                    r = rr;
                    i = p - rr * (rr + 1) / 2;
                    break;
                }
            }
            float dot_kk = 0.0f;
            float dot_qk = 0.0f;
#pragma unroll
            for (int m = 0; m < rows_per_lane; m++) {
                dot_kk += k_reg[r][m] * k_reg[i][m];
                dot_qk += q_reg[r][m] * k_reg[i][m];
            }
            dot_kk = warp_reduce_sum<warp_size>(dot_kk);
            dot_qk = warp_reduce_sum<warp_size>(dot_qk);
            if (lane == 0) {
                const float Lam_ri = s_Lam[r][i];
                s_P[r][i] = Lam_ri * dot_qk;
                if (i < r) {
                    s_A[r][i] = s_beta[r] * Lam_ri * dot_kk;
                }
            }
        }
    }
    __syncthreads();

    /* Per-column work: (S_0^T k_r)[col], (S_0^T q_r)[col] via warp reduce,
     * then forward substitution and outputs. */
    float kv[C];
    float aq[C];
#pragma unroll
    for (int r = 0; r < C; r++) {
        float kv_shard = 0.0f;
        float aq_shard = 0.0f;
#pragma unroll
        for (int m = 0; m < rows_per_lane; m++) {
            kv_shard += s_shard[m] * k_reg[r][m];
            aq_shard += s_shard[m] * q_reg[r][m];
        }
        kv[r] = warp_reduce_sum<warp_size>(kv_shard);
        aq[r] = warp_reduce_sum<warp_size>(aq_shard);
    }

    /* (I + A) D = R, unit-lower forward substitution (per column). */
    float R[C];
    float D[C];
#pragma unroll
    for (int r = 0; r < C; r++) {
        R[r] = s_beta[r] * v_reg[r] - s_beta[r] * s_gamma[r] * kv[r];
    }
#pragma unroll
    for (int r = 0; r < C; r++) {
        float acc = R[r];
#pragma unroll
        for (int i = 0; i < r; i++) {
            acc -= s_A[r][i] * D[i];
        }
        D[r] = acc;
    }

    /* o_r = scale * (gamma_r aq_r + sum_{i<=r} P[r][i] D[i]). */
#pragma unroll
    for (int r = 0; r < C; r++) {
        float acc = s_gamma[r] * aq[r];
#pragma unroll
        for (int i = 0; i <= r; i++) {
            acc += s_P[r][i] * D[i];
        }
        float attn_col = acc * scale;
        if (lane == 0) {
            attn_data[r * S_v * H + col] = attn_col;
        }
    }

    if constexpr (!keep_rs_t) {
        /* Final state S_C = gamma_{C-1} S_0 + sum_r lamC[r] k_r D[r]^T. */
#pragma unroll
        for (int m = 0; m < rows_per_lane; m++) {
            const int i = m * warp_size + lane;
            float s_new = s_gamma[C - 1] * s_shard[m];
#pragma unroll
            for (int r = 0; r < C; r++) {
                s_new += s_lamC[r] * k_reg[r][m] * D[r];
            }
            state[col * S_v + i] = s_new;
        }
    } else {
        /* Snapshots: state after token r is slot (C-1-r), written when in range. */
#pragma unroll
        for (int m = 0; m < rows_per_lane; m++) {
            const int i = m * warp_size + lane;
#pragma unroll
            for (int r = 0; r < C; r++) {
                const int target_slot = (int) n_tokens - 1 - r;
                if (target_slot >= 0 && target_slot < K) {
                    float s_r = s_gamma[r] * s_shard[m];
#pragma unroll
                    for (int i2 = 0; i2 <= r; i2++) {
                        s_r += s_Lam[r][i2] * k_reg[i2][m] * D[i2];
                    }
                    float * slot_state = state + target_slot * state_slot_stride;
                    slot_state[col * S_v + i] = s_r;
                }
            }
        }
    }
}

template <bool keep_rs_t>
static void launch_gated_delta_net_chunk(
        const float * q_d, const float * k_d, const float * v_d,
        const float * g_d, const float * b_d, const float * s_d,
        float * dst_d, float * state_d,
        int64_t S_v,   int64_t H, int64_t n_tokens, int64_t n_seqs,
        int64_t sq1,   int64_t sq2, int64_t sq3,
        int64_t sv1,   int64_t sv2, int64_t sv3,
        int64_t sb1,   int64_t sb2, int64_t sb3,
        int64_t neqk1, int64_t rq3,
        float scale, int64_t state_slot_stride, int K, cudaStream_t stream) {
    const int warp_size = ggml_cuda_info().devices[ggml_cuda_get_device()].warp_size;
    const int num_warps = 4;
    dim3      grid_dims(H, n_seqs, (S_v + num_warps - 1) / num_warps);
    dim3      block_dims(warp_size <= S_v ? warp_size : S_v, num_warps, 1);

    const uint3 neqk1_magic = init_fastdiv_values(neqk1);
    const uint3 rq3_magic   = init_fastdiv_values(rq3);

    const ggml_cuda_kernel_launch_params launch_params = ggml_cuda_kernel_launch_params(grid_dims, block_dims, 0, stream);
    switch (S_v) {
        case 16: {
            switch (n_tokens) {
                case 2:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<16, 2, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                case 3:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<16, 3, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                case 4:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<16, 4, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                case 6:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<16, 6, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                case 8:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<16, 8, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                default: GGML_ABORT("gated_delta_net_chunk: unsupported n_tokens"); break;
            }
            break;
        }
        case 32: {
            switch (n_tokens) {
                case 2:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<32, 2, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                case 3:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<32, 3, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                case 4:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<32, 4, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                case 6:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<32, 6, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                case 8:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<32, 8, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                default: GGML_ABORT("gated_delta_net_chunk: unsupported n_tokens"); break;
            }
            break;
        }
        case 64: {
            switch (n_tokens) {
                case 2:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<64, 2, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                case 3:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<64, 3, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                case 4:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<64, 4, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                case 6:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<64, 6, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                case 8:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<64, 8, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                default: GGML_ABORT("gated_delta_net_chunk: unsupported n_tokens"); break;
            }
            break;
        }
        case 128: {
            switch (n_tokens) {
                case 2:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<128, 2, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                case 3:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<128, 3, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                case 4:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<128, 4, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                case 6:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<128, 6, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                case 8:  ggml_cuda_kernel_launch(gated_delta_net_chunk_cuda<128, 8, keep_rs_t>, launch_params, q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3, sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K); break;
                default: GGML_ABORT("gated_delta_net_chunk: unsupported n_tokens"); break;
            }
            break;
        }
        default:
            GGML_ABORT("fatal error");
            break;
    }
}

// env toggle: GGML_CUDA_GDN_SCAN = off (serial only) | on (chunk for C in {2,3,4,6,8}) | auto (default)
static bool gdn_scan_enabled(void) {
    static const int mode = []() {
        const char * e = getenv("GGML_CUDA_GDN_SCAN");
        if (e == nullptr) return 0;          // auto
        if (strcmp(e, "off") == 0) return -1;
        return 1;                            // on (or anything else)
    }();
    return mode >= 0;
}

static bool gdn_scan_supported(int64_t n_tokens) {
    return n_tokens == 2 || n_tokens == 3 || n_tokens == 4 || n_tokens == 6 || n_tokens == 8;
}

template <bool KDA, bool keep_rs_t>
static void launch_gated_delta_net(
        const float * q_d, const float * k_d, const float * v_d,
        const float * g_d, const float * b_d, const float * s_d,
        float * dst_d, float * state_d,
        int64_t S_v,   int64_t H, int64_t n_tokens, int64_t n_seqs,
        int64_t sq1,   int64_t sq2, int64_t sq3,
        int64_t sv1,   int64_t sv2, int64_t sv3,
        int64_t sb1,   int64_t sb2, int64_t sb3,
        int64_t neqk1, int64_t rq3,
        float scale, int64_t state_slot_stride, int K, cudaStream_t stream) {
    //TODO: Add chunked kernel for even faster pre-fill
    const int warp_size = ggml_cuda_info().devices[ggml_cuda_get_device()].warp_size;
    const int num_warps = 4;
    dim3      grid_dims(H, n_seqs, (S_v + num_warps - 1) / num_warps);
    dim3      block_dims(warp_size <= S_v ? warp_size : S_v, num_warps, 1);

    const uint3 neqk1_magic = init_fastdiv_values(neqk1);
    const uint3 rq3_magic   = init_fastdiv_values(rq3);

    const ggml_cuda_kernel_launch_params launch_params = ggml_cuda_kernel_launch_params(grid_dims, block_dims, 0, stream);
    switch (S_v) {
        case 16:
            ggml_cuda_kernel_launch(gated_delta_net_cuda<16, KDA, keep_rs_t>, launch_params,
                q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H,
                n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3,
                sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K);
            break;
        case 32:
            ggml_cuda_kernel_launch(gated_delta_net_cuda<32, KDA, keep_rs_t>, launch_params,
                q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H,
                n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3,
                sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K);
            break;
        case 64: {
            ggml_cuda_kernel_launch(gated_delta_net_cuda<64, KDA, keep_rs_t>, launch_params,
                q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H,
                n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3,
                sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K);
            break;
        }
        case 128: {
            ggml_cuda_kernel_launch(gated_delta_net_cuda<128, KDA, keep_rs_t>, launch_params,
                q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d, H,
                n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3,
                sb1, sb2, sb3, neqk1_magic, rq3_magic, scale, state_slot_stride, K);
            break;
        }
        default:
            GGML_ABORT("fatal error");
            break;
    }
}

static void ggml_cuda_op_gated_delta_net_impl(
        ggml_backend_cuda_context & ctx, ggml_tensor * dst, const ggml_cuda_gated_delta_net_fused_cache * cache) {
    ggml_tensor * src_q     = dst->src[0];
    ggml_tensor * src_k     = dst->src[1];
    ggml_tensor * src_v     = dst->src[2];
    ggml_tensor * src_g     = dst->src[3];
    ggml_tensor * src_beta  = dst->src[4];
    ggml_tensor * src_state = dst->src[5];

    GGML_TENSOR_LOCALS(int64_t, neq, src_q, ne);
    GGML_TENSOR_LOCALS(size_t , nbq, src_q, nb);
    GGML_TENSOR_LOCALS(int64_t, nek, src_k, ne);
    GGML_TENSOR_LOCALS(size_t , nbk, src_k, nb);
    GGML_TENSOR_LOCALS(int64_t, nev, src_v, ne);
    GGML_TENSOR_LOCALS(size_t,  nbv, src_v, nb);
    GGML_TENSOR_LOCALS(size_t,  nbb, src_beta, nb);

    const int64_t S_v      = nev0;
    const int64_t H        = nev1;
    const int64_t n_tokens = nev2;
    const int64_t n_seqs   = nev3;

    const bool kda = (src_g->ne[0] == S_v);

    GGML_ASSERT(neq1 == nek1);
    const int64_t neqk1 = neq1;

    const int64_t rq3 = nev3 / neq3;

    const float * q_d = (const float *) src_q->data;
    const float * k_d = (const float *) src_k->data;
    const float * v_d = (const float *) src_v->data;
    const float * g_d = (const float *) src_g->data;
    const float * b_d = (const float *) src_beta->data;

    const float * s_d   = (const float *) src_state->data;
    float *       dst_d = (float *) dst->data;

    GGML_ASSERT(ggml_is_contiguous_rows(src_q));
    GGML_ASSERT(ggml_is_contiguous_rows(src_k));
    GGML_ASSERT(ggml_is_contiguous_rows(src_v));
    GGML_ASSERT(ggml_are_same_stride(src_q, src_k));
    GGML_ASSERT(src_g->ne[0] == 1 || kda);
    GGML_ASSERT(ggml_is_contiguous(src_g));
    GGML_ASSERT(ggml_is_contiguous(src_beta));
    GGML_ASSERT(ggml_is_contiguous(src_state));

    // strides in floats (beta strides used for both g and beta offset computation)
    const int64_t sq1 = nbq1 / sizeof(float);
    const int64_t sq2 = nbq2 / sizeof(float);
    const int64_t sq3 = nbq3 / sizeof(float);
    const int64_t sv1 = nbv1 / sizeof(float);
    const int64_t sv2 = nbv2 / sizeof(float);
    const int64_t sv3 = nbv3 / sizeof(float);
    const int64_t sb1 = nbb1 / sizeof(float);
    const int64_t sb2 = nbb2 / sizeof(float);
    const int64_t sb3 = nbb3 / sizeof(float);

    const float scale = 1.0f / sqrtf((float) S_v);

    cudaStream_t stream = ctx.stream();

    // K (snapshot slot count) is an op param; state holds s0 only [S_v, S_v, H, n_seqs].
    const int K = ggml_get_op_params_i32(dst, 0);
    const bool keep_rs = K > 1;

    // recurrent state -> gdn_out tail (after attention scores), or the cache when fusing
    float * state_d           = dst_d + S_v * H * n_tokens * n_seqs;
    int64_t state_slot_stride = S_v * S_v * H * n_seqs;
    if (cache != nullptr) {
        state_d           = cache->data;
        state_slot_stride = cache->slot_stride;
    }

    if (kda) {
        if (keep_rs) {
            launch_gated_delta_net<true, true>(q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d,
                S_v, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3,
                sb1, sb2, sb3, neqk1, rq3, scale, state_slot_stride, K, stream);
        } else {
            launch_gated_delta_net<true, false>(q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d,
                S_v, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3,
                sb1, sb2, sb3, neqk1, rq3, scale, state_slot_stride, K, stream);
        }
    } else {
        // VITRIOL: chunked GDN (non-KDA only). The chunk kernel breaks the
        // serial token recurrence (MTP verify batches n_draft+1 tokens) via
        // the (I+A)D=R identity. Toggle: GGML_CUDA_GDN_SCAN=off|on|auto.
        if (gdn_scan_enabled() && gdn_scan_supported(n_tokens)) {
            if (keep_rs) {
                launch_gated_delta_net_chunk<true>(q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d,
                    S_v, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3,
                    sb1, sb2, sb3, neqk1, rq3, scale, state_slot_stride, K, stream);
            } else {
                launch_gated_delta_net_chunk<false>(q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d,
                    S_v, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3,
                    sb1, sb2, sb3, neqk1, rq3, scale, state_slot_stride, K, stream);
            }
        } else {
            if (keep_rs) {
                launch_gated_delta_net<false, true>(q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d,
                    S_v, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3,
                    sb1, sb2, sb3, neqk1, rq3, scale, state_slot_stride, K, stream);
            } else {
                launch_gated_delta_net<false, false>(q_d, k_d, v_d, g_d, b_d, s_d, dst_d, state_d,
                    S_v, H, n_tokens, n_seqs, sq1, sq2, sq3, sv1, sv2, sv3,
                    sb1, sb2, sb3, neqk1, rq3, scale, state_slot_stride, K, stream);
            }
        }
    }
}

void ggml_cuda_op_gated_delta_net(ggml_backend_cuda_context & ctx, ggml_tensor * dst) {
    ggml_cuda_op_gated_delta_net_impl(ctx, dst, nullptr);
}

void ggml_cuda_op_gated_delta_net_fused_cache(
        ggml_backend_cuda_context & ctx, ggml_tensor * dst, ggml_cuda_gated_delta_net_fused_cache cache) {
    ggml_cuda_op_gated_delta_net_impl(ctx, dst, &cache);
}
