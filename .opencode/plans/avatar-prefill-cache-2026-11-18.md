# 2026-11-18: private PTL avatar prefill comparison

Base 9b3354f; public avatar and original 20B service remain unchanged. Isolated
CPU-only GCC16 build with VITRIOL_CACHE_STORE=ON exposed the owned-cache
disk_files aggregate missing the atime field introduced by 6dd6086. Include
metadata.st_atime, matching the legacy disk scan. No ownership or eviction
policy change. Gate: rebuild server and vitriol-owned-cache-test; test Rust
cache-store. Avatar repository owns HTTP cache/timing evidence and Intel/WBSO
report; no performance claim from compilation. Original checkout dirty vendor
LICENCE.md is preserved.
