/*
 * vitriol-sycl-buffer.hpp — VITRIOL SYCL buffer type + streaming API
 */

#ifndef VITRIOL_SYCL_BUFFER_HPP
#define VITRIOL_SYCL_BUFFER_HPP

#include "ggml-backend.h"
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

struct ggml_tensor;

/* Initialization */
void vitriol_sycl_init(void);
bool vitriol_sycl_is_enabled(void);
bool vitriol_sycl_verbose(void);
size_t vitriol_sycl_lru_mb(void);
bool vitriol_sycl_prefetch_enabled(void);
bool vitriol_sycl_async_dma(void);

/* Buffer type */
bool vitriol_sycl_is_vitriol_buffer_type(ggml_backend_buffer_type_t buft);
ggml_backend_buffer_type_t vitriol_sycl_get_buffer_type(int device);
ggml_backend_buffer_type_t * vitriol_sycl_get_extra_bufts(ggml_backend_dev_t dev);

/* Zero-copy mmap wrap: wrap a loader-owned host range as a VITRIOL buffer */
bool vitriol_sycl_buft_supports_host_ptr(ggml_backend_buffer_type_t buft);
ggml_backend_buffer_t vitriol_sycl_buffer_from_host_ptr(
    ggml_backend_dev_t dev, void * ptr, size_t size, size_t max_tensor_size);

/* LRU cache */
void * vitriol_sycl_lru_ensure(
    const void    *tensor_base,
    int            expert_idx,
    const void    *expert_data,
    size_t         expert_size);

void vitriol_sycl_lru_sync(void);

void vitriol_sycl_lru_prefetch(
    const void    *tensor_base,
    int            expert_idx,
    const void    *expert_data,
    size_t         expert_size);

/* Predictive prefetching */
void vitriol_sycl_predictor_prefetch(
    const void    *tensor_base,
    size_t         expert_size);

void vitriol_sycl_predictor_update(
    const void    *tensor_base,
    size_t         expert_size,
    const int32_t *expert_ids,
    int            n_experts);

/* E34: expert-usage profiler */
void vitriol_sycl_profile_init(void);
bool vitriol_sycl_profile_active(void);
void vitriol_sycl_profile_record(
    const void *tensor_base, const char *ids_host,
    size_t nb0, size_t nb1, int n_ids, int n_iid1, size_t expert_size);

/* E31: hot-expert profile + selective mlock */
void vitriol_sycl_load_hot_profile(const char *path);
bool vitriol_sycl_hot_profile_loaded(void);
void vitriol_sycl_mlock_hot_ranges(struct ggml_context *ctx);
void vitriol_sycl_preload_hot_pages(struct ggml_context *ctx);

/* Cleanup */
void vitriol_sycl_print_stats(void);
void vitriol_sycl_cleanup(void);

#ifdef __cplusplus
}
#endif

#endif /* VITRIOL_SYCL_BUFFER_HPP */
