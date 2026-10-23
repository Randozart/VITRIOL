#include "server-task.h"

#include "build-info.h"
#include "server-chat.h"
#include "chat.h"
#include "common.h"
#include "json-schema-to-grammar.h"
#include "llama.h"
#include "sampling.h"
#include "speculative.h"
#include "server-common.h"

#include <sstream>
#include <algorithm>
#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <dirent.h>
#include <thread>
#include <sys/stat.h>
#include <sys/time.h>

//
// task_params
//

json task_params::format_logit_bias(const std::vector<llama_logit_bias> & logit_bias) const {
    json data = json::array();
    for (const auto & lb : logit_bias) {
        data.push_back(json{
            {"bias", lb.bias},
            {"token", lb.token},
        });
    }
    return data;
}

json task_params::to_json(bool only_metrics) const {
    std::vector<std::string> samplers;
    samplers.reserve(sampling.samplers.size());
    for (const auto & sampler : sampling.samplers) {
        samplers.emplace_back(common_sampler_type_to_str(sampler));
    }

    json lora = json::array();
    for (auto & it : this->lora) {
        lora.push_back({{"id", it.first}, {"scale", it.second}});
    }

    if (only_metrics) {
        return json {
            {"seed",                      sampling.seed},
            {"temperature",               sampling.temp},
            {"dynatemp_range",            sampling.dynatemp_range},
            {"dynatemp_exponent",         sampling.dynatemp_exponent},
            {"top_k",                     sampling.top_k},
            {"top_p",                     sampling.top_p},
            {"min_p",                     sampling.min_p},
            {"top_n_sigma",               sampling.top_n_sigma},
            {"xtc_probability",           sampling.xtc_probability},
            {"xtc_threshold",             sampling.xtc_threshold},
            {"typical_p",                 sampling.typ_p},
            {"repeat_last_n",             sampling.penalty_last_n},
            {"repeat_penalty",            sampling.penalty_repeat},
            {"presence_penalty",          sampling.penalty_present},
            {"frequency_penalty",         sampling.penalty_freq},
            {"dry_multiplier",            sampling.dry_multiplier},
            {"dry_base",                  sampling.dry_base},
            {"dry_allowed_length",        sampling.dry_allowed_length},
            {"dry_penalty_last_n",        sampling.dry_penalty_last_n},
            {"mirostat",                  sampling.mirostat},
            {"mirostat_tau",              sampling.mirostat_tau},
            {"mirostat_eta",              sampling.mirostat_eta},
            {"adaptive_target",           sampling.adaptive_target},
            {"adaptive_decay",            sampling.adaptive_decay},
            {"max_tokens",                n_predict},
            {"n_predict",                 n_predict}, // TODO: deduplicate?
            {"n_keep",                    n_keep},
            {"n_discard",                 n_discard},
            {"ignore_eos",                sampling.ignore_eos},
            {"stream",                    stream},
            {"n_probs",                   sampling.n_probs},
            {"min_keep",                  sampling.min_keep},
            {"chat_format",               common_chat_format_name(chat_parser_params.format)},
            {"reasoning_format",          common_reasoning_format_name(chat_parser_params.reasoning_format)},
            {"reasoning_in_content",      chat_parser_params.reasoning_in_content},
            {"generation_prompt",         chat_parser_params.generation_prompt},
            {"samplers",                  samplers},
            {"speculative.types",         common_speculative_type_name_str(speculative.types)},
            {"timings_per_token",         timings_per_token},
            {"post_sampling_probs",       post_sampling_probs},
            {"backend_sampling",          sampling.backend_sampling},
            {"lora",                      lora},
        };
    }

    auto grammar_triggers = json::array();
    for (const auto & trigger : sampling.grammar_triggers) {
        server_grammar_trigger ct(trigger);
        grammar_triggers.push_back(ct.to_json());
    }

    return json {
        {"seed",                      sampling.seed},
        {"temperature",               sampling.temp},
        {"dynatemp_range",            sampling.dynatemp_range},
        {"dynatemp_exponent",         sampling.dynatemp_exponent},
        {"top_k",                     sampling.top_k},
        {"top_p",                     sampling.top_p},
        {"min_p",                     sampling.min_p},
        {"top_n_sigma",               sampling.top_n_sigma},
        {"xtc_probability",           sampling.xtc_probability},
        {"xtc_threshold",             sampling.xtc_threshold},
        {"typical_p",                 sampling.typ_p},
        {"repeat_last_n",             sampling.penalty_last_n},
        {"repeat_penalty",            sampling.penalty_repeat},
        {"presence_penalty",          sampling.penalty_present},
        {"frequency_penalty",         sampling.penalty_freq},
        {"dry_multiplier",            sampling.dry_multiplier},
        {"dry_base",                  sampling.dry_base},
        {"dry_allowed_length",        sampling.dry_allowed_length},
        {"dry_penalty_last_n",        sampling.dry_penalty_last_n},
        {"dry_sequence_breakers",     sampling.dry_sequence_breakers},
        {"mirostat",                  sampling.mirostat},
        {"mirostat_tau",              sampling.mirostat_tau},
        {"mirostat_eta",              sampling.mirostat_eta},
        {"adaptive_target",           sampling.adaptive_target},
        {"adaptive_decay",            sampling.adaptive_decay},
        {"stop",                      antiprompt},
        {"max_tokens",                n_predict},
        {"n_predict",                 n_predict}, // TODO: deduplicate?
        {"n_keep",                    n_keep},
        {"n_discard",                 n_discard},
        {"ignore_eos",                sampling.ignore_eos},
        {"stream",                    stream},
        {"logit_bias",                format_logit_bias(sampling.logit_bias)},
        {"n_probs",                   sampling.n_probs},
        {"min_keep",                  sampling.min_keep},
        {"grammar",                   common_grammar_value(sampling.grammar)},
        {"grammar_lazy",              sampling.grammar_lazy},
        {"grammar_triggers",          grammar_triggers},
        {"preserved_tokens",          sampling.preserved_tokens},
        {"chat_format",               common_chat_format_name(chat_parser_params.format)},
        {"reasoning_format",          common_reasoning_format_name(chat_parser_params.reasoning_format)},
        {"reasoning_in_content",      chat_parser_params.reasoning_in_content},
        {"generation_prompt",         chat_parser_params.generation_prompt},
        {"samplers",                  samplers},
        {"speculative.types",         common_speculative_type_name_str(speculative.types)},
        {"timings_per_token",         timings_per_token},
        {"post_sampling_probs",       post_sampling_probs},
        {"backend_sampling",          sampling.backend_sampling},
        {"lora",                      lora},
    };
}

//
// task_result_state
//
task_result_state::task_result_state(const common_chat_parser_params & chat_parser_params)
    : chat_parser_params(chat_parser_params)
    , oai_resp_id("resp_" + random_string())
    , oai_resp_reasoning_id("rs_" + random_string())
    , oai_resp_message_id("msg_" + random_string()) {
    if (chat_parser_params.is_continuation && !chat_parser_params.echo) {
        // initialize chat_msg to avoid emitting a delta containing the assistant prefill
        chat_msg = common_chat_parse("", true, chat_parser_params);
    }
}

common_chat_msg task_result_state::update_chat_msg(
        const std::string & text_added,
        bool is_partial,
        std::vector<common_chat_msg_diff> & diffs,
        bool filter_tool_calls) {
    generated_text += text_added;
    auto msg_prv_copy = chat_msg;
    //SRV_DBG("Parsing chat message: %s\n", generated_text.c_str());
    auto new_msg = common_chat_parse(
        generated_text,
        is_partial,
        chat_parser_params);
    if (!new_msg.empty()) {
        new_msg.set_tool_call_ids(generated_tool_call_ids, gen_tool_call_id);
        chat_msg = new_msg;
        auto all_diffs = common_chat_msg_diff::compute_diffs(msg_prv_copy, chat_msg);

        if (!filter_tool_calls) {
            diffs = std::move(all_diffs);
        } else {
            for (auto & d : all_diffs) {
                // If this is a new type of delta, flush all currently pending tool call names
                for (size_t i = 0; i < chat_msg.tool_calls.size(); ++i) {
                    if (sent_tool_call_names.count(i) || chat_msg.tool_calls[i].name.empty()) {
                        continue;
                    }
                    if (d.tool_call_index != i || !d.tool_call_delta.arguments.empty()) {
                        common_chat_msg_diff header;
                        header.tool_call_index      = i;
                        header.tool_call_delta.id   = chat_msg.tool_calls[i].id;
                        header.tool_call_delta.name = chat_msg.tool_calls[i].name;
                        diffs.push_back(std::move(header));
                        sent_tool_call_names.insert(i);
                    }
                }

                if (d.tool_call_index == std::string::npos) {
                    diffs.push_back(std::move(d));
                } else {
                    size_t i = d.tool_call_index;
                    if (sent_tool_call_names.count(i)) {
                        if (!d.tool_call_delta.arguments.empty()) {
                            d.tool_call_delta.name = "";
                            d.tool_call_delta.id   = "";
                            diffs.push_back(std::move(d));
                        }
                    } else {
                        // Not sent yet.
                        if (!d.tool_call_delta.arguments.empty() || !is_partial) {
                            d.tool_call_delta.name = chat_msg.tool_calls[i].name;
                            d.tool_call_delta.id   = chat_msg.tool_calls[i].id;
                            diffs.push_back(std::move(d));
                            sent_tool_call_names.insert(i);
                        } else {
                            // Suppress
                        }
                    }
                }
            }
            // Final check at EOF
            if (!is_partial) {
                for (size_t i = 0; i < chat_msg.tool_calls.size(); ++i) {
                    if (!sent_tool_call_names.count(i) && !chat_msg.tool_calls[i].name.empty()) {
                        common_chat_msg_diff header;
                        header.tool_call_index      = i;
                        header.tool_call_delta.id   = chat_msg.tool_calls[i].id;
                        header.tool_call_delta.name = chat_msg.tool_calls[i].name;
                        diffs.push_back(std::move(header));
                        sent_tool_call_names.insert(i);
                    }
                }
            }
        }
    }
    return chat_msg;
}

//
// result_prompt_progress
//
json result_prompt_progress::to_json() const {
    return json {
        {"total",     total},
        {"cache",     cache},
        {"processed", processed},
        {"time_ms",   time_ms},
    };
}

static inline std::string stop_type_to_str(stop_type type) {
    switch (type) {
        case STOP_TYPE_EOS:   return "eos";
        case STOP_TYPE_WORD:  return "word";
        case STOP_TYPE_LIMIT: return "limit";
        default:              return "none";
    }
}

//
// completion_token_output
//

json completion_token_output::to_json(bool post_sampling_probs) const {
    json probs_for_token = json::array();
    for (const auto & p : probs) {
        std::string txt(p.txt);
        txt.resize(validate_utf8(txt));
        probs_for_token.push_back(json {
            {"id",      p.tok},
            {"token",   txt},
            {"bytes",   str_to_bytes(p.txt)},
            {
                post_sampling_probs ? "prob" : "logprob",
                post_sampling_probs ? p.prob : logarithm(p.prob)
            },
        });
    }
    return probs_for_token;
}

json completion_token_output::probs_vector_to_json(const std::vector<completion_token_output> & probs, bool post_sampling_probs) {
    json out = json::array();
    for (const auto & p : probs) {
        std::string txt(p.text_to_send);
        txt.resize(validate_utf8(txt));
        out.push_back(json {
            {"id",           p.tok},
            {"token",        txt},
            {"bytes",        str_to_bytes(p.text_to_send)},
            {
                post_sampling_probs ? "prob" : "logprob",
                post_sampling_probs ? p.prob : logarithm(p.prob)
            },
            {
                post_sampling_probs ? "top_probs" : "top_logprobs",
                p.to_json(post_sampling_probs)
            },
        });
    }
    return out;
}

float completion_token_output::logarithm(float x) {
    // the JSON library converts -inf to null, so we need to prevent that
    return x == 0.0f ? std::numeric_limits<float>::lowest() : std::log(x);
}

