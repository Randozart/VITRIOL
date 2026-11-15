include_directories(${CMAKE_CURRENT_SOURCE_DIR} ${CMAKE_CURRENT_BINARY_DIR})

# server-context containing the core server logic, used by llama-server and CLI

set(TARGET server-context)

add_library(${TARGET} STATIC
    server-chat.cpp
    server-chat.h
    server-task.cpp
    server-task.h
    server-queue.cpp
    server-queue.h
    server-common.cpp
    server-common.h
    server-context.cpp
    server-context.h
    server-stream.cpp
    server-stream.h
    server-tools.cpp
    server-tools.h
    server-mcp.cpp
    server-mcp.h
    server-schema.cpp
    server-schema.h
)

if (BUILD_SHARED_LIBS)
    set_target_properties(${TARGET} PROPERTIES POSITION_INDEPENDENT_CODE ON)
endif()

target_include_directories(${TARGET} PRIVATE ../mtmd)
target_include_directories(${TARGET} PRIVATE ${CMAKE_SOURCE_DIR})
target_link_libraries(${TARGET} PUBLIC llama-common mtmd ${CMAKE_THREAD_LIBS_INIT})

option(VITRIOL_CACHE_STORE "Build engine-owned Rust KV publication and retirement" OFF)
if(VITRIOL_CACHE_STORE)
    if(NOT UNIX OR NOT CMAKE_CXX_COMPILER_ID STREQUAL "GNU" OR CMAKE_CXX_COMPILER_VERSION VERSION_LESS 16)
        message(FATAL_ERROR "VITRIOL_CACHE_STORE requires POSIX and GCC 16 for host contracts")
    endif()
    find_program(VITRIOL_CARGO cargo REQUIRED)
    get_filename_component(VITRIOL_CACHE_STORE_ROOT "${CMAKE_SOURCE_DIR}/../vitriol-cache-store" ABSOLUTE)
    set(VITRIOL_CACHE_RUST_TARGET "${CMAKE_BINARY_DIR}/vitriol-cache-store")
    set(VITRIOL_CACHE_ARCHIVE "${VITRIOL_CACHE_RUST_TARGET}/release/libvitriol_cache_store.a")
    file(GLOB VITRIOL_CACHE_RUST_SOURCES CONFIGURE_DEPENDS "${VITRIOL_CACHE_STORE_ROOT}/src/*.rs" "${CMAKE_SOURCE_DIR}/../vitriol-core/src/*.rs")
    add_custom_command(OUTPUT "${VITRIOL_CACHE_ARCHIVE}"
        COMMAND ${CMAKE_COMMAND} -E env "CARGO_TARGET_DIR=${VITRIOL_CACHE_RUST_TARGET}" "RUSTFLAGS=$ENV{RUSTFLAGS} -C relocation-model=pic"
            ${VITRIOL_CARGO} build --release --locked --offline --manifest-path "${VITRIOL_CACHE_STORE_ROOT}/Cargo.toml"
        DEPENDS ${VITRIOL_CACHE_RUST_SOURCES} "${VITRIOL_CACHE_STORE_ROOT}/Cargo.toml" "${VITRIOL_CACHE_STORE_ROOT}/Cargo.lock"
        VERBATIM)
    add_custom_target(vitriol-cache-store-build DEPENDS "${VITRIOL_CACHE_ARCHIVE}")
    add_library(vitriol-cache-store STATIC IMPORTED)
    set_target_properties(vitriol-cache-store PROPERTIES IMPORTED_LOCATION "${VITRIOL_CACHE_ARCHIVE}")
    add_dependencies(vitriol-cache-store vitriol-cache-store-build)
    target_sources(server-context PRIVATE vitriol-owned-cache.cpp)
    target_include_directories(server-context PRIVATE "${VITRIOL_CACHE_STORE_ROOT}/include")
    target_compile_definitions(server-context PUBLIC VITRIOL_CACHE_STORE=1)
    target_compile_features(server-context PUBLIC cxx_std_26)
    target_compile_options(server-context PUBLIC -fcontracts -fcontract-evaluation-semantic=ignore)
    target_link_libraries(server-context PUBLIC vitriol-cache-store ${CMAKE_DL_LIBS} m)
    add_executable(vitriol-owned-cache-test EXCLUDE_FROM_ALL "${VITRIOL_CACHE_STORE_ROOT}/tests/engine-cache.cpp")
    target_link_libraries(vitriol-owned-cache-test PRIVATE server-context)
endif()

# llama-server-impl: server logic, reusable by app

set(TARGET llama-server-impl)

add_library(${TARGET}
    server.cpp
    server-http.cpp
    server-http.h
    server-models.cpp
    server-models.h
)
set_target_properties(${TARGET} PROPERTIES WINDOWS_EXPORT_ALL_SYMBOLS ON)

target_include_directories(${TARGET} PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_include_directories(${TARGET} PRIVATE ../mtmd ${CMAKE_SOURCE_DIR})
target_link_libraries(${TARGET} PUBLIC server-context llama-ui cpp-httplib ${CMAKE_THREAD_LIBS_INIT})

add_dependencies(${TARGET} llama-ui-assets)

if(LLAMA_TOOLS_INSTALL)
    install(TARGETS ${TARGET} LIBRARY)
endif()

# llama-server executable
# VITRIOL: shipped as `vitriol-server` (monorepo rename 2026-10-23); the
# CMake target keeps the upstream name so scripts/builds stay compatible.

set(TARGET llama-server)

add_executable(${TARGET} main.cpp)
set_target_properties(${TARGET} PROPERTIES OUTPUT_NAME vitriol-server)
install(TARGETS ${TARGET} RUNTIME)

target_link_libraries(${TARGET} PRIVATE llama-server-impl)
target_compile_features(${TARGET} PRIVATE cxx_std_17)
