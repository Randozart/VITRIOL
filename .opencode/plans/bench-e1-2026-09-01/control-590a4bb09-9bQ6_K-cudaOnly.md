| model                          |       size |     params | backend    | ngl |  fa | dev          |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | --: | ------------ | --------------: | -------------------: |
| qwen35 9B Q6_K                 |   7.03 GiB |     9.20 B | CUDA,Vulkan |  99 |   0 | CUDA0        |           pp512 |      1504.41 ± 47.02 |
| qwen35 9B Q6_K                 |   7.03 GiB |     9.20 B | CUDA,Vulkan |  99 |   0 | CUDA0        |           tg128 |         41.89 ± 0.07 |
| qwen35 9B Q6_K                 |   7.03 GiB |     9.20 B | CUDA,Vulkan |  99 |   1 | CUDA0        |           pp512 |      1526.68 ± 19.98 |
| qwen35 9B Q6_K                 |   7.03 GiB |     9.20 B | CUDA,Vulkan |  99 |   1 | CUDA0        |           tg128 |         42.07 ± 0.04 |

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