std::vector<unsigned char> completion_token_output::str_to_bytes(const std::string & str) {
    std::vector<unsigned char> bytes;
    for (unsigned char c : str) {
        bytes.push_back(c);
    }
    return bytes;
}

//
// server_task_result_cmpl_final
//
json server_task_result_cmpl_final::to_json() {
    GGML_ASSERT(is_updated && "update() must be called before to_json()");
    switch (res_type) {
        case TASK_RESPONSE_TYPE_NONE:
            return to_json_non_oaicompat();
        case TASK_RESPONSE_TYPE_OAI_CMPL:
            return to_json_oaicompat();
        case TASK_RESPONSE_TYPE_OAI_CHAT:
            return stream ? to_json_oaicompat_chat_stream() : to_json_oaicompat_chat();
        case TASK_RESPONSE_TYPE_OAI_RESP:
            return stream ? to_json_oaicompat_resp_stream() : to_json_oaicompat_resp();
        case TASK_RESPONSE_TYPE_OAI_ASR:
            return to_json_oaicompat_asr();
        case TASK_RESPONSE_TYPE_ANTHROPIC:
            return stream ? to_json_anthropic_stream() : to_json_anthropic();
        default:
            GGML_ASSERT(false && "Invalid task_response_type");
    }
}

json server_task_result_cmpl_final::to_json_non_oaicompat() {
    json res = json {
        {"index",               index},
        {"content",             content},
        {"tokens",              tokens},
        {"id_slot",             id_slot},
        {"stop",                true},
        {"model",               oaicompat_model},
        {"tokens_predicted",    n_decoded},
        {"tokens_evaluated",    n_prompt_tokens},
        {"generation_settings", generation_params.to_json()},
        {"prompt",              prompt},
        {"has_new_line",        has_new_line},
        {"truncated",           truncated},
        {"stop_type",           stop_type_to_str(stop)},
        {"stopping_word",       stopping_word},
        {"tokens_cached",       n_tokens_cached},
        {"timings",             stats.to_json()},
    };
    if (!stream && !probs_output.empty()) {
        res["completion_probabilities"] = completion_token_output::probs_vector_to_json(probs_output, post_sampling_probs);
    }
    return response_fields.empty() ? res : json_get_nested_values(response_fields, res);
}

json server_task_result_cmpl_final::usage_json_oaicompat() {
    return json {
        {"completion_tokens", n_decoded},
        {"prompt_tokens",     n_prompt_tokens},
        {"total_tokens",      n_decoded + n_prompt_tokens},
        {"prompt_tokens_details", json { {"cached_tokens", n_prompt_tokens_cache} }},
    };
}

json server_task_result_cmpl_final::to_json_oaicompat() {
    std::time_t t = std::time(0);
    json logprobs = json(nullptr); // OAI default to null
    if (!stream && probs_output.size() > 0) {
        logprobs = json{
            {"content", completion_token_output::probs_vector_to_json(probs_output, post_sampling_probs)},
        };
    }
    json finish_reason = "length";
    if (stop == STOP_TYPE_WORD || stop == STOP_TYPE_EOS) {
        finish_reason = "stop";
    }
    json res = json {
        {"choices",            json::array({
            json{
                {"text",          content},
                {"index",         index},
                {"logprobs",      logprobs},
                {"finish_reason", finish_reason},
            }
        })},
        {"created",            t},
        {"model",              oaicompat_model},
        {"system_fingerprint", std::string(llama_build_info())},
        {"object",             "text_completion"},
        {"usage",              usage_json_oaicompat()},
        {"id", oaicompat_cmpl_id}
    };

    // extra fields for debugging purposes
    if (verbose) {
        res["__verbose"] = to_json_non_oaicompat();
    }
    if (stats.is_set()) {
        res["timings"] = stats.to_json();
    }

    return res;
}

json server_task_result_cmpl_final::to_json_oaicompat_chat() {
    std::string finish_reason = "length";
    common_chat_msg msg;
    if (!oaicompat_msg.empty()) {
        msg = oaicompat_msg;
    } else {
        msg.role = "assistant";
        msg.content = content;
    }
    if (stop == STOP_TYPE_WORD || stop == STOP_TYPE_EOS) {
        finish_reason = msg.tool_calls.empty() ? "stop" : "tool_calls";
    }

    json choice {
        {"finish_reason", finish_reason},
        {"index", index},
        {"message", msg.to_json_oaicompat()},
    };

    if (!stream && probs_output.size() > 0) {
        choice["logprobs"] = json{
            {"content", completion_token_output::probs_vector_to_json(probs_output, post_sampling_probs)},
        };
    }

    std::time_t t = std::time(0);

    json res = json {
        {"choices",            json::array({choice})},
        {"created",            t},
        {"model",              oaicompat_model},
        {"system_fingerprint", std::string(llama_build_info())},
        {"object",             "chat.completion"},
        {"usage",              usage_json_oaicompat()},
        {"id", oaicompat_cmpl_id}
    };

    // extra fields for debugging purposes
    if (verbose) {
        res["__verbose"] = to_json_non_oaicompat();
    }
    if (stats.is_set()) {
        res["timings"] = stats.to_json();
    }

    return res;
}

json server_task_result_cmpl_final::to_json_oaicompat_chat_stream() {
    std::time_t t = std::time(0);
    std::string finish_reason = "length";
    if (stop == STOP_TYPE_WORD || stop == STOP_TYPE_EOS) {
        finish_reason = oaicompat_msg.tool_calls.empty() ? "stop" : "tool_calls";
    }

    json deltas = json::array();
    for (const auto & diff : oaicompat_msg_diffs) {
        deltas.push_back({
            {"choices", json::array({
                json {
                    {"finish_reason", nullptr},
                    {"index", index},
                    {"delta", server_chat_msg_diff_to_json_oaicompat(diff)},
                },
            })},
            {"created", t},
            {"id", oaicompat_cmpl_id},
            {"model", oaicompat_model},
            {"system_fingerprint", std::string(llama_build_info())},
            {"object", "chat.completion.chunk"},
        });
    }

    deltas.push_back({
        {"choices", json::array({
            json {
                {"finish_reason", finish_reason},
                {"index", index},
                {"delta", json::object()},
            },
        })},
        {"created",            t},
        {"id",                 oaicompat_cmpl_id},
        {"model",              oaicompat_model},
        {"system_fingerprint", std::string(llama_build_info())},
        {"object",             "chat.completion.chunk"},
    });

    if (include_usage) {
        // OpenAI API spec for chat.completion.chunks specifies an empty `choices` array for the last chunk when including usage
        // https://platform.openai.com/docs/api-reference/chat_streaming/streaming#chat_streaming/streaming-choices
        deltas.push_back({
            {"choices", json::array()},
            {"created",            t},
            {"id",                 oaicompat_cmpl_id},
            {"model",              oaicompat_model},
            {"system_fingerprint", std::string(llama_build_info())},
            {"object",             "chat.completion.chunk"},
            {"usage",              usage_json_oaicompat()},
        });
    }

    if (stats.is_set()) {
        deltas.back()["timings"] = stats.to_json();
    }

    // extra fields for debugging purposes
    if (verbose && !deltas.empty()) {
        deltas.front()["__verbose"] = to_json_non_oaicompat();
    }

    return deltas;
}

json server_task_result_cmpl_final::to_json_oaicompat_resp() {
    common_chat_msg msg;
    if (!oaicompat_msg.empty()) {
        msg = oaicompat_msg;
    } else {
        msg.role = "assistant";
        msg.content = content;
    }

    std::vector<json> output;

    if (msg.reasoning_content != "") {
        output.push_back(json {
            {"id",      "rs_" + random_string()},
            {"summary", json::array()},
            {"type",    "reasoning"},
            {"content", json::array({ json {
                {"text", msg.reasoning_content},
                {"type", "reasoning_text"},
            }})},
            {"encrypted_content", ""},
            {"status",            "completed"},
        });
    }

    if (msg.content != "") {
        output.push_back(json {
            {"content", json::array({ json {
                {"type",        "output_text"},
                {"annotations", json::array()},
                {"logprobs",    json::array()},
                {"text",        msg.content},
            }})},
            {"id",     "msg_" + random_string()},
            {"role",   msg.role},
            {"status", "completed"},
            {"type",   "message"},
        });
    }

    for (const common_chat_tool_call & tool_call : oaicompat_msg.tool_calls) {
        output.push_back(json {
            {"id",        "fc_" + tool_call.id},
            {"type",      "function_call"},
            {"status",    "completed"},
            {"arguments", tool_call.arguments},
            {"call_id",   "call_" + tool_call.id},
            {"name",      tool_call.name},
        });
    }

    std::time_t t = std::time(0);
    json res = {
        {"completed_at", t},
        {"created_at",   t},
        {"id",           oai_resp_id},
        {"model",        oaicompat_model},
        {"object",       "response"},
        {"output",       output},
        {"status",       "completed"},
        {"usage",        json {
            {"input_tokens",  n_prompt_tokens},
            {"output_tokens", n_decoded},
            {"total_tokens",  n_decoded + n_prompt_tokens},
            {"input_tokens_details", json { {"cached_tokens", n_prompt_tokens_cache} }},
        }},
    };

    return res;
}

json server_task_result_cmpl_final::to_json_oaicompat_resp_stream() {
    std::vector<json> server_sent_events;
    std::vector<json> output;

    if (oaicompat_msg.reasoning_content != "") {
        const json output_item = json {
            {"id",      oai_resp_reasoning_id},
            {"summary", json::array()},
            {"type",    "reasoning"},
            {"content", json::array({ json {
                {"text", oaicompat_msg.reasoning_content},
                {"type", "reasoning_text"},
            }})},
            {"encrypted_content", ""},
        };

        server_sent_events.push_back(json {
            {"event", "response.output_item.done"},
            {"data", json {
                {"type", "response.output_item.done"},
                {"item", output_item}
            }}
        });
        output.push_back(output_item);
    }

    if (oaicompat_msg.content != "") {
        server_sent_events.push_back(json {
            {"event", "response.output_text.done"},
            {"data", json {
                {"type",    "response.output_text.done"},
                {"item_id", oai_resp_message_id},
                {"text",    oaicompat_msg.content}
            }}
        });

        const json content_part = {
            {"type",        "output_text"},
            {"annotations", json::array()},
            {"logprobs",    json::array()},
            {"text",        oaicompat_msg.content}
        };

        server_sent_events.push_back(json {
            {"event", "response.content_part.done"},
            {"data", json {
                {"type",    "response.content_part.done"},
                {"item_id", oai_resp_message_id},
                {"part",    content_part}
            }}
        });
        const json output_item = {
            {"type",    "message"},
            {"status",  "completed"},
            {"id",      oai_resp_message_id},
            {"content", json::array({content_part})},
            {"role",    "assistant"}
        };

        server_sent_events.push_back(json {
            {"event", "response.output_item.done"},
            {"data", json {
                {"type", "response.output_item.done"},
                {"item", output_item}
            }}
        });
        output.push_back(output_item);
    }

    for (const common_chat_tool_call & tool_call : oaicompat_msg.tool_calls) {
        const json output_item = {
            {"id",        "fc_" + tool_call.id},
            {"type",      "function_call"},
            {"status",    "completed"},
            {"arguments", tool_call.arguments},
            {"call_id",   "call_" + tool_call.id},
            {"name",      tool_call.name}
        };
        server_sent_events.push_back(json {
            {"event", "response.output_item.done"},
            {"data", json {
                {"type", "response.output_item.done"},
                {"item", output_item}
            }}
        });
        output.push_back(output_item);
    }

    std::time_t t = std::time(0);
    server_sent_events.push_back(json {
        {"event", "response.completed"},
        {"data", json {
            {"type", "response.completed"},
            {"response", json {
                {"id",         oai_resp_id},
                {"object",     "response"},
                {"created_at", t},
                {"status",     "completed"},
                {"model",      oaicompat_model},
                {"output",     output},
                {"usage",      json {
                    {"input_tokens",  n_prompt_tokens},
                    {"output_tokens", n_decoded},
                    {"total_tokens",  n_decoded + n_prompt_tokens},
                    {"input_tokens_details", json { {"cached_tokens", n_prompt_tokens_cache} }},
                }}
            }},
        }}
    });

    if (stats.is_set()) {
        server_sent_events.back().at("data")["timings"] = stats.to_json();
    }

    return server_sent_events;
}

