# VITRIOL C++ Conventions

Goal: **safest + fastest, minimum footgun, at zero runtime cost.**
Every mandate in this guide is zero-runtime-cost under the shipped toolchain
(GCC 16.2 `-std=c++26 -fcontracts -fcontract-evaluation-semantic=ignore`).
Anything that cannot be made zero-cost is banned from hot paths.

## Toolchain floor (verified 2026-09-23)

| component | version | supports |
|---|---|---|
| g++ (host .cpp) | 16.2.1 | C++26, contracts (P2900R14), mdspan, span, expected, concepts, `[[assume]]` |
| nvcc (device .cu + headers it parses) | 13.4 | **max C++20**, concepts; NO contracts, NO mdspan |
| ggml global standard | C++17 | untouched — `ggml/src/CMakeLists.txt:609` `# don't bump` |

TU classes:

| TU | standard | contracts | allowed |
|---|---|---|---|
| `vitriol-*.cpp` bodies | **C++26** (per-file property) | **yes** | full set |
| `vitriol-store.*` | **C++26** | **yes** | full set |
| headers incl. by `.cu` (`vitriol-cuda-integration.h`, `vitriol-buffer.h`) | C++20 | **no** | concepts, span; no mdspan/contracts |
| `.cu` / `ggml-cuda.cu` | C++17/20 | **never** | nvcc floor |

CMake wiring (ggml-cuda target): `set_property(SOURCE vitriol-*.cpp PROPERTY
CXX_STANDARD 26)`, `-fcontracts` on CXX-only compile options (never CUDA),
`-fcontract-evaluation-semantic=ignore` default.

## Zero-cost contract (the guiding rule)

`-fcontract-evaluation-semantic=ignore` compiles contracts to **nothing**
(verified: `f(int)` = `lea; ret`, no branch, no call). The contract text stays
in source — self-documenting, toolable — and a separate `enforce`/`observe`
build re-enables checks with **zero source edits**.

- Contracts on every VITRIOL public host function:
  `pre(...)` on entry invariants, `post(r: ...)` on guarantees,
  `contract_assert` at post-state sites.
- `static_assert` / `constexpr` / `consteval` for compile-time invariants
  (superblock layout, magic, stride) — always free, always enforced.
- **Predicates MUST be side-effect-free** (unspecified whether evaluated).
- Contracts NEVER in nvcc-parsed headers or device code.
- Never ship `enforce` on hot paths.

## Bans (footguns, all concentrated in the hot MoE path)

1. **Pointer-as-integer keys** — `uintptr_t tensor_base` as LRU key. Use a
   typed pointer key (`const void*`) — same hash cost, kills int→ptr UB.
   Never dynamic `std::span<T>` as a hot-map key (adds a size member → hash
   cost); static `std::span<T, N>` is 8B (ptr-only) and acceptable.
   **Hasher**: libstdc++ `std::hash<const void*>` does NOT mix bits (raw
   `reinterpret_cast<size_t>`, functional_hash.h:114). With `std::unordered_map`
   (prime buckets) aligned-pointer clustering is mitigated; with ANY
   power-of-2 flat map (flat_hash_map/robin-hood) use a bit-mixing hasher
   (splitmix64 or `ptr * 2654435761U` Fibonacci).
2. **C-style casts** (`(float*)`, `(uint32_t)`) — use `static_cast` /
   `reinterpret_cast`. `reinterpret_cast` only where ABI demands (CE regs,
   packed structs).
3. **Raw `void* + offset + size` buffer math** — `std::span<T>` /
   `std::mdspan` (default accessor, unchecked `[]`). Zero overhead.
4. **Unchecked expert indexing** — `tensor_base + expert_idx * expert_size`
   without a bounds contract. `pre(expert_idx < count)` + `std::array`.
5. **Raw alloc/free without RAII** — `mmap`/`cuMemAlloc`/`cudaHostRegister`
   paired with `std::unique_ptr`/custom-deleter guard. Zero overhead.
