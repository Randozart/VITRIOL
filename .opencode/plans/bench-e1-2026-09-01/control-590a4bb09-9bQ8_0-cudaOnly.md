| model                          |       size |     params | backend    | ngl |  fa | dev          |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | --: | ------------ | --------------: | -------------------: |
| qwen35 9B Q8_0                 |   9.10 GiB |     9.20 B | CUDA,Vulkan |  99 |   0 | CUDA0        |           pp512 |      1721.42 ± 78.52 |
| qwen35 9B Q8_0                 |   9.10 GiB |     9.20 B | CUDA,Vulkan |  99 |   0 | CUDA0        |           tg128 |         36.31 ± 0.07 |
| qwen35 9B Q8_0                 |   9.10 GiB |     9.20 B | CUDA,Vulkan |  99 |   1 | CUDA0        |           pp512 |      1763.13 ± 46.60 |
| qwen35 9B Q8_0                 |   9.10 GiB |     9.20 B | CUDA,Vulkan |  99 |   1 | CUDA0        |           tg128 |         36.50 ± 0.06 |

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