json server_task_result_cmpl_final::to_json_oaicompat_asr() {
    json event = json {
        {"type",  "transcript.text.done"},
        {"text",  oaicompat_msg.content},
        {"usage", json {
            {"type",         "tokens"},
            {"input_tokens",  n_prompt_tokens},
            {"output_tokens", n_decoded},
            {"total_tokens",  n_decoded + n_prompt_tokens},
            {"input_tokens_details", json { {"cached_tokens", n_prompt_tokens_cache} }},
        }},
    };
    return event;
}

json server_task_result_cmpl_final::to_json_anthropic() {
    std::string stop_reason = "max_tokens";
    if (stop == STOP_TYPE_WORD || stop == STOP_TYPE_EOS) {
        stop_reason = oaicompat_msg.tool_calls.empty() ? "end_turn" : "tool_use";
    }

    json content_blocks = json::array();

    common_chat_msg msg;
    if (!oaicompat_msg.empty()) {
        msg = oaicompat_msg;
    } else {
        msg.role = "assistant";
        msg.content = content;
    }

    // thinking block comes first (Anthropic extended thinking format)
    if (!msg.reasoning_content.empty()) {
        content_blocks.push_back({
            {"type", "thinking"},
            {"thinking", msg.reasoning_content},
            {"signature", ""}  // empty signature for local models (no cryptographic verification)
        });
    }

    if (!msg.content.empty()) {
        content_blocks.push_back({
            {"type", "text"},
            {"text", msg.content}
        });
    }

    for (const auto & tool_call : msg.tool_calls) {
        json tool_use_block = {
            {"type", "tool_use"},
            {"id", tool_call.id},
            {"name", tool_call.name}
        };

        try {
            tool_use_block["input"] = json::parse(tool_call.arguments);
        } catch (const std::exception &) {
            tool_use_block["input"] = json::object();
        }

        content_blocks.push_back(tool_use_block);
    }

    json res = {
        {"id", oaicompat_cmpl_id},
        {"type", "message"},
        {"role", "assistant"},
        {"content", content_blocks},
        {"model", oaicompat_model},
        {"stop_reason", stop_reason},
        {"stop_sequence", stopping_word.empty() ? nullptr : json(stopping_word)},
        {"usage", {
            {"cache_read_input_tokens", n_prompt_tokens_cache},
            {"input_tokens", n_prompt_tokens - n_prompt_tokens_cache},
            {"output_tokens", n_decoded}
        }}
    };

    return res;
}

json server_task_result_cmpl_final::to_json_anthropic_stream() {
    json events = json::array();

    std::string stop_reason = "max_tokens";
    if (stop == STOP_TYPE_WORD || stop == STOP_TYPE_EOS) {
        stop_reason = oaicompat_msg.tool_calls.empty() ? "end_turn" : "tool_use";
    }

    bool has_thinking = !oaicompat_msg.reasoning_content.empty();
    bool has_text     = !oaicompat_msg.content.empty();
    size_t num_tool_calls = oaicompat_msg.tool_calls.size();

    // content block indices: thinking (0) -> text (0 or 1) -> tool_use (n+)
    size_t thinking_block_index = 0;
    size_t text_block_index     = has_thinking ? 1 : 0;

    bool thinking_block_started = false;
    bool text_block_started     = false;
    std::unordered_set<size_t> tool_calls_started;

    for (const auto & diff : oaicompat_msg_diffs) {
        // handle thinking/reasoning content
        if (!diff.reasoning_content_delta.empty()) {
            if (!thinking_block_started) {
                events.push_back({
                    {"event", "content_block_start"},
                    {"data", {
                        {"type", "content_block_start"},
                        {"index", thinking_block_index},
                        {"content_block", {
                            {"type", "thinking"},
                            {"thinking", ""}
                        }}
                    }}
                });
                thinking_block_started = true;
            }

            events.push_back({
                {"event", "content_block_delta"},
                {"data", {
                    {"type", "content_block_delta"},
                    {"index", thinking_block_index},
                    {"delta", {
                        {"type", "thinking_delta"},
                        {"thinking", diff.reasoning_content_delta}
                    }}
                }}
            });
        }

        // handle regular text content
        if (!diff.content_delta.empty()) {
            if (!text_block_started) {
                events.push_back({
                    {"event", "content_block_start"},
                    {"data", {
                        {"type", "content_block_start"},
                        {"index", text_block_index},
                        {"content_block", {
                            {"type", "text"},
                            {"text", ""}
                        }}
                    }}
                });
                text_block_started = true;
            }

            events.push_back({
                {"event", "content_block_delta"},
                {"data", {
                    {"type", "content_block_delta"},
                    {"index", text_block_index},
                    {"delta", {
                        {"type", "text_delta"},
                        {"text", diff.content_delta}
                    }}
                }}
            });
        }

        // handle tool calls
        if (diff.tool_call_index != std::string::npos) {
            size_t content_block_index = (has_thinking ? 1 : 0) + (has_text ? 1 : 0) + diff.tool_call_index;

            if (tool_calls_started.find(diff.tool_call_index) == tool_calls_started.end()) {
                const auto & full_tool_call = oaicompat_msg.tool_calls[diff.tool_call_index];

                events.push_back({
                    {"event", "content_block_start"},
                    {"data", {
                        {"type", "content_block_start"},
                        {"index", content_block_index},
                        {"content_block", {
                            {"type", "tool_use"},
                            {"id", full_tool_call.id},
                            {"name", full_tool_call.name}
                        }}
                    }}
                });
                tool_calls_started.insert(diff.tool_call_index);
            }

            if (!diff.tool_call_delta.arguments.empty()) {
                events.push_back({
                    {"event", "content_block_delta"},
                    {"data", {
                        {"type", "content_block_delta"},
                        {"index", content_block_index},
                        {"delta", {
                            {"type", "input_json_delta"},
                            {"partial_json", diff.tool_call_delta.arguments}
                        }}
                    }}
                });
            }
        }
    }

    // close content blocks in order
    if (has_thinking) {
        // Anthropic API requires a signature_delta before closing thinking blocks
        // We use an empty signature since we can't generate a cryptographic signature for local models
        events.push_back({
            {"event", "content_block_delta"},
            {"data", {
                {"type", "content_block_delta"},
                {"index", thinking_block_index},
                {"delta", {
                    {"type", "signature_delta"},
                    {"signature", ""}
                }}
            }}
        });
        events.push_back({
            {"event", "content_block_stop"},
            {"data", {
                {"type", "content_block_stop"},
                {"index", thinking_block_index}
            }}
        });
    }

    if (has_text) {
        events.push_back({
            {"event", "content_block_stop"},
            {"data", {
                {"type", "content_block_stop"},
                {"index", text_block_index}
            }}
        });
    }

    for (size_t i = 0; i < num_tool_calls; i++) {
        size_t content_block_index = (has_thinking ? 1 : 0) + (has_text ? 1 : 0) + i;
        events.push_back({
            {"event", "content_block_stop"},
            {"data", {
                {"type", "content_block_stop"},
                {"index", content_block_index}
            }}
        });
    }

    events.push_back({
        {"event", "message_delta"},
        {"data", {
            {"type", "message_delta"},
            {"delta", {
                {"stop_reason", stop_reason},
                {"stop_sequence", stopping_word.empty() ? nullptr : json(stopping_word)}
            }},
            {"usage", {
                {"output_tokens", n_decoded}
            }}
        }}
    });

    events.push_back({
        {"event", "message_stop"},
        {"data", {
            {"type", "message_stop"}
        }}
    });

    return events;
}

//
// server_task_result_cmpl_partial
//
void server_task_result_cmpl_partial::update(task_result_state & state) {
    is_updated = true;
    if (is_begin) {
        return; // begin marker only flushes headers, skip parsing
    }
    state.update_chat_msg(content, true, oaicompat_msg_diffs);

    // Copy current state for use in to_json_*() (reflects state BEFORE this chunk)
    thinking_block_started = state.thinking_block_started;
    text_block_started     = state.text_block_started;

    oai_resp_created       = state.oai_resp_created;
    oai_resp_id            = state.oai_resp_id;
    oai_resp_reasoning_id  = state.oai_resp_reasoning_id;
    oai_resp_message_id    = state.oai_resp_message_id;
    oai_resp_fc_id         = state.oai_resp_fc_id;

    // track if the accumulated message has any reasoning content
    anthropic_has_reasoning = !state.chat_msg.reasoning_content.empty();

    if (res_type == TASK_RESPONSE_TYPE_OAI_RESP && !state.oai_resp_created && (is_progress || n_decoded == 1)) {
        state.oai_resp_created = true;
    }

    // Pre-compute state updates based on diffs (for next chunk)
    for (const common_chat_msg_diff & diff : oaicompat_msg_diffs) {
        if (!diff.reasoning_content_delta.empty() && !state.thinking_block_started) {
            state.thinking_block_started = true;
        }
        if (!diff.content_delta.empty() && !state.text_block_started) {
            state.text_block_started = true;
        }
        if (!diff.tool_call_delta.name.empty()) {
            state.oai_resp_fc_id = diff.tool_call_delta.id;
        }
    }
}

json server_task_result_cmpl_partial::to_json() {
    GGML_ASSERT(is_updated && "update() must be called before to_json()");
    if (is_begin) {
        return nullptr; // simply signal to HTTP handler to send the headers and status code
    }
    switch (res_type) {
        case TASK_RESPONSE_TYPE_NONE:
            return to_json_non_oaicompat();
        case TASK_RESPONSE_TYPE_OAI_CMPL:
            return to_json_oaicompat();
        case TASK_RESPONSE_TYPE_OAI_CHAT:
            return to_json_oaicompat_chat();
        case TASK_RESPONSE_TYPE_OAI_RESP:
            return to_json_oaicompat_resp();
        case TASK_RESPONSE_TYPE_OAI_ASR:
            return to_json_oaicompat_asr();
        case TASK_RESPONSE_TYPE_ANTHROPIC:
            return to_json_anthropic();
        default:
            GGML_ASSERT(false && "Invalid task_response_type");
    }
}

json server_task_result_cmpl_partial::to_json_non_oaicompat() {
    // non-OAI-compat JSON
    json res = json {
        {"index",            index},
        {"content",          content},
        {"tokens",           tokens},
        {"stop",             false},
        {"id_slot",          id_slot},
        {"tokens_predicted", n_decoded},
        {"tokens_evaluated", n_prompt_tokens},
    };
    // populate the timings object when needed (usually for the last response or with timings_per_token enabled)
    if (stats.is_set()) {
        res["timings"] = stats.to_json();
    }
    if (is_progress) {
        res["prompt_progress"] = progress.to_json();
    }
    if (!prob_output.probs.empty()) {
        res["completion_probabilities"] = completion_token_output::probs_vector_to_json({prob_output}, post_sampling_probs);
    }
    return res;
}

json server_task_result_cmpl_partial::to_json_oaicompat() {
    std::time_t t = std::time(0);
    json logprobs = json(nullptr); // OAI default to null
    if (prob_output.probs.size() > 0) {
        logprobs = json{
            {"content", completion_token_output::probs_vector_to_json({prob_output}, post_sampling_probs)},
        };
    }
    json res = json {
        {"choices",            json::array({
            json{
                {"text",          content},
                {"index",         index},
                {"logprobs",      logprobs},
                {"finish_reason", nullptr},
            }
        })},
        {"created",            t},
        {"model",              oaicompat_model},
        {"system_fingerprint", std::string(llama_build_info())},
        {"object",             "text_completion"},
        {"id",                 oaicompat_cmpl_id}
    };

    // extra fields for debugging purposes
    if (verbose) {
        res["__verbose"] = to_json_non_oaicompat();
    }
    if (stats.is_set()) {
        res["timings"] = stats.to_json();
    }
    if (is_progress) {
        res["prompt_progress"] = progress.to_json();
    }

    return res;
}

