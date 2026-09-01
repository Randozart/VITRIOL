| model                          |       size |     params | backend    | ngl |  fa |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | --: | --------------: | -------------------: |
| qwen35 9B Q8_0                 |   9.10 GiB |     9.20 B | CUDA,Vulkan |  99 |   0 |           pp512 |       522.17 ± 16.78 |
| qwen35 9B Q8_0                 |   9.10 GiB |     9.20 B | CUDA,Vulkan |  99 |   0 |           tg128 |         27.68 ± 0.08 |
| qwen35 9B Q8_0                 |   9.10 GiB |     9.20 B | CUDA,Vulkan |  99 |   1 |           pp512 |        531.76 ± 8.84 |
| qwen35 9B Q8_0                 |   9.10 GiB |     9.20 B | CUDA,Vulkan |  99 |   1 |           tg128 |         26.93 ± 0.14 |

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