6. **`#define` constants** — `constexpr`/`consteval`. (Bitfield doc-tokens
   like `16:0` stay comments — not valid C++.)

## Error handling

- **Boundary / control-plane** APIs return `std::expected<T, CUresult/cudaError_t>`.
- **ABI caveat (SYSV AMD64)**: return values ≤ 16 bytes go in `%rax:%rdx`;
  larger → hidden `sret` pointer + stack traffic. `expected<uint64_t, CUresult>`
  is 16B (fits); `expected<BigStruct, E>` spills. Keep `sizeof(expected) <= 16`
  or the boundary type is small.
- **Hot inner-loop micro-APIs** MUST NOT return `expected` — return a primitive
  scalar (bare pointer with `nullptr` on error, or error enum) or use an
  out-param, to avoid per-iteration stack traffic in MoE routing.
- Contracts cover preconditions/programming bugs; `expected` covers dynamic
  CUDA failure.
- BAN the `sigjmp_buf` SEGV-guard pattern (d2b/spike) — replace with
  `pread`/ioctl + `expected`.

## Concurrency

- `g_vitriol_config`: immutable after init (document).
- Counters: accumulate locally, publish once. `std::atomic` only with
  `relaxed` where eventual consistency suffices — **seq_cst atomics are NOT
  free** (`lock` instr on hot path); otherwise drop.

## Performance rules

- `[[assume(expr)]]` for proven invariants on hot paths — removes checks.
  (UB if the assumption is ever wrong — only after a contract proves it.)
- **Ignored contracts are NOT optimization assumptions** (verified GCC 16.2:
  an `idiv` guarded by `pre(x > 0)` survives at `-O2` under `ignore` — the
  compiler does not use the ignored predicate for codegen). To get both the
  check and the hint from one condition, use the shared helper:

  ```cpp
  #define VITRIOL_ASSUME_FROM_CONTRACT(cond) \
      [[assume(cond)]]
  // usage:
  int f(int idx) pre(idx < 8) {           // checked in enforce/CI builds
      VITRIOL_ASSUME_FROM_CONTRACT(idx < 8); // used by optimizer in shipped builds
      return table[idx];
  }
  ```

  Never write the raw `[[assume(cond)]]` twice by hand.
- Bounds-checked access (`.at()`, checked accessors) cold paths only.
- Alignment verification:
  - Dynamic buffer alignment (runtime pointer): `pre(std::is_sufficiently_aligned<64>(ptr))`
    — zero-cost under `ignore`. `is_sufficiently_aligned` inspects the pointer
    value, so it CANNOT be a `static_assert`.
  - Type/layout alignment: `static_assert(alignof(T) >= 64)` (compile-time, free).

## Build hygiene

- `main()`-bearing harnesses (`vitriol_ce_d2b.cpp`, `vitriol_ce_spike.cpp`)
  must NOT be in the ggml-cuda source GLOB (duplicate-symbol link collision).
  Exclude in CMake; build standalone with g++.
- CI/enforce build: `-fcontract-evaluation-semantic=enforce -Wall -Wextra
  -Wpedantic -Werror` + clang-tidy (cppcoreguidelines-pro-type-cstyle-cast,
  no-int-to-ptr, bounds) on host `.cpp` only.

## Availability caveat

"Zero cost" is verified on **GCC 16.2 only**. clang has no contracts yet
(`-fcontracts` rejected) — portability of the free guarantee is future work.
`std::indirect`/`std::polymorphic` are NOT in GCC 16 libstdc++ — out of scope.

## Flag verification note

Contract-semantic flag on this GCC 16.2 is exactly
`-fcontract-evaluation-semantic=ignore` (`-fcontract-semantic=ignore` and
`-fcontracts-ignore` are REJECTED). Before any toolchain bump, re-check with
`g++ --help=common | grep fcontract` — P2900R14 semantic-flag naming has
churned across snapshots.