json server_task_result_cmpl_partial::to_json_oaicompat_chat() {
    bool first = n_decoded == 1;
    std::time_t t = std::time(0);
    json choices;

    std::vector<json> deltas;
    auto add_delta = [&](const json & delta) {
        deltas.push_back({
            {"choices", json::array({
                json {
                    {"finish_reason", nullptr},
                    {"index", index},
                    {"delta", delta},
                },
            })},
            {"created", t},
            {"id", oaicompat_cmpl_id},
            {"model", oaicompat_model},
            {"system_fingerprint", std::string(llama_build_info())},
            {"object", "chat.completion.chunk"},
        });
    };
    // We have to send an initial update to conform to openai behavior
    if (first || is_progress) {
        add_delta({
            {"role", "assistant"},
            {"content", nullptr},
        });
    }

    for (const auto & diff : oaicompat_msg_diffs) {
        add_delta(server_chat_msg_diff_to_json_oaicompat(diff));
    }

    if (!deltas.empty()) {
        auto & last_json = deltas[deltas.size() - 1];
        GGML_ASSERT(last_json.at("choices").size() >= 1);

        if (prob_output.probs.size() > 0) {
            last_json.at("choices").at(0)["logprobs"] = json {
                {"content", completion_token_output::probs_vector_to_json({prob_output}, post_sampling_probs)},
            };
        }

        if (stats.is_set()) {
            last_json["timings"] = stats.to_json();
        }
        if (is_progress) {
            last_json["prompt_progress"] = progress.to_json();
        }
    }

    return deltas;
}

json server_task_result_cmpl_partial::to_json_oaicompat_resp() {
    std::vector<json> events;

    if (!oai_resp_created) {
        events.push_back(json {
            {"event", "response.created"},
            {"data", json {
                {"type", "response.created"},
                {"response", json {
                    {"id",     oai_resp_id},
                    {"object", "response"},
                    {"status", "in_progress"},
                }},
            }},
        });
        events.push_back(json {
            {"event", "response.in_progress"},
            {"data", json {
                {"type", "response.in_progress"},
                {"response", json {
                    {"id",     oai_resp_id},
                    {"object", "response"},
                    {"status", "in_progress"},
                }},
            }},
        });
    } else if (is_progress) {
        events.push_back(json {
            {"event", "response.in_progress"},
            {"data", json {
                {"type", "response.in_progress"},
                {"response", json {
                    {"id",     oai_resp_id},
                    {"object", "response"},
                    {"status", "in_progress"},
                }},
            }},
        });
    }

    for (const common_chat_msg_diff & diff : oaicompat_msg_diffs) {
        if (!diff.reasoning_content_delta.empty()) {
            if (!thinking_block_started) {
                events.push_back(json {
                    {"event", "response.output_item.added"},
                    {"data", json {
                        {"type", "response.output_item.added"},
                        {"item", json {
                            {"id",                oai_resp_reasoning_id},
                            {"summary",           json::array()},
                            {"type",              "reasoning"},
                            {"content",           json::array()},
                            {"encrypted_content", ""},
                            {"status",            "in_progress"},
                        }},
                    }},
                });
                thinking_block_started = true;
            }
            events.push_back(json {
                {"event", "response.reasoning_text.delta"},
                {"data", json {
                    {"type",    "response.reasoning_text.delta"},
                    {"delta",   diff.reasoning_content_delta},
                    {"item_id", oai_resp_reasoning_id},
                }},
            });
        }

        if (!diff.content_delta.empty()) {
            if (!text_block_started) {
                events.push_back(json {
                    {"event", "response.output_item.added"},
                    {"data", json {
                        {"type", "response.output_item.added"},
                        {"item", json {
                            {"content", json::array()},
                            {"id",      oai_resp_message_id},
                            {"role",    "assistant"},
                            {"status",  "in_progress"},
                            {"type",    "message"},
                        }},
                    }},
                });
                events.push_back(json {
                    {"event", "response.content_part.added"},
                    {"data", json {
                        {"type",    "response.content_part.added"},
                        {"item_id", oai_resp_message_id},
                        {"part", json {
                            {"type", "output_text"},
                            {"text", ""},
                        }},
                    }},
                });
                text_block_started = true;
            }
            events.push_back(json {
                {"event", "response.output_text.delta"},
                {"data", json {
                    {"type",    "response.output_text.delta"},
                    {"item_id", oai_resp_message_id},
                    {"delta",   diff.content_delta},
                }},
            });
        }

        if (!diff.tool_call_delta.name.empty()) {
            events.push_back(json {
                {"event", "response.output_item.added"},
                {"data", json {
                    {"type",  "response.output_item.added"},
                    {"item", json {
                        {"id",        "fc_" + diff.tool_call_delta.id},
                        {"arguments", ""},
                        {"call_id",   "call_" + diff.tool_call_delta.id},
                        {"name",      diff.tool_call_delta.name},
                        {"type",      "function_call"},
                        {"status",    "in_progress"},
                    }},
                }},
            });
            oai_resp_fc_id = diff.tool_call_delta.id;
        }

        if (!diff.tool_call_delta.arguments.empty()) {
            events.push_back(json {
                {"event", "response.function_call_arguments.delta"},
                {"data", json {
                    {"type",    "response.function_call_arguments.delta"},
                    {"delta",   diff.tool_call_delta.arguments},
                    {"item_id", "fc_" + oai_resp_fc_id},
                }},
            });
        }
    }

    if (!events.empty()) {
        json & data = events.back().at("data");
        if (stats.is_set()) {
            data["timings"] = stats.to_json();
        }
        if (is_progress) {
            data["prompt_progress"] = progress.to_json();
        }
    }

    return events;
}

json server_task_result_cmpl_partial::to_json_oaicompat_asr() {
    json event = json {
        {"type", "transcript.text.delta"},
        {"delta", content},
    };
    return event;
}

json server_task_result_cmpl_partial::to_json_anthropic() {
    json events = json::array();
    bool first = (n_decoded == 1);
    // use member variables to track block state across streaming calls
    // (anthropic_thinking_block_started, anthropic_text_block_started)

    if (first) {
        events.push_back({
            {"event", "message_start"},
            {"data", {
                {"type", "message_start"},
                {"message", {
                    {"id", oaicompat_cmpl_id},
                    {"type", "message"},
                    {"role", "assistant"},
                    {"content", json::array()},
                    {"model", oaicompat_model},
                    {"stop_reason", nullptr},
                    {"stop_sequence", nullptr},
                    {"usage", {
                        {"cache_read_input_tokens", n_prompt_tokens_cache},
                        {"input_tokens", n_prompt_tokens - n_prompt_tokens_cache},
                        {"output_tokens", 0}
                    }}
                }}
            }}
        });
    }

    // content block indices: thinking (0) -> text (0 or 1) -> tool_use (n+)
    size_t thinking_block_index = 0;
    // use anthropic_has_reasoning (set in update()) to know if ANY reasoning was generated
    size_t text_block_index     = anthropic_has_reasoning ? 1 : 0;

    // use local copies of streaming state (copied from task_result_state in update())
    // these reflect the state BEFORE this chunk was processed
    bool thinking_started = thinking_block_started;
    bool text_started     = text_block_started;

    for (const auto & diff : oaicompat_msg_diffs) {
        // handle thinking/reasoning content
        if (!diff.reasoning_content_delta.empty()) {
            if (!thinking_started) {
                events.push_back({
                    {"event", "content_block_start"},
                    {"data", {
                        {"type", "content_block_start"},
                        {"index", thinking_block_index},
                        {"content_block", {
                            {"type", "thinking"},
                            {"thinking", ""}
                        }}
                    }}
                });
                thinking_started = true;
            }

            events.push_back({
                {"event", "content_block_delta"},
                {"data", {
                    {"type", "content_block_delta"},
                    {"index", thinking_block_index},
                    {"delta", {
                        {"type", "thinking_delta"},
                        {"thinking", diff.reasoning_content_delta}
                    }}
                }}
            });
        }

        // handle regular text content
        if (!diff.content_delta.empty()) {
            if (!text_started) {
                events.push_back({
                    {"event", "content_block_start"},
                    {"data", {
                        {"type", "content_block_start"},
                        {"index", text_block_index},
                        {"content_block", {
                            {"type", "text"},
                            {"text", ""}
                        }}
                    }}
                });
                text_started = true;
            }

            events.push_back({
                {"event", "content_block_delta"},
                {"data", {
                    {"type", "content_block_delta"},
                    {"index", text_block_index},
                    {"delta", {
                        {"type", "text_delta"},
                        {"text", diff.content_delta}
                    }}
                }}
            });
        }

        // handle tool calls
        if (diff.tool_call_index != std::string::npos) {
            // use anthropic_has_reasoning for thinking block count (persists across calls)
            size_t content_block_index = (anthropic_has_reasoning ? 1 : 0) + (text_started ? 1 : 0) + diff.tool_call_index;

            if (!diff.tool_call_delta.name.empty()) {
                events.push_back({
                    {"event", "content_block_start"},
                    {"data", {
                        {"type", "content_block_start"},
                        {"index", content_block_index},
                        {"content_block", {
                            {"type", "tool_use"},
                            {"id", diff.tool_call_delta.id},
                            {"name", diff.tool_call_delta.name}
                        }}
                    }}
                });
            }

            if (!diff.tool_call_delta.arguments.empty()) {
                events.push_back({
                    {"event", "content_block_delta"},
                    {"data", {
                        {"type", "content_block_delta"},
                        {"index", content_block_index},
                        {"delta", {
                            {"type", "input_json_delta"},
                            {"partial_json", diff.tool_call_delta.arguments}
                        }}
                    }}
                });
            }
        }
    }

    return events;
}

//
// server_task_result_embd
//
json server_task_result_embd::to_json() {
    return res_type == TASK_RESPONSE_TYPE_OAI_EMBD
        ? to_json_oaicompat()
        : to_json_non_oaicompat();
}

json server_task_result_embd::to_json_non_oaicompat() {
    return json {
        {"index",     index},
        {"embedding", embedding},
    };
}

json server_task_result_embd::to_json_oaicompat() {
    return json {
        {"index",            index},
        {"embedding",        embedding[0]},
        {"tokens_evaluated", n_tokens},
    };
}

//
// server_task_result_rerank
//
json server_task_result_rerank::to_json() {
    return json {
        {"index",            index},
        {"score",            score},
        {"tokens_evaluated", n_tokens},
    };
}

//
// server_task_result_error
//
json server_task_result_error::to_json() {
    json res = format_error_response(err_msg, err_type);
    if (err_type == ERROR_TYPE_EXCEED_CONTEXT_SIZE) {
        res["n_prompt_tokens"] = n_prompt_tokens;
        res["n_ctx"]           = n_ctx;
    }
    return res;
}

//
// server_task_result_metrics
//
json server_task_result_slots::to_json() {
    return slots_data;
}

json server_task_result_metrics::to_json() {
    // not used, /metrics renders prometheus text via to_metrics()
    return json{};
}

