| model                          |       size |     params | backend    | ngl |  fa | dev          |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | --: | ------------ | --------------: | -------------------: |
| qwen35 9B Q8_0                 |   9.10 GiB |     9.20 B | CUDA,Vulkan |  99 |   0 | CUDA0        |           pp512 |      1721.66 ± 89.31 |
| qwen35 9B Q8_0                 |   9.10 GiB |     9.20 B | CUDA,Vulkan |  99 |   0 | CUDA0        |           tg128 |         36.36 ± 0.03 |
| qwen35 9B Q8_0                 |   9.10 GiB |     9.20 B | CUDA,Vulkan |  99 |   1 | CUDA0        |           pp512 |      1781.64 ± 46.35 |
| qwen35 9B Q8_0                 |   9.10 GiB |     9.20 B | CUDA,Vulkan |  99 |   1 | CUDA0        |           tg128 |         36.55 ± 0.03 |

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
