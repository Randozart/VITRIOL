#include "server-task.h"
#include "server-schema.h"

#include <chrono>
#include <filesystem>
#include <iostream>
#include <stdexcept>
#include <thread>
#include <unistd.h>

namespace fs = std::filesystem;

// Keep assertions active in release builds.
static void require(bool condition, const char * message) {
    if (!condition) { throw std::runtime_error(message); }
}

struct temporary_directory {
    fs::path path;
    temporary_directory() {
        const auto stamp = std::chrono::steady_clock::now().time_since_epoch().count();
        path = fs::temp_directory_path() / ("vitriol-engine-cache-" + std::to_string(getpid()) + "-" + std::to_string(stamp));
        require(fs::create_directory(path), "test directory already exists");
    }
    ~temporary_directory() { std::error_code error; fs::remove_all(path, error); }
};

static void configure_identity(server_prompt_cache & cache) {
    cache.identity.model_key = 11;
    cache.identity.build_key = 22;
    cache.identity.n_ctx = 128;
    cache.identity.n_stream = 1;
}

static void await_spills(const server_prompt_cache & cache, uint64_t expected, bool managed) {
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(5);
    while ((managed ? cache.owned->spills.load(std::memory_order_relaxed) : cache.n_disk_spills.load()) < expected) {
        require(std::chrono::steady_clock::now() < deadline, "disk writer did not finish");
        std::this_thread::sleep_for(std::chrono::milliseconds(5));
    }
}

static server_prompt prompt(int suffix) {
    server_prompt result;
    result.tokens.push_back(1);
    result.tokens.push_back(2);
    result.tokens.push_back(suffix);
    return result;
}

static std::string publish(server_prompt_cache & cache, int suffix) {
    auto state = prompt(suffix);
    state.cache_owner = cache.owned_begin("test-session", "main");
    require(state.cache_owner != nullptr, "no owner grant");
    state.cache_owner->finish(true);
    state.cache_owner->finish(false); // Slot release cannot downgrade a completed request.
    const auto expected = cache.owned->spills.load(std::memory_order_relaxed) + 1;
    cache.disk_spill(state, std::make_shared<std::vector<uint8_t>>(4, 7), {});
    await_spills(cache, expected, true);
    require(!state.cache_source.empty(), "publication id not attached to RAM state");
    return state.cache_source;
}

static void legacy_parity(const fs::path & root) {
    fs::create_directory(root);
    setenv("VITRIOL_KV_OWNED_CACHE", "0", 1);
    server_prompt_cache cache(1, 128, root.string());
    configure_identity(cache);
    cache.owned_init();
    require(!cache.owned, "disabled feature opened a store");
    auto state = prompt(3);
    cache.disk_spill(state, std::make_shared<std::vector<uint8_t>>(4, 7), {});
    await_spills(cache, 1, false);
    auto files = cache.disk_list();
    require(files.size() == 1 && files[0].verdict == 1, "legacy state format changed");
    require(files[0].name.find('/') == std::string::npos, "legacy state moved namespaces");
}

static void managed_publication_and_read(const fs::path & root) {
    fs::create_directory(root);
    setenv("VITRIOL_KV_OWNED_CACHE", "1", 1);
    setenv("VITRIOL_KV_OWNED_RECLAIM", "0", 1);
    server_prompt_cache cache(1, 128, root.string());
    configure_identity(cache);
    cache.owned_init();
    require(cache.owned != nullptr, "managed store unavailable");
    const auto first = publish(cache, 3);
    publish(cache, 4);
    publish(cache, 5);
    const auto info = cache.owned_info();
    require(!info.at("store").at("reclamationEnabled").get<bool>(), "dry-run mode misreported");
    require(info.at("store").at("audit").at("eligibleBytes").get<uint64_t>() > 0, "no conditional eligibility");
    const auto files = cache.disk_list();
    require(files.size() == 3, "dry-run unexpectedly removed state");
    for (const auto & file : files) { require(file.verdict == 1 && file.version == 2, "managed native header is invalid"); }
    require(cache.disk_read_best((root / "owned-kv-v1" / first).string(), 1.0f), "managed native read failed");
    require(cache.states.back().prompt.cache_read != nullptr, "RAM restore lost read lease");
    require(cache.states.back().prompt.cache_source == first, "RAM restore lost snapshot identity");
}

static void managed_reclamation(const fs::path & root) {
    setenv("VITRIOL_KV_OWNED_RECLAIM", "1", 1);
    server_prompt_cache cache(1, 128, root.string());
    configure_identity(cache);
    cache.owned_init();
    require(cache.owned != nullptr, "store lock was not released");
    require(cache.owned_info().at("store").at("reclamationEnabled").get<bool>(), "reclamation mode misreported");
    auto initial = cache.disk_list();
    require(initial.size() == 3, "restart discarded managed state");
    const auto old_id = initial.front().name.substr(std::string("owned-kv-v1/").size());
    auto reader = cache.owned->read(old_id);
    require(reader.has_value(), "read lease unavailable");
    publish(cache, 6);
    publish(cache, 7);
    require(fs::exists(root / "owned-kv-v1" / old_id), "active reader lost backing file");
    reader->reset();
    json state;
    require(cache.owned->call(json{{"op", "list"}}, state).has_value(), "no audit epoch");
    json result;
    require(cache.owned->call(json{{"op", "collect"}, {"epoch", state.at("epoch")}, {"dry_run", false}}, result).has_value(), "reclamation failed");
    require(result.at("removed").get<uint64_t>() <= 2, "reclamation exceeded bound");
    require(!fs::exists(root / "owned-kv-v1" / old_id), "obsolete unpinned state remained");
    require(cache.disk_list().size() == 2, "two rollback anchors not preserved");
}

static void ownership_schema() {
    common_params base;
    task_params params;
    auto fields = server_schema::make_llama_cmpl_schema(base, params);
    server_schema::field_eval_context context(params);
    json input{{"cache_owner_session", "s"}, {"cache_owner_branch", "b"}};
    for (const auto & field : fields) {
        if (std::string(field->name.front()).rfind("cache_owner_", 0) == 0) { field->eval(context, input); }
    }
    require(params.cache_owner_session == "s" && params.cache_owner_branch == "b", "schema dropped ownership metadata");
}

int main() {
    try {
        temporary_directory fixture;
        legacy_parity(fixture.path / "legacy");
        const auto root = fixture.path / "managed";
        managed_publication_and_read(root);
        managed_reclamation(root);
        ownership_schema();
        std::cout << "managed engine cache: publication, restart, read leases, bounded GC and legacy parity passed\n";
        return 0;
    } catch (const std::exception & error) { std::cerr << error.what() << '\n'; return 1; }
}
