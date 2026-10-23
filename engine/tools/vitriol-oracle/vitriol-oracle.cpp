// PROVENANCE: inspiration — OurobourOS bitnet-rs oracle harness (own work,
// Apache-2.0 OR MIT) and ggml-org/llama.cpp (MIT), cb_eval eval-callback
// pattern from the public llama_context_params API; differential-capture
// concept learned from T-MAC-era kernel parity workflows, implemented fresh.
//
// vitriol-oracle: capture every graph node of one decode step.
//
// Registers params.cb_eval, runs a single decode, and dumps each non-view
// node (name, type, shape, raw bytes) to <out>.bin with a JSONL index
// <out>.idx. Two captures (e.g. CPU vs CUDA, or build A vs build B) are
// compared by tools/vitriol-oracle/diff.py against the parity ladder
// (L0 byte-exact, L1 cos >= 0.999, L2 greedy-equal).
//
// Usage:
//   llama-vitriol-oracle -m model.gguf -p "prompt" [-ngl 99] [-o prefix]

#include "arg.h"
#include "common.h"
#include "log.h"
#include "sampling.h"
#include "llama.h"

#include <cstdio>
#include <cstring>
#include <fstream>
#include <string>
#include <vector>
#include <cstdlib>

namespace {

struct capture_state {
    std::ofstream data;
    std::ofstream index;
    uint64_t offset = 0;
    int nodes = 0;
    int seen = 0;    // graph order, counts every node visited
    int cur = -1;    // graph order of the node currently claimed
};

// ORACLE_IDX_LO / ORACLE_IDX_HI bound the capture window in graph order.
// Nodes outside the window are never claimed, so they never trigger a D2H.
// That keeps the run close to unserialized while still letting us diff a
// narrow slice of the graph.
static int g_idx_lo = 0;
static int g_idx_hi = 0x7fffffff;

bool capture_cb(struct ggml_tensor * t, bool ask, void * user_data) {
    auto * st = (capture_state *) user_data;
    if (ask) {
        st->cur = st->seen++;
        if (st->cur < g_idx_lo || st->cur >= g_idx_hi) {
            return false;
        }
        // Claim this node so the callback fires again with data present.
        return true;
    }
    if (!t || !t->data || t->view_src != nullptr) {
        return true; // skip views: their storage is captured at the source
    }

    const uint64_t nbytes = ggml_nbytes(t);
    std::vector<char> buf(nbytes);
    ggml_backend_tensor_get(t, buf.data(), 0, nbytes);
    st->data.write(buf.data(), nbytes);

    char row[2048];
    /* O1 probe: record the op and the src0 tensor + its buffer type so the
     * capture identifies exactly which op and which memory each node reads
     * (answers the gate/up placement question). */
    char src0desc[256] = "";
    if (t->src[0]) {
        const char * bn = "(none)";
        if (t->src[0]->buffer) {
            bn = ggml_backend_buffer_name(t->src[0]->buffer);
        }
        snprintf(src0desc, sizeof(src0desc), "%s|%s",
                 t->src[0]->name ? t->src[0]->name : "?", bn);
    }
    snprintf(row, sizeof(row),
             "{\"i\":%d,\"name\":\"%s\",\"op\":\"%s\",\"src0\":\"%s\",\"type\":\"%s\",\"ne\":[%lld,%lld,%lld,%lld],\"off\":%llu,\"n\":%llu}\n",
             st->cur, t->name, ggml_op_name(t->op), src0desc, ggml_type_name(t->type),
             (long long) t->ne[0], (long long) t->ne[1],
             (long long) t->ne[2], (long long) t->ne[3],
             (unsigned long long) st->offset, (unsigned long long) nbytes);
    st->index << row;

    st->offset += nbytes;
    st->nodes++;
    return true;
}

} // namespace

