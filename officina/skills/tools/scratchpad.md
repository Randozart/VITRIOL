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

Three sections:

- `facts` — evidence: exact numbers (t/s, ms, bytes), tensor shapes, argv
  that produced a result, observed error strings, config values. Verbatim.
- `leads` — open hypotheses and the next thing to try. One line each.
- `dead` — ruled-out ideas, stated briefly. Keeping a closed case here
  prevents re-chasing it after compaction; prune it when fully cold.

Rules:

- Write RIGHT AFTER you measure or decide something. A fact that lives only
  in the transcript will be compacted away; a fact in the notebook will not.
- A section you name is REPLACED wholesale. Pruning = rewriting the section
  without the stale lines. Delete anything no longer load-bearing — this is
  a notebook, not a history or a log.
- Hard cap (default 60 lines total). Over cap, the write is rejected with
  current counts: prune first, then re-add only the essentials.
- Do NOT put task tracking here (use update_tasks), long-term conventions
  (those belong in memory), or narrative ("then I tried..."). Facts, leads,
  dead — nothing else.

Example:

```tool
{"name": "scratchpad_write", "input": {"facts": [
  "sm_61 mmvq q6_K: 169 GB/s effective = 66% of 256 peak (sm_86: 81%)",
  "tg@43k q4_0 KV baseline 7.57 t/s; @54k 6.80 t/s (build 04a4f5f12)"
], "leads": ["mmvq GENERIC table on sm_61 may be the 15-pt loss"], "dead": ["Vulkan control build: parity on 3060, not the gap's cause"]}}
```
