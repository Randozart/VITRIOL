---
name: scratchpad-guidance
type: tool-guidance
target_tool: scratchpad_write
priority: 6
token_cost: 160
user-invocable: false
---
## Scratchpad — hot detective notebook

`scratchpad_write` maintains a project-scoped notebook of load-bearing
working state: `.officina/SCRATCHPAD.md`. It is re-injected into your
context EVERY turn from disk, so it survives compaction — you never need
to re-read source files just to recover a number you already measured.

Four sections:

- `facts` — evidence: exact numbers (t/s, ms, bytes), tensor shapes, argv
  that produced a result, observed error strings, config values. Verbatim.
- `context` — structured working data: error lists, file excerpts in progress,
  intermediate results. Bridge the gap when tool-result-clearer evicts reads.
  Write the actual extracted data here, not just summaries.
- `leads` — open hypotheses and the next thing to try. One line each.
- `dead` — ruled-out ideas, stated briefly. Keeping a closed case here
  prevents re-chasing it after compaction; prune it when fully cold.

Rules:

- Write RIGHT AFTER you measure or decide something. A fact that lives only
  in the transcript will be compacted away; a fact in the notebook will not.
- A section you name is REPLACED wholesale. Pruning = rewriting the section
  without the stale lines. Delete anything no longer load-bearing — this
  is a notebook, not a history or a log.
- Hard cap (default 120 lines total). Over cap, the write is rejected with
  current counts: prune first, then re-add only the essential ones.
- Do NOT put task tracking here (use update_tasks), long-term conventions
  (those belong in memory), or narrative ("then I tried..."). Facts, context,
  leads, dead — nothing else.
- `context` is for structured data that would otherwise be lost when
  tool-result-clearer stubs old reads. If you grep for error patterns and
  the results are important, write them into `context` immediately so they
  survive compaction.

Example:

```tool
{"name": "scratchpad_write", "input": {"facts": [
  "sm_61 mmvq q6_K: 169 GB/s effective = 66% of 256 peak (sm_86: 81%)",
  "tg@43k q4_0 KV baseline 7.57 t/s; @54k 6.80 t/s (build 04a4f5f12)"
], "context": [
  "vault.rs errors (30): E0599 method not found on Vault (lines 142, 187, 203...)",
  "main.rs errors (22): E0432 unresolved import vault::key_for (lines 8, 15, 31...)"
], "leads": ["mmvq GENERIC table on sm_61 may be the 15-pt loss"], "dead": ["Vulkan control build: parity on 3060, not the gap's cause"]}}
```
