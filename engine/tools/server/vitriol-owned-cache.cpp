#include "vitriol-owned-cache.h"
#include "vitriol-cache-store.h"
#include "log.h"

#include <utility>

struct server_owned_runtime::impl {
    std::unique_ptr<vitriol_cache_handle, decltype(&vitriol_cache_close)> handle{nullptr, vitriol_cache_close};
};

server_owned_runtime::~server_owned_runtime() = default;

std::expected<std::unique_ptr<server_owned_runtime>, server_owned_error> server_owned_runtime::open(const std::string & directory, const common_json & config) {
    char * error = nullptr;
    const auto request = common_json{{"path", directory}, {"config", config}}.dump();
    std::unique_ptr<vitriol_cache_handle, decltype(&vitriol_cache_close)> handle(
        vitriol_cache_open(request.c_str(), &error), vitriol_cache_close);
    std::unique_ptr<char, decltype(&vitriol_cache_string_free)> message(error, vitriol_cache_string_free);
    if (!handle) {
        LOG_WRN("managed KV cache unavailable: %s\n", message ? message.get() : "no error detail");
        return std::unexpected(server_owned_error::backend);
    }
    auto runtime = std::make_unique<server_owned_runtime>();
    runtime->state = std::make_unique<impl>();
    runtime->state->handle = std::move(handle);
    runtime->directory = directory;
    return runtime;
}

std::expected<void, server_owned_error> server_owned_runtime::call(const common_json & command, common_json & result) const {
    try {
        const auto request = command.dump();
        std::unique_ptr<char, decltype(&vitriol_cache_string_free)> raw(
            vitriol_cache_call(state->handle.get(), request.c_str()), vitriol_cache_string_free);
        if (!raw) { return std::unexpected(server_owned_error::protocol); }
        auto reply = common_json::parse(raw.get());
        if (!reply.value("ok", false)) {
            LOG_DBG("managed KV operation rejected: %s\n", reply.value("error", std::string("unknown")).c_str());
            return std::unexpected(server_owned_error::backend);
        }
        result = reply.at("result");
        return {};
    } catch (const std::exception & error) {
        LOG_WRN("managed KV reply rejected: %s\n", error.what());
        return std::unexpected(server_owned_error::protocol);
    }
}

std::expected<std::unique_ptr<server_owned_request>, server_owned_error> server_owned_runtime::begin(const std::string & session, const std::string & branch) {
    common_json grant;
    auto status = call(common_json{{"op", "begin"}, {"session", session}, {"branch", branch}}, grant);
    if (!status) { return std::unexpected(status.error()); }
    return std::make_unique<server_owned_request>(shared_from_this(), grant.at("generation").get<uint64_t>());
}

std::expected<std::unique_ptr<server_owned_read>, server_owned_error> server_owned_runtime::read(const std::string & id, const std::shared_ptr<server_owned_request> & owner) {
    common_json request{{"op", "acquire_read"}, {"id", id}, {"generation", common_json()}};
    if (owner) { request["generation"] = owner->generation; }
    common_json result;
    auto status = call(request, result);
    if (!status) { return std::unexpected(status.error()); }
    return std::make_unique<server_owned_read>(shared_from_this(), result.get<uint64_t>());
}

server_owned_request::server_owned_request(std::shared_ptr<server_owned_runtime> runtime, uint64_t generation)
    : runtime(std::move(runtime)), generation(generation) {}

server_owned_request::~server_owned_request() {
    try { common_json ignored; (void)runtime->call(common_json{{"op", "release_tag"}, {"generation", generation}}, ignored); }
    catch (...) { /* Destruction cannot make inference fail. */ }
}

void server_owned_request::finish(bool success) {
    if (finished) { return; }
    finished = true;
    common_json ignored;
    (void)runtime->call(common_json{{"op", "finish"}, {"generation", generation}, {"success", success}}, ignored);
}

server_owned_read::server_owned_read(std::shared_ptr<server_owned_runtime> runtime, uint64_t lease)
    : runtime(std::move(runtime)), lease(lease) {}

server_owned_read::~server_owned_read() {
    try { common_json ignored; (void)runtime->call(common_json{{"op", "release_read"}, {"lease", lease}}, ignored); }
    catch (...) { /* A failed release retains data until owner recovery. */ }
}