// metrics definition: https://prometheus.io/docs/practices/naming/#metric-names
std::string server_task_result_metrics::to_metrics() {
    const std::vector<metric_item> counters = {
        {
            "prompt_tokens_total",
            "Number of prompt tokens processed, excluding cached tokens",
            (double) metrics.prompt.count
        }, {
            "prompt_tokens_cached_total",
            "Number of prompt tokens reused from the cache",
            (double) metrics.n_prompt_cached
        }, {
            "prompt_seconds_total",
            "Total time spent processing prompts",
            metrics.prompt.time / 1.e6
        }, {
            "tokens_predicted_total",
            "Number of generation tokens processed",
            (double) metrics.predict.count
        }, {
            "tokens_predicted_seconds_total",
            "Total time spent generating tokens",
            metrics.predict.time / 1.e6
        }, {
            "n_decode_total",
            "Total number of llama_decode() calls, excluding speculative decoding and multimodal decoding",
            (double) metrics.n_decode
        }, {
            "n_tokens_max",
            "Largest observed sequence length (prompt + generation)",
            (double) metrics.n_tokens_max
        }, {
            "spec_decode_num_draft_tokens_total",
            "Speculative: Total draft tokens generated",
            (double) metrics.n_draft_tokens
        }, {
            "spec_decode_num_accepted_tokens_total",
            "Speculative: Total draft tokens accepted by the target model",
            (double) metrics.n_draft_accepted
        }, {
            "spec_decode_num_drafts_total",
            "Speculative: Total speculative decoding verification steps",
            (double) metrics.n_draft_verif_steps
        },
    };

    const std::vector<metric_item> gauges = {
        {
            "prompt_tokens_seconds",
            "Average prompt throughput in tokens/s",
            metrics.prompt_bucket.n_per_second()
        }, {
            "predicted_tokens_seconds",
            "Average generation throughput in tokens/s",
            metrics.predict_bucket.n_per_second()
        }, {
            "requests_processing",
            "Number of requests processing",
            (double) n_processing_slots
        }, {
            "requests_deferred",
            "Number of requests deferred",
            (double) n_tasks_deferred
        }, {
            "n_busy_slots_per_decode",
            "Average number of busy slots per llama_decode() call",
            (double) metrics.n_busy_slots / std::max((double) metrics.n_decode, 1.0)
        }, {
            "spec_decode_acceptance_rate",
            "Speculative: Fraction of drafted tokens accepted (accepted / drafted)",
            metrics.n_draft_tokens > 0
                ? (double) metrics.n_draft_accepted / (double) metrics.n_draft_tokens
                : 0.0
        }, {
            "spec_decode_accepted_per_step",
            "Speculative: Mean draft tokens accepted per verification step (0 when idle)",
            metrics.n_draft_verif_steps > 0
                ? (double) metrics.n_draft_accepted / (double) metrics.n_draft_verif_steps
                : 0.0
        }, {
            "kv_ejected_total",
            "VITRIOL sparse-KV: KV cells ejected since boot (attention-scored eviction + eager floor sweep)",
            (double) metrics.n_kv_ejected
        }, {
            "kv_cache_spill_total",
            "VITRIOL Optane tier: prompt-cache states mirrored to disk since boot",
            (double) metrics.n_kv_cache_spills
        }, {
            "kv_cache_disk_restored_total",
            "VITRIOL Optane tier: disk states loaded into RAM (boot or fetch)",
            (double) metrics.n_kv_cache_disk_restored
        }, {
            "kv_cache_disk_fetched_total",
            "VITRIOL Optane tier: runtime disk fetches that restored a state",
            (double) metrics.n_kv_cache_disk_fetched
        }, {
            "kv_cache_disk_miss_total",
            "VITRIOL Optane tier: disk scans that found no usable mirror",
            (double) metrics.n_kv_cache_disk_miss
        }, {
            "kv_cache_disk_rejected_total",
            "VITRIOL Optane tier: disk states rejected for identity mismatch",
            (double) metrics.n_kv_cache_disk_rejected
        }, {
            "kv_cache_disk_unverified_total",
            "VITRIOL Optane tier: v1 disk states accepted without identity",
            (double) metrics.n_kv_cache_disk_unverified
        }, {
            "kv_cache_disk_pruned_total",
            "VITRIOL Optane tier: disk states removed (damaged or VITRIOL_KV_DISK_PRUNE=1)",
            (double) metrics.n_kv_cache_disk_pruned
        }, {
            "kv_cache_mem_entries",
            "VITRIOL Optane tier: prompt states currently held in the RAM cache",
            (double) metrics.n_kv_cache_mem_entries
        },
    };

    std::stringstream prometheus;

    auto add_items = [&prometheus](const char * type, const std::vector<metric_item> & items) {
        for (const auto & item : items) {
            prometheus << "# HELP llamacpp:" << item.name << " " << item.description << "\n"
                       << "# TYPE llamacpp:" << item.name << " " << type             << "\n"
                       << "llamacpp:"        << item.name << " " << item.value       << "\n";
        }
    };

    add_items("counter", counters);
    add_items("gauge",   gauges);

    // labeled counter: one time series per draft position
    if (!metrics.n_accepted_per_pos.empty()) {
        prometheus << "# HELP llamacpp:spec_decode_num_accepted_tokens_per_pos_total"
                      " Accepted tokens per draft position\n"
                   << "# TYPE llamacpp:spec_decode_num_accepted_tokens_per_pos_total counter\n";
        for (size_t i = 0; i < metrics.n_accepted_per_pos.size(); i++) {
            prometheus << "llamacpp:spec_decode_num_accepted_tokens_per_pos_total{position=\""
                       << i << "\"} " << metrics.n_accepted_per_pos[i] << "\n";
        }
    }

    // labeled counter: denominator of the per-position acceptance rate
    if (!metrics.n_offered_per_pos.empty()) {
        prometheus << "# HELP llamacpp:spec_decode_num_offered_tokens_per_pos_total"
                      " Draft tokens offered for verification at each draft position\n"
                   << "# TYPE llamacpp:spec_decode_num_offered_tokens_per_pos_total counter\n";
        for (size_t i = 0; i < metrics.n_offered_per_pos.size(); i++) {
            prometheus << "llamacpp:spec_decode_num_offered_tokens_per_pos_total{position=\""
                       << i << "\"} " << metrics.n_offered_per_pos[i] << "\n";
        }
    }

    // labeled counter: verify outcome histogram [draft_len x accepted]
    // (vLLM SpecDecodingStats parity - feeds adaptive draft-K decisions)
    if (!metrics.n_verify_hist.empty()) {
        prometheus << "# HELP llamacpp:spec_decode_verify_outcomes_total"
                      " Verification outcomes by offered draft length and accepted count\n"
                   << "# TYPE llamacpp:spec_decode_verify_outcomes_total counter\n";
        for (size_t r = 0; r < metrics.n_verify_hist.size(); r++) {
            for (size_t c = 0; c < metrics.n_verify_hist[r].size(); c++) {
                if (metrics.n_verify_hist[r][c] == 0) {
                    continue;
                }
                prometheus << "llamacpp:spec_decode_verify_outcomes_total{draft_len=\""
                           << r << "\",accepted=\"" << c << "\"} "
                           << metrics.n_verify_hist[r][c] << "\n";
            }
        }
    }
    // ---- VITRIOL stack-pull S1: latency + cache observability ----
    // PROVENANCE: inspiration - vLLM (Apache-2.0) v1 metrics; re-derived.
    {
        const std::vector<metric_item> vitriol_counters = {
            {
                "sampling_time_seconds_total",
                "Host-side sampling time since boot (grammar + samplers, excludes GPU wait)",
                (double) metrics.n_sampling_us / 1.e6
            }, {
                "sampling_calls_total",
                "Number of common_sampler_sample() calls since boot",
                (double) metrics.n_sampling_calls
            }, {
                "graph_captures_total",
                "CUDA graphs: capture rounds since boot",
                (double) metrics.graph_captures
            }, {
                "graph_reuse_total",
                "CUDA graphs: launches without recapture since boot",
                (double) metrics.graph_reuse
            }, {
                "graph_updates_total",
                "CUDA graphs: exec updates/instantiations after property change",
                (double) metrics.graph_updates
            }, {
                "graph_evicts_total",
                "CUDA graphs: graphs dropped by the 10s-idle sweep",
                (double) metrics.graph_evicts
            },
        };
        add_items("counter", vitriol_counters);

        const double cached = (double) metrics.n_prompt_cached;
        const double processed = (double) metrics.prompt.count;
        const std::vector<metric_item> vitriol_gauges = {
            {
                "cache_hit_ratio",
                "Fraction of prompt tokens served from cache (cached / (cached + processed))",
                (cached + processed) > 0 ? cached / (cached + processed) : 0.0
            },
        };
        add_items("gauge", vitriol_gauges);

        // prometheus histogram helper (bucket + _sum + _count)
        auto add_hist = [&prometheus](const char * name, const char * help,
                                      const uint64_t * buckets, int n_buckets,
                                      const int64_t * bounds_us, uint64_t count, uint64_t sum_us) {
            prometheus << "# HELP llamacpp:" << name << " " << help << "\n"
                       << "# TYPE llamacpp:" << name << " histogram\n";
            uint64_t cum = 0;
            for (int i = 0; i < n_buckets; i++) {
                cum += buckets[i];
                prometheus << "llamacpp:" << name << "_bucket{le=\""
                           << (bounds_us[i] / 1e6) << "\"} " << cum << "\n";
            }
            prometheus << "llamacpp:" << name << "_bucket{le=\"+Inf\"} " << count << "\n"
                       << "llamacpp:" << name << "_sum "    << sum_us / 1e6 << "\n"
                       << "llamacpp:" << name << "_count "  << count << "\n";
        };

        add_hist("ttft_seconds",
                 "Time from task post to first token out",
                 metrics.ttft_bucket, server_metrics::N_LAT_BUCKETS,
                 server_metrics::lat_bounds_us, metrics.ttft_count, metrics.ttft_sum_us);
        add_hist("itl_seconds",
                 "Inter-token latency (one sample per decode step that emitted >= 1 token)",
                 metrics.itl_bucket, server_metrics::N_LAT_BUCKETS,
                 server_metrics::lat_bounds_us, metrics.itl_count, metrics.itl_sum_us);
        add_hist("queue_time_seconds",
                 "Time from task post to slot launch",
                 metrics.queue_bucket, server_metrics::N_LAT_BUCKETS,
                 server_metrics::lat_bounds_us, metrics.queue_count, metrics.queue_sum_us);

        // prefix reuse depth at launch (labeled cumulative counter, tokens)
        prometheus << "# HELP llamacpp:cache_reuse_lcp_bucket"
                      " Requests by prompt tokens reused from cache at slot launch (cumulative)\n"
                   << "# TYPE llamacpp:cache_reuse_lcp_bucket counter\n";
        {
            uint64_t cum = 0;
            for (int i = 0; i < server_metrics::N_LCP_BUCKETS; i++) {
                cum += metrics.lcp_bucket[i];
                prometheus << "llamacpp:cache_reuse_lcp_bucket{le=\""
                           << server_metrics::lcp_bounds[i] << "\"} " << cum << "\n";
            }
            cum += metrics.lcp_inf;
            prometheus << "llamacpp:cache_reuse_lcp_bucket{le=\"+Inf\"} " << cum << "\n"
                       << "llamacpp:cache_reuse_lcp_zero_total " << metrics.lcp_zero << "\n";
        }
    }

    return prometheus.str();
}

//
// server_task_result_slot_save_load
//
json server_task_result_slot_save_load::to_json() {
    if (is_save) {
        return json {
            { "id_slot",   id_slot },
            { "filename",  filename },
            { "n_saved",   n_tokens },
            { "n_written", n_bytes },
            { "timings", {
                { "save_ms", t_ms }
            }},
        };
    }

    return json {
        { "id_slot",    id_slot },
        { "filename",   filename },
        { "n_restored", n_tokens },
        { "n_read",     n_bytes },
        { "timings", {
            { "restore_ms", t_ms }
        }},
    };
}

//
// server_task_result_slot_erase
//
json server_task_result_slot_erase::to_json() {
    return json {
        { "id_slot",  id_slot },
        { "n_erased", n_erased },
    };
}

//
// server_task_result_kv_policy
//
json server_task_result_kv_policy::to_json() {
    return json {
        { "floor",        floor },
        { "protect",      protect },
        { "swept",        swept },
        { "resident",     resident },
        { "buffer_bytes", buffer_bytes },
    };
}

//
// server_task_result_ctx_resize
//
json server_task_result_ctx_resize::to_json() {
    return json {
        { "n_ctx_old",        n_ctx_old },
        { "n_ctx_new",        n_ctx_new },
        { "n_spooled",        n_spooled },
        { "n_tokens_spooled", n_tokens_spooled },
    };
}

//
// server_task_result_get_lora
//

json server_task_result_get_lora::to_json() {
    json result = json::array();
    for (size_t i = 0; i < loras.size(); ++i) {
        auto & lora = loras[i];
        json entry = {
            {"id",            i},
            {"path",          lora.info.path},
            {"scale",         lora.info.scale},
            {"task_name",     lora.info.task_name},
            {"prompt_prefix", lora.info.prompt_prefix},
        };
        if (!lora.alora_invocation_tokens.empty()) {
            entry["alora_invocation_string"] = lora.alora_invocation_string;
            entry["alora_invocation_tokens"] = lora.alora_invocation_tokens;
        }
        result.push_back(std::move(entry));
    }
    return result;
}

