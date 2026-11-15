#pragma once

#include "json.h"

#include <cstdint>
#include <atomic>
#include <expected>
#include <memory>
#include <string>

enum class server_owned_error { backend, protocol };
struct server_owned_runtime;

struct server_owned_request {
    std::shared_ptr<server_owned_runtime> runtime;
    uint64_t generation;
    bool finished = false;

    server_owned_request(std::shared_ptr<server_owned_runtime> runtime, uint64_t generation)
        pre(runtime != nullptr) pre(generation > 0);
    ~server_owned_request();
    void finish(bool success) pre(runtime != nullptr);
};

struct server_owned_read {
    std::shared_ptr<server_owned_runtime> runtime;
    uint64_t lease;

    server_owned_read(std::shared_ptr<server_owned_runtime> runtime, uint64_t lease)
        pre(runtime != nullptr) pre(lease > 0);
    ~server_owned_read();
};

struct server_owned_runtime : std::enable_shared_from_this<server_owned_runtime> {
    struct impl;
    std::unique_ptr<impl> state;
    std::string directory;
    std::atomic<uint64_t> spills{0};

    ~server_owned_runtime();
    static std::expected<std::unique_ptr<server_owned_runtime>, server_owned_error> open(const std::string & directory, const common_json & config)
        pre(!directory.empty());
    std::expected<void, server_owned_error> call(const common_json & command, common_json & result) const pre(state != nullptr);
    std::expected<std::unique_ptr<server_owned_request>, server_owned_error> begin(const std::string & session, const std::string & branch)
        pre(!session.empty()) pre(!branch.empty());
    std::expected<std::unique_ptr<server_owned_read>, server_owned_error> read(const std::string & id, const std::shared_ptr<server_owned_request> & owner = {})
        pre(!id.empty());
};

static_assert(sizeof(std::expected<void, server_owned_error>) <= 16);