int main(int argc, char ** argv) {
    common_params params;
    params.n_predict = 1;
    if (const char * np = getenv("ORACLE_N_PREDICT")) params.n_predict = atoi(np);
    params.prompt = "The capital of France is";

    if (!common_params_parse(argc, argv, params, LLAMA_EXAMPLE_COMMON)) {
        return 1;
    }

    auto * st = new capture_state();
    const char * no_cb = getenv("ORACLE_NO_CB");
    if (!(no_cb && *no_cb)) params.cb_eval = capture_cb;
    params.cb_eval_user_data = st;

    if (const char * lo = getenv("ORACLE_IDX_LO")) g_idx_lo = atoi(lo);
    if (const char * hi = getenv("ORACLE_IDX_HI")) g_idx_hi = atoi(hi);

    LOG_INF("%s: capture start | model=%s n_gpu_layers=%d\n", __func__,
            params.model.path.c_str(), params.n_gpu_layers);

    llama_backend_init();
    llama_numa_init(params.numa);

    auto init = common_init_from_params(params);
    if (!init || !init->context()) {
        LOG_ERR("%s: failed to init model/context\n", __func__);
        return 1;
    }
    llama_context * ctx = init->context();

    const char * env_out = getenv("ORACLE_OUT");
    const std::string out_prefix = (env_out && *env_out) ? env_out : "oracle-capture";
    st->data.open(out_prefix + ".bin", std::ios::binary);
    st->index.open(out_prefix + ".idx");
    if (!st->data || !st->index) {
        LOG_ERR("%s: cannot open %s.bin/.idx\n", __func__, out_prefix.c_str());
        return 1;
    }
    // common_init_from_params() runs a warmup decode: its callback firings hit
    // the (open but unused) streams above with zero bytes; reset so the MEASURED
    // decode is captured from offset 0.
    st->offset = 0;
    st->nodes = 0;
    st->seen = 0;
    st->cur = -1;
    st->data.seekp(0);
    st->index.seekp(0);
    st->data.close();
    st->index.close();
    st->data.open(out_prefix + ".bin", std::ios::binary | std::ios::trunc);
    st->index.open(out_prefix + ".idx", std::ios::trunc);

    std::vector<llama_token> tokens = common_tokenize(ctx, params.prompt, true);
    llama_batch batch = llama_batch_get_one(tokens.data(), tokens.size());
    if (llama_decode(ctx, batch) != 0) {
        LOG_ERR("%s: decode failed\n", __func__);
        return 1;
    }
    llama_synchronize(ctx);

    llama_token tok = common_sampler_sample(init->sampler(0), ctx, -1);
    LOG_INF("%s: sampled token %d | nodes captured: %d | graph seen: %d | window: [%d,%d) | bytes: %llu\n",
            __func__, tok, st->nodes, st->seen, g_idx_lo, g_idx_hi,
            (unsigned long long) st->offset);

    // Continue decoding so the capture also covers n_tokens=1 decode-step
    // graphs - the request-2 collapse shows up there, not in the prefill.
    for (int step = 1; step < params.n_predict; ++step) {
        std::vector<llama_token> t1 = { tok };
        llama_batch b1 = llama_batch_get_one(t1.data(), 1);
        if (llama_decode(ctx, b1) != 0) {
            LOG_ERR("%s: decode step %d failed\n", __func__, step);
            return 1;
        }
        llama_synchronize(ctx);
        tok = common_sampler_sample(init->sampler(0), ctx, -1);
    }
    LOG_INF("%s: shot-1 decode steps done (%d)\n", __func__, params.n_predict - 1);

    st->data.close();
    st->index.close();

    // ORACLE_EXTRA_PROMPT: run a second, different-length prompt in the SAME
    // process (KV cleared between shots), capturing shot 2 separately. The
    // request-2 collapse reproduces only when a new graph shape follows an
    // earlier one; diffing shot 2 against a solo run of the same prompt
    // localizes the first corrupted node.
    const char * extra = getenv("ORACLE_EXTRA_PROMPT");
    if (extra && *extra) {
        const char * out2 = getenv("ORACLE_OUT2");
        const std::string p2 = (out2 && *out2) ? out2 : "oracle-capture2";
        st->data.open(p2 + ".bin", std::ios::binary | std::ios::trunc);
        st->index.open(p2 + ".idx", std::ios::trunc);
        if (!st->data || !st->index) {
            LOG_ERR("%s: cannot open %s.bin/.idx\n", __func__, p2.c_str());
            return 1;
        }
        st->offset = 0;
        st->nodes = 0;
        st->seen = 0;
        st->cur = -1;

        llama_memory_clear(llama_get_memory(ctx), false);
        std::vector<llama_token> tokens2 = common_tokenize(ctx, extra, true);
        llama_batch batch2 = llama_batch_get_one(tokens2.data(), tokens2.size());
        if (llama_decode(ctx, batch2) != 0) {
            LOG_ERR("%s: shot-2 decode failed\n", __func__);
            return 1;
        }
        llama_synchronize(ctx);
        llama_token tok2 = common_sampler_sample(init->sampler(0), ctx, -1);
        LOG_INF("%s: shot-2 sampled token %d | nodes captured: %d | graph seen: %d | bytes: %llu\n",
                __func__, tok2, st->nodes, st->seen, (unsigned long long) st->offset);
        for (int step = 1; step < params.n_predict; ++step) {
            std::vector<llama_token> t2 = { tok2 };
            llama_batch b2 = llama_batch_get_one(t2.data(), 1);
            if (llama_decode(ctx, b2) != 0) {
                LOG_ERR("%s: shot-2 decode step %d failed\n", __func__, step);
                return 1;
            }
            llama_synchronize(ctx);
            tok2 = common_sampler_sample(init->sampler(0), ctx, -1);
        }
        LOG_INF("%s: shot-2 decode steps done (%d)\n", __func__, params.n_predict - 1);
        st->data.close();
        st->index.close();
        LOG_INF("%s: wrote %s.bin and %s.idx\n", __func__, p2.c_str(), p2.c_str());
    }

    llama_backend_free();
    delete st;
    return 0;
}