//
// server_task_result_apply_lora
//

json server_task_result_apply_lora::to_json() {
    return json {{ "success", true }};
}

//
// server_prompt_cache
//
size_t server_prompt_cache::size() const {
    size_t res = 0;

    for (const auto & state : states) {
        res += state.size();
    }

    return res;
}

size_t server_prompt_cache::n_tokens() const {
    size_t res = 0;

    for (const auto & state : states) {
        res += state.prompt.n_tokens();
    }

    return res;
}

server_prompt_cache_state * server_prompt_cache::alloc(const server_prompt & prompt, size_t state_size_tgt, size_t state_size_dft) {
    // first check if the current state is contained fully in the cache
    for (auto it = states.begin(); it != states.end(); ++it) {
        const int cur_lcp_len = it->prompt.tokens.get_common_prefix(prompt.tokens);

        if (cur_lcp_len == (int) prompt.tokens.size()) {
            SRV_TRC("%s", " - prompt is already in the cache, skipping\n");
            return nullptr;
        }
    }

    // calculate checkpoints size to see if it will fit with the prompt
    size_t checkpoints_size = 0;
    for (const auto & ckpt : prompt.checkpoints) {
        checkpoints_size += ckpt.size();
    }

    const size_t state_size_new = state_size_tgt + state_size_dft + checkpoints_size;

    // skip over-limit entries to avoid disturbing the cache
    if (limit_size > 0 && state_size_new > limit_size) {
        SRV_WRN(" - prompt state size %.3f MiB exceeds cache size limit %.3f MiB, skipping\n",
                state_size_new / (1024.0 * 1024.0), limit_size / (1024.0 * 1024.0));
        return nullptr;
    }

    // remove any cached prompts that are fully contained in the current prompt
    for (auto it = states.begin(); it != states.end();) {
        const int len = it->prompt.tokens.get_common_prefix(prompt.tokens);

        if (len == (int) it->prompt.tokens.size()) {
            SRV_TRC(" - removing obsolete cached prompt with length %d\n", len);

            it = states.erase(it);
        } else {
            ++it;
        }
    }

    if (limit_size > 0) {
        // make room before allocating the new vectors to avoid breaching the limit
        while (!states.empty() && size() + state_size_new > limit_size) {
            // VITRIOL stack-pull S3: least-recently-hit victim, not FIFO
            auto it_victim = states.begin();
            for (auto it = std::next(it_victim); it != states.end(); ++it) {
                if (it->t_last_hit < it_victim->t_last_hit) {
                    it_victim = it;
                }
            }

            SRV_WRN(" - making room for prompt cache entry, removing least-recently-hit entry (hits = %llu, size = %.3f MiB)\n",
                    (unsigned long long) it_victim->n_hits, it_victim->size() / (1024.0 * 1024.0));

            states.erase(it_victim);
        }
    }

    std::vector<uint8_t> state_data_tgt;
    std::vector<uint8_t> state_data_dft;

    // check if we can allocate enough memory for the new state
    try {
        state_data_tgt.resize(state_size_tgt);
        state_data_dft.resize(state_size_dft);
    } catch (const std::bad_alloc & e) {
        SRV_ERR("failed to allocate memory for prompt cache state: %s\n", e.what());

        limit_size = std::max<size_t>(1, 0.4*size());

        SRV_WRN(" - cache size limit reduced to %.3f MiB\n", limit_size / (1024.0 * 1024.0));

        update();

        return nullptr;
    }

    states.push_back({
        /*.prompt =*/ {
            /*.tokens      =*/ prompt.tokens.clone(),
            /*.checkpoints =*/ prompt.checkpoints,
        },
        /*.data   =*/ {
            /*.main =*/ std::make_shared<std::vector<uint8_t>>(std::move(state_data_tgt)),
            /*.drft =*/ std::make_shared<std::vector<uint8_t>>(std::move(state_data_dft)),
        },
    });

    return &states.back();
}

bool server_prompt_cache::load(server_prompt & prompt, const server_tokens & tokens_new, llama_context * ctx_tgt, llama_context * ctx_dft, int32_t id_slot) {
    const int lcp_best = prompt.tokens.get_common_prefix(tokens_new);

    float f_keep_best = prompt.tokens.size() > 0 ? float(lcp_best) / prompt.tokens.size() : -1.0f; // empty slot: any cache entry wins
    float f_sim_best  = float(lcp_best) / tokens_new.size();

    SRV_TRC(" - looking for better prompt, base f_keep = %.3f, f_sim = %.3f\n", f_keep_best, f_sim_best);

    auto it_best = states.end();

    // find the most similar cached prompt, that would also preserve the most context
    for (auto it = states.begin(); it != states.end(); ++it) {
        const int lcp_cur = it->prompt.tokens.get_common_prefix(tokens_new);

        const float f_keep_cur = float(lcp_cur) / it->prompt.tokens.size();
        const float f_sim_cur  = float(lcp_cur) / tokens_new.size();

        SRV_TRC("   - prompt with length %7zu, lcp = %7d, f_keep = %.3f, f_sim = %.3f\n", it->prompt.tokens.size(), lcp_cur, f_keep_cur, f_sim_cur);

        // a state saved under a larger window cannot fit a smaller one
        // (VITRIOL /ctx/resize: n_ctx left the identity, so fit is per-state)
        if (identity.n_ctx > 0 && it->prompt.tokens.size() > identity.n_ctx) {
            continue;
        }

        // don't trash large prompts
        if (f_keep_cur < 0.25f) {
            continue;
        }

        if (f_keep_best < f_keep_cur && f_sim_best < f_sim_cur) {
            f_keep_best = f_keep_cur;
            f_sim_best  = f_sim_cur;

            it_best = it;
        }
    }

    // Optane tier: compare the best disk mirror against the best RAM entry and
    // take the better one, instead of consulting disk only when RAM is empty.
    {
        std::string disk_path;
        float disk_keep = 0.0f;
        float disk_sim  = 0.0f;
        if (disk_find_best(tokens_new, disk_path, disk_keep, disk_sim)) {
            const bool ram_has  = it_best != states.end();
            const bool disk_wins = !ram_has ||
                disk_keep > f_keep_best ||
                (disk_keep == f_keep_best && disk_sim > f_sim_best);
            if (disk_wins && disk_read_best(disk_path, disk_keep)) {
                it_best = std::prev(states.end());
                f_keep_best = disk_keep;
                f_sim_best  = disk_sim;
                SRV_TRC(" - disk fetch wins: f_keep = %.3f, f_sim = %.3f\n", f_keep_best, f_sim_best);
            }
        }
    }

    if (it_best != states.end()) {
        SRV_TRC(" - found better prompt with f_keep = %.3f, f_sim = %.3f\n", f_keep_best, f_sim_best);

        // VITRIOL stack-pull S3: hit tracking + opt-in resident restore.
        // keep_restored=false keeps the upstream consume-on-load behavior
        // (frees the blob: the slot now holds the state live, and peak
        // host RAM stays halved). keep_restored=true retains blob + tokens
        // so an identical resend is a pure cache hit.
        it_best->n_hits++;
        it_best->t_last_hit = ggml_time_us();

        {
            auto & data = *it_best->data.main;

            const size_t size = data.size();
            const size_t n = llama_state_seq_set_data_ext(ctx_tgt, data.data(), size, id_slot, 0);
            if (n != size) {
                SRV_ERR("failed to restore state with size %zu\n", size);

                return false;
            }

            if (!keep_restored) {
                it_best->data.main.reset();
            }
        }

        {
            auto & data_ptr = it_best->data.drft;

            if (data_ptr && !data_ptr->empty()) {
                GGML_ASSERT(ctx_dft);

                const size_t size = data_ptr->size();
                const size_t n = llama_state_seq_set_data_ext(ctx_dft, data_ptr->data(), size, id_slot, 0);
                if (n != size) {
                    SRV_WRN("failed to restore state with size %zu\n", size);

                    return false;
                }

                if (!keep_restored) {
                    data_ptr.reset();
                }
            }
        }

        if (keep_restored) {
            // copy (not move): the entry must keep its tokens to stay
            // matchable; server_tokens is move-assign only, hence clone()
            prompt.tokens       = it_best->prompt.tokens.clone();
            prompt.checkpoints  = it_best->prompt.checkpoints;
        } else {
            prompt = std::move(it_best->prompt);

            states.erase(it_best);
        }
    }

    return true;
}

void server_prompt_cache::update() {
    if (limit_size > 0) {
        // VITRIOL stack-pull S3: evict least-recently-hit, not FIFO. Never
        // restored entries (t_last_hit == 0) go first in insertion order.
        while (!states.empty() && size() > limit_size) {
            auto it_victim = states.begin();
            for (auto it = std::next(it_victim); it != states.end(); ++it) {
                if (it->t_last_hit < it_victim->t_last_hit) {
                    it_victim = it;
                }
            }

            SRV_WRN(" - cache size limit reached, removing least-recently-hit entry (hits = %llu, size = %.3f MiB)\n",
                    (unsigned long long) it_victim->n_hits, it_victim->size() / (1024.0 * 1024.0));

            states.erase(it_victim);
        }
    }

    // average size per token
    const float size_per_token = std::max<float>(1.0f, float(size()) / (std::max<size_t>(1, n_tokens())));

    // dynamically increase the token limit if it can fit in the memory limit
    const size_t limit_tokens_cur = limit_size > 0 ? std::max<size_t>(limit_tokens, limit_size/size_per_token) : limit_tokens;

    if (limit_tokens > 0) {
        while (!states.empty() && n_tokens() > limit_tokens_cur) {
            SRV_WRN(" - cache token limit (%zu, est: %zu) reached, removing oldest entry (size = %.3f MiB)\n",
                    limit_tokens, limit_tokens_cur, states.front().size() / (1024.0 * 1024.0));

            states.pop_front();
        }
    }

    SRV_TRC(" - cache state: %zu prompts, %.3f MiB (limits: %.3f MiB, %zu tokens, %zu est)\n",
            states.size(), size() / (1024.0 * 1024.0), limit_size / (1024.0 * 1024.0), limit_tokens, limit_tokens_cur);

    for (const auto & state : states) {
        SRV_TRC("   - prompt %p: %7d tokens, checkpoints: %2zu, %9.3f MiB\n",
                (const void *)&state, state.prompt.n_tokens(), state.prompt.checkpoints.size(), state.size() / (1024.0 * 1024.0));
    }
}

//
// server_prompt_cache::disk_spill / disk_load (VITRIOL Optane tier)
//
// Mirror admitted states under disk_dir so a server restart reloads them
// instead of re-prefilling. The mirror is a superset of the RAM cache:
// files are content-keyed (same prompt = same file) and trimmed by total
// size only, so states evicted from RAM stay restorable until trimmed.
//

static const uint32_t SERVER_PROMPT_CACHE_DISK_MAGIC   = 0x63706767; // 'ggpc'
static const uint32_t SERVER_PROMPT_CACHE_DISK_VERSION = 2; // v1 = no identity block, still readable

static_assert(sizeof(server_prompt_cache_identity) == 48, "disk identity block must stay 48 bytes");

// env flag: unset/empty = default; "0" = false; anything else = true
static bool server_prompt_cache_env_flag(const char * name, bool def) {
    const char * v = getenv(name);
    if (v == nullptr || v[0] == '\0') {
        return def;
    }
    return !(v[0] == '0' && v[1] == '\0');
}

// verdict codes used by disk read/scan and the /cache endpoint
enum server_prompt_cache_verdict {
    SERVER_PROMPT_CACHE_VERDICT_UNKNOWN  = 0,
    SERVER_PROMPT_CACHE_VERDICT_OK       = 1, // v2 and matches
    SERVER_PROMPT_CACHE_VERDICT_MISMATCH = 2, // v2 but different model/config
    SERVER_PROMPT_CACHE_VERDICT_V1       = 3, // no identity block
};

