# Optimization: prompt-cache checkpoint pipelining

Status: **validated**.
Lever: built-in server path (`server-context.cpp`), no config knob.

## What it is

Two fixes that avoid full re-prefill on repeat requests:

1. **Checkpoint-erasure removal** (`server-context.cpp:2662-2673`): stopped
   erasing all checkpoints with `pos_max > pos_next` after a restore. A
   checkpoint at position X is valid for ANY future request whose first X tokens
   match; the `n_ctx_checkpoints=32` capacity already evicts naturally.
   Aggressive erasure destroyed valid intermediate checkpoints → massive
   re-prefill on the next request.
2. **Idle-slot state save** (`server-context.cpp:2686`): when a slot goes idle,
   its full state (including Recurrent State buffers for SSM/Gated Delta Net
   layers) is saved to the prompt cache and KV freed. Restored on the next
   matching request without full re-prefill. Mitigates the "hybrid amnesia"
   issue where Qwen3.6's 62.81 MiB RS buffer forced `do_reset = true`.

## Measured

Prefill time reduction: **~90% for context-heavy tasks**. Validated by build +
integration (server-context.cpp), not a timing A/B.

Source: `.opencode/plans/prefill-optimization-plan.md`.

## Undo

This is intrinsic server behavior, not a profile knob. Reverting means restoring
the checkpoint-erasure loop; not recommended — it caused re-prefill storms.
