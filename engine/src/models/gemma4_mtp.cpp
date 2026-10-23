#include "models.h"

// Stub — gemma4_mtp model is declared in VITRIOL but its full implementation
// requires newer upstream API features (layer.wq_s, layer.wo_s, etc.).
// This provides the vtable so the build succeeds. GEMMA4_MTP GGUFs do not
// currently exist; full support will be backported when needed.

void llama_model_gemma4_mtp::load_arch_hparams(llama_model_loader & ml) {
    GGML_ABORT("GEMMA4_MTP not yet supported in VITRIOL fork");
}

void llama_model_gemma4_mtp::load_arch_tensors(llama_model_loader & ml) {
    GGML_ABORT("GEMMA4_MTP not yet supported in VITRIOL fork");
}

std::unique_ptr<llm_graph_context> llama_model_gemma4_mtp::build_arch_graph(const llm_graph_params & params) const {
    GGML_ABORT("GEMMA4_MTP not yet supported in VITRIOL fork");
    return nullptr;
}

llama_model_gemma4_mtp::graph::graph(const llama_model & model, const llm_graph_params & params)
    : llm_graph_context(params) {
    GGML_ABORT("GEMMA4_MTP not yet supported in VITRIOL fork");
}