// hard incompatibility: KV layout or semantics differ
static bool server_prompt_cache_identity_hard_ok(
        const server_prompt_cache_identity & cur, const server_prompt_cache_identity & f) {
    return f.type_k == cur.type_k && f.type_v == cur.type_v && f.n_stream == cur.n_stream;
}

static int server_prompt_cache_identity_verdict(
        const server_prompt_cache_identity & cur, const server_prompt_cache_identity & f, uint32_t version) {
    if (version < 2) {
        return SERVER_PROMPT_CACHE_VERDICT_V1;
    }
    if (!server_prompt_cache_identity_hard_ok(cur, f)) {
        return SERVER_PROMPT_CACHE_VERDICT_MISMATCH;
    }
    if (f.model_key != cur.model_key) {
        // model differs; strict by default, relaxed when explicitly allowed
        return server_prompt_cache_env_flag("VITRIOL_KV_DISK_ALLOW_MODEL_MISMATCH", false)
            ? SERVER_PROMPT_CACHE_VERDICT_OK
            : SERVER_PROMPT_CACHE_VERDICT_MISMATCH;
    }
    return SERVER_PROMPT_CACHE_VERDICT_OK;
}

static bool server_prompt_cache_verdict_accepts(int verdict) {
    if (verdict == SERVER_PROMPT_CACHE_VERDICT_OK) {
        return true;
    }
    if (verdict == SERVER_PROMPT_CACHE_VERDICT_V1) {
        // v1 files carry no identity; accept unless strict mode is on
        return !server_prompt_cache_env_flag("VITRIOL_KV_DISK_STRICT", false);
    }
    return false;
}

server_prompt_cache::server_prompt_cache(int32_t limit_size_mib, size_t limit_tokens, const std::string & disk_dir) {
    this->limit_size   = 1024ull*1024ull*(limit_size_mib < 0 ? 0 : limit_size_mib);
    this->limit_tokens = limit_tokens;
    if (!disk_dir.empty()) {
        this->disk_dir = disk_dir.back() == '/' ? disk_dir : disk_dir + "/";
    }
    // VITRIOL stack-pull S3: opt-in resident-restore (OOM discipline: the
    // default consume-on-load halves peak host RAM for big states)
    keep_restored = getenv("VITRIOL_CACHE_KEEP") != nullptr && getenv("VITRIOL_CACHE_KEEP")[0] == '1';
}

// FNV-1a over token count + head + tail: same prompt = same token hash
static uint64_t server_prompt_cache_disk_token_hash(const server_tokens & tokens) {
    const llama_tokens & t = tokens.get_tokens();
    const size_t n = t.size();

    uint64_t h = 1469598103934665603ull;
    auto mix = [&h] (uint64_t v) {
        h ^= v;
        h *= 1099511628211ull;
    };
    mix(n);
    const size_t head = std::min<size_t>(n, 64);
    for (size_t i = 0; i < head; i++) {
        mix((uint64_t)(uint32_t) t[i]);
    }
    for (size_t i = n - std::min<size_t>(n, 64); i < n; i++) {
        mix((uint64_t)(uint32_t) t[i]);
    }

    return h;
}

// v2 name carries the model key so states from different models cannot collide.
// The key is informational for pruning; validation always reads the file body.
static std::string server_prompt_cache_disk_name(const server_tokens & tokens, uint64_t model_key) {
    const uint64_t th = server_prompt_cache_disk_token_hash(tokens);

    char buf[48]; // "cache-" + 16 + "-" + 16 + ".bin" + NUL
    snprintf(buf, sizeof(buf), "cache-%016llx-%016llx.bin",
            (unsigned long long) model_key, (unsigned long long) th);

    return std::string(buf);
}

static bool server_prompt_cache_disk_write(const std::string & path,
        const server_prompt_cache_identity & identity, const server_tokens & tokens,
        const std::vector<uint8_t> & data_main, const std::vector<uint8_t> & data_drft) {
    std::vector<char> packed;
    try {
        packed = tokens.serialize();
    } catch (const std::exception & e) {
        SRV_WRN(" - disk spill: failed to serialize tokens: %s\n", e.what());
        return false;
    }

    FILE * f = fopen((path + ".tmp").c_str(), "wb");
    if (f == nullptr) {
        return false;
    }

    const uint32_t magic   = SERVER_PROMPT_CACHE_DISK_MAGIC;
    const uint32_t version = SERVER_PROMPT_CACHE_DISK_VERSION;
    const uint64_t n_main  = data_main.size();
    const uint64_t n_drft  = data_drft.size();
    const uint64_t n_pack  = packed.size();

    auto ok_write = [&f] (const void * p, size_t n) {
        return n == 0 || fwrite(p, 1, n, f) == n;
    };

    bool ok = ok_write(&magic, 4) && ok_write(&version, 4) &&
              ok_write(&identity, sizeof(identity)) &&
              ok_write(&n_main, 8) && ok_write(&n_drft, 8) && ok_write(&n_pack, 8) &&
              ok_write(data_main.data(), n_main) && ok_write(data_drft.data(), n_drft) &&
              ok_write(packed.data(), n_pack);

    ok = fclose(f) == 0 && ok;

    if (!ok) {
        SRV_ERR(" - disk spill: short write on %s.tmp\n", path.c_str());
        remove((path + ".tmp").c_str());
        return false;
    }

    return rename((path + ".tmp").c_str(), path.c_str()) == 0;
}

// reads the header + token blob only; skips the KV bytes with fseek.
// identity_out is zeroed for v1 files. Returns false on a damaged header.
static bool server_prompt_cache_disk_read_tokens(const std::string & path, server_tokens & out,
        uint64_t & n_main, uint64_t & n_drft, server_prompt_cache_identity & identity_out, uint32_t & version_out) {
    FILE * f = fopen(path.c_str(), "rb");
    if (f == nullptr) {
        return false;
    }

    uint32_t magic   = 0;
    uint32_t version = 0;
    uint64_t n_pack  = 0;

    auto ok_read = [&f] (void * p, size_t n) {
        return n == 0 || fread(p, 1, n, f) == n;
    };

    bool ok = ok_read(&magic, 4) && ok_read(&version, 4);
    if (ok && version >= 2) {
        ok = ok_read(&identity_out, sizeof(identity_out));
    } else {
        identity_out = {};
    }
    ok = ok && ok_read(&n_main, 8) && ok_read(&n_drft, 8) && ok_read(&n_pack, 8);

    ok = ok && magic == SERVER_PROMPT_CACHE_DISK_MAGIC && version >= 1 && version <= SERVER_PROMPT_CACHE_DISK_VERSION;
    ok = ok && n_pack % sizeof(uint32_t) == 0 && n_pack > 0 && n_pack <= (1ull << 30);
    ok = ok && n_main <= (64ull << 30) && n_drft <= (8ull << 30);

    std::vector<llama_token> packed;
    if (ok) {
        packed.resize(n_pack / sizeof(uint32_t));
        ok = fseek(f, (long) (n_main + n_drft), SEEK_CUR) == 0 &&
             ok_read(packed.data(), n_pack);
    }
    fclose(f);

    if (!ok) {
        return false;
    }

    version_out = version;

    try {
        out = server_tokens::deserialize(packed, false);
    } catch (const std::exception & e) {
        SRV_WRN(" - disk load: %s: bad token blob: %s\n", path.c_str(), e.what());
        return false;
    }

    return out.size() > 0;
}

static bool server_prompt_cache_disk_read(const std::string & path, server_prompt_cache_state & out,
        server_prompt_cache_identity & identity_out, uint32_t & version_out) {
    FILE * f = fopen(path.c_str(), "rb");
    if (f == nullptr) {
        return false;
    }

    uint32_t magic   = 0;
    uint32_t version = 0;
    uint64_t n_main  = 0;
    uint64_t n_drft  = 0;
    uint64_t n_pack  = 0;

    auto ok_read = [&f] (void * p, size_t n) {
        return n == 0 || fread(p, 1, n, f) == n;
    };

    bool ok = ok_read(&magic, 4) && ok_read(&version, 4);
    if (ok && version >= 2) {
        ok = ok_read(&identity_out, sizeof(identity_out));
    } else {
        identity_out = {};
    }
    ok = ok && ok_read(&n_main, 8) && ok_read(&n_drft, 8) && ok_read(&n_pack, 8);

    ok = ok && magic == SERVER_PROMPT_CACHE_DISK_MAGIC && version >= 1 && version <= SERVER_PROMPT_CACHE_DISK_VERSION;
    ok = ok && n_pack % sizeof(uint32_t) == 0 && n_pack <= (1ull << 30) &&
         n_main <= (64ull << 30) && n_drft <= (8ull << 30);

    if (!ok) {
        fclose(f);
        return false;
    }

    out.data.main = std::make_shared<std::vector<uint8_t>>(n_main);
    out.data.drft = std::make_shared<std::vector<uint8_t>>(n_drft);
    std::vector<llama_token> packed(n_pack / sizeof(uint32_t));

    ok = ok_read(out.data.main->data(), n_main) &&
         ok_read(out.data.drft->data(), n_drft) &&
         ok_read(packed.data(), n_pack);
    fclose(f);

    if (!ok) {
        return false;
    }

    version_out = version;

    try {
        out.prompt.tokens = server_tokens::deserialize(packed, false);
    } catch (const std::exception & e) {
        SRV_WRN(" - disk load: %s: bad token blob: %s\n", path.c_str(), e.what());
        return false;
    }

    return out.prompt.tokens.size() > 0;
}

// collect cache-*.bin entries, oldest first; returns total bytes
struct server_prompt_cache_disk_file {
    time_t mtime;
    size_t size;
    std::string name;
};

static size_t server_prompt_cache_disk_scan(const std::string & dir,
        std::vector<server_prompt_cache_disk_file> & files) {
    size_t total = 0;

    DIR * d = opendir(dir.c_str());
    if (d == nullptr) {
        return 0;
    }

    dirent * e;
    while ((e = readdir(d)) != nullptr) {
        const std::string name(e->d_name);
        if (name.rfind("cache-", 0) != 0 || name.size() < 10 ||
                name.compare(name.size() - 4, 4, ".bin") != 0) {
            continue;
        }
        struct stat st;
        if (stat((dir + name).c_str(), &st) == 0 && S_ISREG(st.st_mode)) {
            files.push_back({ st.st_mtime, (size_t) st.st_size, name });
            total += st.st_size;
        }
    }
    closedir(d);

    std::sort(files.begin(), files.end(), [](const auto & a, const auto & b) {
        if (a.mtime != b.mtime) {
            return a.mtime < b.mtime;
        }
        return a.name < b.name;
    });
    return total;
}

// mirror cap = multiplier x RAM pool. Was 8x; at cram=12288 the live mirror sat
// at 95.7/96 GiB, so every spill silently deleted the oldest session and a
// restart re-prefilled it from token 0. Override: VITRIOL_KV_DISK_TRIM_X (1..1024).
static size_t server_prompt_cache_disk_trim_limit(size_t limit_size) {
    int mult = 12;
    const char * v = getenv("VITRIOL_KV_DISK_TRIM_X");
    if (v != nullptr && v[0] != '\0') {
        const int parsed = atoi(v);
        if (parsed >= 1 && parsed <= 1024) {
            mult = parsed;
        } else {
            SRV_WRN(" - disk trim: VITRIOL_KV_DISK_TRIM_X=%s out of range 1..1024, using %d\n", v, mult);
        }
    }
    return limit_size * (size_t) mult;
}

static void server_prompt_cache_disk_trim(const std::string & dir, size_t limit_bytes, size_t incoming) {
    if (limit_bytes == 0 || incoming > limit_bytes) {
        return;
    }

    std::vector<server_prompt_cache_disk_file> files;
    size_t total = server_prompt_cache_disk_scan(dir, files);

    if (total + incoming <= limit_bytes) {
        return;
    }

    SRV_WRN(" - disk trim: mirror at %.1f GiB + incoming %.1f GiB exceeds %.1f GiB limit, removing oldest files\n",
            total / 1073741824.0, incoming / 1073741824.0, limit_bytes / 1073741824.0);

    size_t n_removed = 0;
    size_t bytes_removed = 0;
    for (const auto & file : files) {
        if (total + incoming <= limit_bytes) {
            break;
        }
        if (remove((dir + file.name).c_str()) == 0) {
            total -= file.size;
            bytes_removed += file.size;
            n_removed++;
            SRV_WRN(" - disk trim: removed %s (%.2f GiB)\n", file.name.c_str(), file.size / 1073741824.0);
        }
    }

    SRV_WRN(" - disk trim: freed %zu files (%.1f GiB), mirror now %.1f GiB\n",
            n_removed, bytes_removed / 1073741824.0, total / 1073741824.0);
}

