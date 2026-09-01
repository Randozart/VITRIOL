| model                          |       size |     params | backend    | ngl | n_ubatch | type_k | type_v |  fa | ts           |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | -------: | -----: | -----: | --: | ------------ | --------------: | -------------------: |
| qwen35 27B Q3_K - Medium       |  12.86 GiB |    27.32 B | CUDA,Vulkan |  99 |       64 |   q4_0 |   q4_0 |   1 | 22.00/14.00  |           pp512 |        299.20 ± 1.04 |
| qwen35 27B Q3_K - Medium       |  12.86 GiB |    27.32 B | CUDA,Vulkan |  99 |       64 |   q4_0 |   q4_0 |   1 | 22.00/14.00  |           tg128 |         12.88 ± 0.00 |
| qwen35 27B Q3_K - Medium       |  12.86 GiB |    27.32 B | CUDA,Vulkan |  99 |       64 |   q4_0 |   q4_0 |   1 | 22.00/14.00  |  pp512 @ d43000 |        163.20 ± 0.12 |
| qwen35 27B Q3_K - Medium       |  12.86 GiB |    27.32 B | CUDA,Vulkan |  99 |       64 |   q4_0 |   q4_0 |   1 | 22.00/14.00  |  tg128 @ d43000 |          7.57 ± 0.01 |
| qwen35 27B Q3_K - Medium       |  12.86 GiB |    27.32 B | CUDA,Vulkan |  99 |       64 |   q4_0 |   q4_0 |   1 | 22.00/14.00  |  pp512 @ d54000 |        146.54 ± 0.83 |
| qwen35 27B Q3_K - Medium       |  12.86 GiB |    27.32 B | CUDA,Vulkan |  99 |       64 |   q4_0 |   q4_0 |   1 | 22.00/14.00  |  tg128 @ d54000 |          6.72 ± 0.02 |

build: 04a4f5f12 (1606)
=== VITRIOL Statistics ===
Mode: 0
LRU Hits: 0
LRU Misses: 0
LRU Hit Rate: 0.00%
LRU Evictions: 0
Predictor: none
Output Cache: none
Expert Pinning: none
Strategy: RAM Shot + LRU VRAM cache
===============================
