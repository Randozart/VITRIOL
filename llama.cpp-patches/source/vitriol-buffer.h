#pragma once

#include "ggml-backend.h"
#include "ggml-backend-impl.h"

#ifdef __cplusplus
extern "C" {
#endif

// Returns the VITRIOL buffer type for the given device.
// This buffer type allocates hugepage-backed, page-locked system RAM.
// Reports is_host=true — GPU reads expert weights over PCIe DMA.
GGML_API ggml_backend_buffer_type_t vitriol_get_buffer_type(int device);

// Returns true if the buffer type is VITRIOL.
GGML_API bool vitriol_is_vitriol_buffer_type(ggml_backend_buffer_type_t buft);

#ifdef __cplusplus
}
#endif
