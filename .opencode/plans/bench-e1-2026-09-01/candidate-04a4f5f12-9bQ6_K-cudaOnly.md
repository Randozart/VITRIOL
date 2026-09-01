| model                          |       size |     params | backend    | ngl |  fa | dev          |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | --: | ------------ | --------------: | -------------------: |
| qwen35 9B Q6_K                 |   7.03 GiB |     9.20 B | CUDA,Vulkan |  99 |   0 | CUDA0        |           pp512 |      1518.15 ± 53.59 |
| qwen35 9B Q6_K                 |   7.03 GiB |     9.20 B | CUDA,Vulkan |  99 |   0 | CUDA0        |           tg128 |         42.07 ± 0.04 |
| qwen35 9B Q6_K                 |   7.03 GiB |     9.20 B | CUDA,Vulkan |  99 |   1 | CUDA0        |           pp512 |      1546.86 ± 29.97 |
| qwen35 9B Q6_K                 |   7.03 GiB |     9.20 B | CUDA,Vulkan |  99 |   1 | CUDA0        |           tg128 |         42.24 ± 0.07 |

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
