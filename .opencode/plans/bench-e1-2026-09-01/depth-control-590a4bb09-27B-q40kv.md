| model                          |       size |     params | backend    | ngl | n_ubatch | type_k | type_v |  fa | ts           |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | -------: | -----: | -----: | --: | ------------ | --------------: | -------------------: |
| qwen35 27B Q3_K - Medium       |  12.86 GiB |    27.32 B | CUDA,Vulkan |  99 |       64 |   q4_0 |   q4_0 |   1 | 22.00/14.00  |           pp512 |        299.06 ± 0.57 |
| qwen35 27B Q3_K - Medium       |  12.86 GiB |    27.32 B | CUDA,Vulkan |  99 |       64 |   q4_0 |   q4_0 |   1 | 22.00/14.00  |           tg128 |         12.88 ± 0.00 |
| qwen35 27B Q3_K - Medium       |  12.86 GiB |    27.32 B | CUDA,Vulkan |  99 |       64 |   q4_0 |   q4_0 |   1 | 22.00/14.00  |  pp512 @ d43000 |        162.59 ± 0.26 |
| qwen35 27B Q3_K - Medium       |  12.86 GiB |    27.32 B | CUDA,Vulkan |  99 |       64 |   q4_0 |   q4_0 |   1 | 22.00/14.00  |  tg128 @ d43000 |          7.59 ± 0.01 |
| qwen35 27B Q3_K - Medium       |  12.86 GiB |    27.32 B | CUDA,Vulkan |  99 |       64 |   q4_0 |   q4_0 |   1 | 22.00/14.00  |  pp512 @ d54000 |        146.90 ± 0.17 |
| qwen35 27B Q3_K - Medium       |  12.86 GiB |    27.32 B | CUDA,Vulkan |  99 |       64 |   q4_0 |   q4_0 |   1 | 22.00/14.00  |  tg128 @ d54000 |          6.81 ± 0.01 |

build: 9723942ad (1572)
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
