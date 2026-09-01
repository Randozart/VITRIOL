| model                          |       size |     params | backend    | ngl |  fa |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | --: | --------------: | -------------------: |
| qwen35 9B Q6_K                 |   7.03 GiB |     9.20 B | CUDA,Vulkan |  99 |   0 |           pp512 |        456.00 ± 5.62 |
| qwen35 9B Q6_K                 |   7.03 GiB |     9.20 B | CUDA,Vulkan |  99 |   0 |           tg128 |         34.89 ± 0.00 |
| qwen35 9B Q6_K                 |   7.03 GiB |     9.20 B | CUDA,Vulkan |  99 |   1 |           pp512 |        462.21 ± 1.44 |
| qwen35 9B Q6_K                 |   7.03 GiB |     9.20 B | CUDA,Vulkan |  99 |   1 |           tg128 |         34.87 ± 0.02 |

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
