# VITRIOL Store: mkfs + raw-zone store for expert/embed streaming

Date: 2026-09-23. Status: building.

## Objective

Convert the Qwen3.8-Flash-Next-UD-Q3_K_XL GGUF (3 shards, 89.98 GB) into an
exactly-sized raw-zone layout on Optane 900P p3 (279 GiB, raw, no filesystem),
then stream experts + n-gram embedding through it with O_DIRECT/io_uring at
zero page-cache cost. Kernel is not a tier.

## Exact model composition (offset-derived, validated 100%)

Method: GGUF tensor offsets are authoritative (next-offset - offset = exact
on-disk bytes). No type-table math. Sum == file size (89.98 vs 89.97 GB).

| component | size | count | tensors |
|---|---|---|---|
| experts | 56.67 GB | 576 | blk.*.ffn_{up,gate,down}_exps, *_shexp |
| n-gram embed | 29.48 GB | 1 | per_layer_token_embd.weight [160, 320001536] |
| attn | 2.58 GB | | wq/wk/wv/wo |
| other | 1.25 GB | | norms, gates, MTP |
| total | 89.98 GB | | == disk 89.97 GB |

Earlier "experts 100GB / embed 102GB" was 2x inflated by a buggy type table in
`libvitriol/gguf_reader.py` (out of sync with ggml.h; unknown dtypes default to
f32 4 B/elem). mkfs MUST use offsets or `ggml_type_size()`, never that dict.

## Zone layout on p3 (279 GiB kept as-is)

```
[0x0000] super 4K  magic VITS, ver, zone table, CRC (double-copied)
[index]  compact slot map (persistent; open() = 1 super + 1 index read)
[expert] 56.7 GiB  fixed 2 MiB slots (28340 slots, dense)  O(1) base+slot*stride
[embed]  29.5 GiB  per_layer_token_embd, sequential bulk
[kv]     64 GiB    reserved (P3), append-only ring
--------------------------------------------------------------
total    ~151 GiB of 279 GiB -> ~128 GiB slack for 2nd model / bigger KV
```

Zone layout exactly sized at mkfs from GGUF. Partition p3 kept at 279 GiB
(decision 2026-09-23): slack is free (no other use), buys model-2 + KV growth,
avoids repartition. XFS p2 (96G) stays the GGUF staging/archive.

## Store API (C++26, per docs/cpp26-conventions.md)

`vitriol-store.{h,cpp}`:
- `store_open(path) pre(path)` -> `std::expected<store*, errno_t>`
- `store_read_slot(st, slot, dst) pre(slot < count) pre(dst)` -> ssize_t (scalar, hot)
- `store_read_embed(st, offset, len, dst)` (sequential bulk)
- io_uring async path + pread fallback; fixed registered buffers; linked SQEs
- constexpr superblock layout + static_assert invariants
- O_DIRECT: 512B-aligned offsets/len/buffers (2 MiB slots satisfy trivially)

CMake: `set_property(SOURCE vitriol-store.cpp PROPERTY CXX_STANDARD 26)` +
`-fcontracts -fcontract-evaluation-semantic=ignore` on CXX only.

## mkfs tool

`vitriol-store-mkfs` (host C++26, links llama.cpp gguf.cpp):
1. open 3 shards, walk header + tensor table (offset-derived sizes)
2. compute zones: expert slots (dense), embed, kv ring
3. write super (2 copies, CRC), index zone, then stream experts + embed
4. verify: re-read super + index, spot-check slots

Access: udev rule grants randozart rw on /dev/nvme0n1p3 (GROUP disk or acl).

## Wire-in (next commits)

- `vitriol-buffer.cpp:164-208` disk branch -> store_open/read
- `vitriol-cuda-integration.cpp:862/957` LRU ensure/prefetch source ->
  store_read_slot
- profile keys: `store.path=/dev/nvme0n1p3`, `store.zones=experts|embed|kv`

## Verification

- mkfs: super CRC, index readback, slot count == 28340, embed bytes == 29.48 GB
- store bench: 3x64 token unit bench vs current mmap path
- filled-depth ladder (A-track certification later)

## Side fix

- `libvitriol/gguf_reader.py` type table: correct to ggml.h enum + type_traits
  (or make it offset-derived) so `vitriol calibrate` stops over-reporting.