void server_prompt_cache::disk_spill(const server_prompt & prompt,
        std::shared_ptr<std::vector<uint8_t>> data_main, std::shared_ptr<std::vector<uint8_t>> data_drft) {
    if (disk_dir.empty() || prompt.tokens.has_mtmd || !data_main) {
        return;
    }

    const size_t spill_size = data_main->size() + (data_drft ? data_drft->size() : 0);

    server_prompt_cache_disk_trim(disk_dir, server_prompt_cache_disk_trim_limit(limit_size), spill_size); // mirror keeps ~12x the RAM pool: evicted-from-RAM states stay fetchable

    const std::string name = server_prompt_cache_disk_name(prompt.tokens, identity.model_key);
    const std::string path = disk_dir + name;
    const server_prompt_cache_identity id = identity;

    // detached writer: dispatch hands off the bytes (shared, no copy) and
    // returns immediately; a torn .tmp from a lost race is inert
    std::thread([this, name, path, id, tokens = prompt.tokens.clone(), data_main, data_drft]() {
        if (server_prompt_cache_disk_write(path, id, tokens, *data_main, data_drft ? *data_drft : std::vector<uint8_t>())) {
            n_disk_spills++;
            SRV_TRC(" - disk spill: %s (%.1f MiB)\n", name.c_str(), (data_main->size() + (data_drft ? data_drft->size() : 0)) / (1024.0 * 1024.0));
        } else {
            SRV_WRN(" - disk spill failed: %s\n", path.c_str());
        }
    }).detach();
}

bool server_prompt_cache::disk_find_best(const server_tokens & tokens_new, std::string & path_out,
        float & f_keep_out, float & f_sim_out) {
    path_out.clear();
    if (disk_dir.empty() || tokens_new.size() == 0) {
        return false;
    }

    std::vector<server_prompt_cache_disk_file> files;
    server_prompt_cache_disk_scan(disk_dir, files);
    // newest first: an evolved prefix beats its ancestors on ties
    std::sort(files.begin(), files.end(), [](const auto & a, const auto & b) {
        if (a.mtime != b.mtime) {
            return a.mtime > b.mtime;
        }
        return a.name > b.name;
    });

    const bool prune = server_prompt_cache_env_flag("VITRIOL_KV_DISK_PRUNE", false);
    float best_keep = 0.0f;
    float best_sim  = 0.0f;

    // near-miss diagnostics: best candidate that failed the f_keep gate
    int    rejected_keep  = 0;
    float  worst_keep     = 0.0f;
    float  worst_keep_sim = 0.0f;
    size_t worst_keep_tok = 0;
    size_t n_scanned      = 0;

    for (const auto & file : files) {
        const std::string path = disk_dir + file.name;
        n_scanned++;

        server_tokens cand;
        uint64_t n_main = 0;
        uint64_t n_drft = 0;
        server_prompt_cache_identity id {};
        uint32_t version = 0;
        if (!server_prompt_cache_disk_read_tokens(path, cand, n_main, n_drft, id, version)) {
            n_disk_damaged++;
            if (prune) {
                remove(path.c_str());
                n_disk_pruned++;
            }
            continue;
        }

        const int verdict = server_prompt_cache_identity_verdict(identity, id, version);
        if (!server_prompt_cache_verdict_accepts(verdict)) {
            if (verdict == SERVER_PROMPT_CACHE_VERDICT_MISMATCH) {
                if (server_prompt_cache_identity_hard_ok(identity, id)) {
                    n_disk_rejected_model++;
                } else {
                    n_disk_rejected_config++;
                }
            }
            SRV_WRN(" - disk fetch: skip %s (identity mismatch, key %016llx)\n",
                    file.name.c_str(), (unsigned long long) id.model_key);
            if (prune) {
                remove(path.c_str());
                n_disk_pruned++;
            }
            continue;
        }
        if (verdict == SERVER_PROMPT_CACHE_VERDICT_V1) {
            n_disk_unverified++;
        }

        // skip candidates already fully covered by a live RAM entry
        bool contained = false;
        for (const auto & ex : states) {
            if (ex.prompt.tokens.get_common_prefix(cand) == (int) cand.size()) {
                contained = true;
                break;
            }
        }
        if (contained) {
            n_disk_skipped_contained++;
            continue;
        }

        const int lcp = cand.get_common_prefix(tokens_new);
        if (lcp <= 0) {
            continue;
        }

        // same "do not trash large prompts" rule as the RAM scan
        const float f_keep = float(lcp) / cand.size();
        const float f_sim  = float(lcp) / tokens_new.size();
        if (f_keep < 0.25f) {
            rejected_keep++;
            if (f_keep > worst_keep) {
                worst_keep     = f_keep;
                worst_keep_sim = f_sim;
                worst_keep_tok = cand.size();
            }
            continue;
        }

        if (f_keep > best_keep || (f_keep == best_keep && f_sim > best_sim)) {
            best_keep = f_keep;
            best_sim  = f_sim;
            path_out  = path;
        }
    }

    if (path_out.empty()) {
        n_disk_miss++;
        if (rejected_keep > 0) {
            // a mirror of this conversation exists but the f_keep gate refused it
            SRV_WRN(" - disk fetch: no usable mirror for %zu tokens (%zu files scanned, %d rejected below f_keep 0.25: best lcp kept %.2f of %zu tokens, f_sim %.2f)\n",
                    tokens_new.size(), n_scanned, rejected_keep, worst_keep, worst_keep_tok, worst_keep_sim);
        } else {
            SRV_TRC(" - disk fetch: no usable mirror for %zu tokens (%zu files scanned)\n", tokens_new.size(), n_scanned);
        }
        return false;
    }

    f_keep_out = best_keep;
    f_sim_out  = best_sim;
    return true;
}

bool server_prompt_cache::disk_read_best(const std::string & path, float f_keep) {
    server_prompt_cache_state st;
    server_prompt_cache_identity id {};
    uint32_t version = 0;
    if (!server_prompt_cache_disk_read(path, st, id, version)) {
        SRV_WRN(" - disk fetch: damaged state %s\n", path.c_str());
        n_disk_damaged++;
        remove(path.c_str());
        return false;
    }

    const int verdict = server_prompt_cache_identity_verdict(identity, id, version);
    if (!server_prompt_cache_verdict_accepts(verdict)) {
        n_disk_rejected_model++;
        SRV_WRN(" - disk fetch: rejecting %s (identity mismatch, key %016llx)\n",
                path.c_str(), (unsigned long long) id.model_key);
        return false;
    }

    if (limit_size > 0 && size() + st.size() > limit_size) {
        // make room the same way alloc() does (oldest first), never trim to zero
        while (!states.empty() && size() + st.size() > limit_size) {
            states.pop_front();
        }
    }

    SRV_WRN(" - disk fetch: restored %.1f MiB state (%d tokens, keep %.2f, key %016llx) from %s\n",
            st.size() / (1024.0 * 1024.0), st.prompt.n_tokens(), f_keep,
            (unsigned long long) id.model_key, path.c_str());
    states.push_back(std::move(st));
    n_disk_restored++;
    n_disk_fetched++;

    // refresh mtime so the oldest-first trim prefers least-recently-fetched
    utimes(path.c_str(), nullptr);

    return true;
}

bool server_prompt_cache::disk_fetch(const server_tokens & tokens_new) {
    std::string path;
    float f_keep = 0.0f;
    float f_sim  = 0.0f;
    if (!disk_find_best(tokens_new, path, f_keep, f_sim)) {
        return false;
    }
    return disk_read_best(path, f_keep);
}

void server_prompt_cache::disk_load() {
    if (disk_dir.empty()) {
        return;
    }

    std::vector<server_prompt_cache_disk_file> files;
    server_prompt_cache_disk_scan(disk_dir, files);

    // newest first: evolved (longer) prefixes load before their ancestors
    std::sort(files.begin(), files.end(), [](const auto & a, const auto & b) {
        if (a.mtime != b.mtime) {
            return a.mtime > b.mtime;
        }
        return a.name > b.name;
    });

    const bool prune = server_prompt_cache_env_flag("VITRIOL_KV_DISK_PRUNE", false);
    size_t n_loaded    = 0;
    size_t n_dropped   = 0;
    size_t n_rejected  = 0;
    size_t n_unverified = 0;
    for (const auto & file : files) {
        server_prompt_cache_state st;
        server_prompt_cache_identity id {};
        uint32_t version = 0;
        if (!server_prompt_cache_disk_read(disk_dir + file.name, st, id, version)) {
            SRV_WRN(" - disk load: removing damaged state %s\n", file.name.c_str());
            remove((disk_dir + file.name).c_str());
            n_disk_damaged++;
            n_dropped++;
            continue;
        }

        const int verdict = server_prompt_cache_identity_verdict(identity, id, version);
        if (!server_prompt_cache_verdict_accepts(verdict)) {
            if (server_prompt_cache_identity_hard_ok(identity, id)) {
                n_disk_rejected_model++;
            } else {
                n_disk_rejected_config++;
            }
            SRV_WRN(" - disk load: skip %s (identity mismatch, key %016llx)\n",
                    file.name.c_str(), (unsigned long long) id.model_key);
            if (prune) {
                remove((disk_dir + file.name).c_str());
                n_disk_pruned++;
            }
            n_rejected++;
            continue;
        }
        if (verdict == SERVER_PROMPT_CACHE_VERDICT_V1) {
            n_disk_unverified++;
            n_unverified++;
        }

        // skip entries fully contained in an already loaded (newer) entry
        bool contained = false;
        for (const auto & ex : states) {
            if (ex.prompt.tokens.get_common_prefix(st.prompt.tokens) == (int) st.prompt.tokens.size()) {
                contained = true;
                break;
            }
        }
        if (contained) {
            n_disk_skipped_contained++;
            n_dropped++;
            continue;
        }

        if (limit_size > 0 && size() + st.size() > limit_size) {
            // RAM pool full: keep scanning, a later (older, smaller) file may fit
            SRV_TRC(" - disk load: %s kept on disk (RAM pool full)\n", file.name.c_str());
            n_dropped++;
            continue;
        }

        states.push_back(std::move(st));
        n_disk_restored++;
        n_loaded++;
    }

    if (n_loaded > 0 || n_dropped > 0 || n_rejected > 0) {
        SRV_WRN(" - disk load: restored %zu prompt states (%zu skipped, %zu rejected, %zu unverified) from %s\n",
                n_loaded, n_dropped, n_rejected, n_unverified, disk_dir.c_str());
    }
}

std::vector<server_prompt_cache_disk_info> server_prompt_cache::disk_list() const {
    std::vector<server_prompt_cache_disk_info> out;
    if (disk_dir.empty()) {
        return out;
    }

    std::vector<server_prompt_cache_disk_file> files;
    server_prompt_cache_disk_scan(disk_dir, files);

    for (const auto & file : files) {
        server_prompt_cache_disk_info info;
        info.name  = file.name;
        info.size  = file.size;
        info.mtime = file.mtime;

        server_tokens cand;
        uint64_t n_main = 0;
        uint64_t n_drft = 0;
        server_prompt_cache_identity id {};
        uint32_t version = 0;
        if (server_prompt_cache_disk_read_tokens(disk_dir + file.name, cand, n_main, n_drft, id, version)) {
            info.n_tokens  = cand.size();
            info.version   = version;
            info.model_key = id.model_key;
            info.verdict   = server_prompt_cache_identity_verdict(identity, id, version);
        } else {
            info.verdict = SERVER_PROMPT_CACHE_VERDICT_UNKNOWN;
        }

        out.push_back(std::move(info));
    }

    return out;
}
