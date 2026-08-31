# Trismegistus — Custom Agent Harness: VITRIOL + Hermes + little-coder + Aider techniques

**Name:** Trismegistus — Hermes Trismegistus, binder of the stack's three roles: messenger (Hermes gateway), alchemist (VITRIOL engine), scribe (memory/skills). See REPORT-02-EXPANSION.md §1.

**Goal:** Best possible local coding agent harness for Qwen 3.8 27B on dual GPU (RTX 3060 + GTX 1070 Ti), maximizing ~54k usable tokens.

Techniques sourced from: little-coder (context extensions), Aider (repo map, async compaction), OpenHands (batch-aware condensation, event-sourced state), VITRIOL (sparse KV, LULL scoring), Hermes (memory, skills, gateway). Rounds 2-3 additions (Claude Code, ReWOO, LLMLingua, Letta, OpenCode, OmniRoute) in REPORT-02-EXPANSION.md.

---

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                  GATEWAY LAYER                       │
│              Hermes (MIT, always-on)                 │
│  Telegram / Discord / Slack / CLI / Dashboard        │
│  Memory (FTS5 + Hermetis) / Skills / Cron            │
│  Plugin: vitriol-bridge → dispatches to coding agent │
└──────────────────────┬──────────────────────────────┘
                       │ /lc <task> or auto-dispatch
                       ▼
┌─────────────────────────────────────────────────────┐
│                SCAFFOLD LAYER                        │
│         little-coder (Apache-2.0, Node)              │
│  12 context-optimization extensions                  │
│  + repo-map (tree-sitter + PageRank) [from Aider]    │
│  + async background compaction [from Aider]          │
│  + batch-aware condensation [from OpenHands]         │
│  Sub-coder dispatch (isolation)                      │
│  Plan mode (research separate from implementation)   │
│  Selective injection (300 tok skill + 200 tok knowledge) │
│  Write guard / read guard / thinking budget          │
└──────────────────────┬──────────────────────────────┘
                       │ OpenAI-compatible API
                       ▼
┌─────────────────────────────────────────────────────┐
│                  ENGINE LAYER                        │
│         VITRIOL llama-server (Apache-2.0)            │
│  Qwen 3.8 27B Q3_K_M on dual GPU                    │
│  Sparse KV + LULL scoring (96K+ filled tokens)       │
│  Slot checkpoints (crash recovery)                   │
│  NO SHIM — scaffold owns context management          │
└─────────────────────────────────────────────────────┘
```

**Key design decision:** VITRIOL runs WITHOUT the shim. The scaffold (little-coder + borrowed techniques from Aider/OpenHands) manages context. VITRIOL's native LULL sparse KV eviction handles GPU-level cache efficiency independently.

---

## Phase 1: License Change (VITRIOL + llama.cpp fork)

**Scope:** Change from GPL-2.0 to Apache-2.0 across two repos (both owned by the same author).

### Files to modify in VITRIOL

| # | File | Change |
|---|------|--------|
| 1 | `VITRIOL/LICENSE` | Replace GPL-2.0 full text with Apache-2.0 full text |
| 2 | `VITRIOL/llama.cpp/LICENSE` | Replace GPL-2.0 full text with Apache-2.0 full text |
| 3 | `VITRIOL/AGENTS.md` | Update licensing section — remove GPL-2.0 compatibility table, replace with Apache-2.0 terms |
| 4 | `VITRIOL/llama.cpp/AGENTS.md` | Same — update licensing section |
| 5 | `VITRIOL/docs/ARCHITECTURE.md` | Update provenance/license references |
| 6 | `VITRIOL/docs/REBIS.md` | Update licensing references |
| 7 | `VITRIOL/docs/provenance/kimi-k3-in-c.md` | Update license note |
| 8 | `VITRIOL/docs/provenance/pymander.md` | Update license note |
| 9 | `VITRIOL/docs/archive/PROJECT_STRUCTURE.md` | Verify correct (already says Apache 2.0 — stale) |
| 10 | `VITRIOL/EXPERIMENT_LOG.md` | Update any GPL references |
| 11 | `VITRIOL/alka-handoff/HANDOFF.md` | Update "Apache 2.0 with Runtime Exception" to plain Apache 2.0 |

### Files already Apache-2.0 (no change needed)

- `VITRIOL/alka-executor/gguf-offset-resolver.c`
- `VITRIOL/alka-executor/vitriol_copy_engine.h`
- `VITRIOL/alka-executor/vitriol_copy_engine.c`
- `VITRIOL/alka-executor/vitriol_alka_user.h`
- `VITRIOL/alka-executor/executor.c`
- `VITRIOL/alka-handoff/vitriol_alka.h`
- `VITRIOL/vitriol-daemon/vitriol_alka_kernel.h`

### Files that stay as-is

- Kernel modules (`vitriol-daemon/vitriol.c`, `artifacts/moore_stream.c`, `artifacts/test_simple.c`) — `MODULE_LICENSE("GPL")` stays for Linux kernel symbol access
- llama.cpp upstream Apache-2.0 files (Intel SYCL with LLVM exception, OpenVINO frontend) — separate upstream code
- MIT-licensed vendored deps (cpp-httplib, nlohmann/json, gguf-py) — all GPL-compatible

---

## Phase 2: Clone + Configure little-coder

### Source

- **Repo:** https://github.com/itayinbarr/little-coder
- **License:** Apache-2.0
- **Language:** TypeScript (Node.js 22.19+)
- **Built on:** pi agent framework (@earendil-works/pi-coding-agent)

### Install

```bash
cd /home/randozart/Desktop/Projects
git clone https://github.com/itayinbarr/little-coder.git
cd little-coder
npm install
```

### Verify against VITRIOL

- little-coder's `models.json` points to `http://127.0.0.1:8279/v1` (VITRIOL's llama-server)
- Config already exists at `~/.config/little-coder/models.json`
- Test: `little-coder --model llamacpp/qwen3.8-27b` against running VITRIOL server

---

## Phase 3: Integration

### 3a. Hermes Plugin: VITRIOL Backend

**New file:** `hermes-agent/plugins/vitriol-backend/plugin.yaml` + `__init__.py`

Registers:
- **Model provider** — VITRIOL inference via `ctx.register_provider(ProviderProfile(...))` pointing at `http://127.0.0.1:8279/v1`
- **Pre/post tool call hooks** — Safety gates using VITRIOL's safety levels (1=safe, 2=dma, 3=raw pci)
- **MCP server registration** — Expose VITRIOL's `/slots`, `/metrics`, `/props` as MCP tools

### 3b. Hermes Plugin: little-coder Bridge

**New file:** `hermes-agent/plugins/little-coder-bridge/plugin.yaml` + `__init__.py`

Registers:
- **Tool** — `dispatch_little_coder(goal, context)` that spawns a little-coder sub-process
- **Slash command** — `/lc <task>` to hand off coding tasks to little-coder
- **Hook** — `pre_llm_call` that detects coding-heavy prompts and suggests little-coder delegation

### 3c. little-coder Extensions for VITRIOL

**New extensions in** `little-coder/.pi/extensions/`:

1. **`vitriol-status/index.ts`** — Exposes VITRIOL slot/GPU/VRAM status as a tool
2. **`vitriol-memory/index.ts`** — Bridges little-coder's session to VITRIOL's Hermetis memory DB
3. **`hermes-bridge/index.ts`** — Allows sub-coders to call back to Hermes

### 3d. Unified Config

**New file:** `~/.config/trismegistus/config.yaml` (full shape in REPORT-02 §2, §3.5; cert-wiring per §2.4)

```yaml
engine:
  vitriol:
    endpoint: http://127.0.0.1:8279
    model: Qwen3.8-27B-Q3_K_M
    profile: qwen38-master
    shim:
      enabled: false   # Trismegistus default; other profiles may re-enable
    cert_required: true  # refuse uncertified model/engine combos

gateway:
  hermes:
    enabled: true
    platforms: [telegram, cli]
    memory: hermetis
    skills: auto-curate

coding:
  little_coder:
    enabled: true
    subcoder_concurrency: 2
    plan_mode: true

safety:
  approval_required: [force_push, schema_migration, delete_unbacked]
  max_concurrent_subcoders: 3
  loop_breaker: { threshold: 3, action: inject_rethink }
```

---

## Token Budget Breakdown (~54K usable tokens)

| Allocation | Tokens | Managed by | Technique Source |
|------------|--------|------------|------------------|
| System prompt (little-coder) | ~7,000 | little-coder cold-start | little-coder |
| **Repo map** | **~1,000** | **tree-sitter + PageRank** | **Aider** |
| Active conversation | ~18,000 | **async** background compaction | Aider + little-coder |
| Tool outputs (guarded) | ~4,000 | **batch-aware** condensation | OpenHands + little-coder |
| Skill cards (selective) | ~300/turn | little-coder skill-inject | little-coder |
| Knowledge blocks (scored) | ~200/turn | little-coder knowledge-inject | little-coder |
| Thinking budget | ~4,096 | little-coder thinking-budget extension | little-coder |
| Memory retrieval | ~500 | Hermes FTS5 → compact summary | Hermes |
| Sub-coder reports | ~500 each | little-coder sub-agent isolation | little-coder |
| **Headroom** | **~19,400** | Buffer for growth | Net +3K vs naive approach |

**Total context engineering gain:** ~12K tokens saved vs. naive approach (repo map saves 5-10K of blind reads; async + batch-aware condensation saves 2-3K of stalls and compression waste).

---

## Context Management: What Each System Handles

| Responsibility | System | Mechanism | Source |
|----------------|--------|-----------|--------|
| Codebase structure | repo-map MCP | tree-sitter + PageRank | Aider |
| What enters context | little-coder | 12 extensions (selective injection, guards, compaction) | little-coder |
| Async compaction | little-coder + Aider | Background summarization, never blocks | Aider |
| Condensation quality | little-coder + OpenHands | Batch-aware action-observation pairs | OpenHands |
| KV cache efficiency | VITRIOL | LULL scoring, sparse eviction, tq3_0 KV quant | VITRIOL |
| Cross-session memory | Hermes | FTS5 search, Hermetis DB, skill curator | Hermes |
| Crash recovery | VITRIOL | Slot checkpoints every 300s, proactive bounce | VITRIOL |
| Gateway/persistence | Hermes | Telegram always-on, cron, multi-platform | Hermes |

### Aider Techniques (adapted)

| # | Technique | Mechanism | Token Savings | Why It Matters |
|---|-----------|-----------|---------------|----------------|
| 1 | **Repository map (tree-sitter + PageRank)** | Parse every file, build dependency graph, rank by PageRank, inject ~1K of structural signatures | Replaces 5-10K of blind file reads | Model knows WHERE code lives before reading it |
| 2 | **Async background summarization** | Compaction runs on background thread, never blocks main loop | 0 tokens, eliminates stalls | On 11 t/s model, sync compaction burns seconds |
| 3 | **Dual-buffer conversation** | `done_messages` (compressed) vs `cur_messages` (raw, active) | Cleaner compression boundary | Current turn always has full context |
| 4 | **Dynamic repo map budgeting** | 8x budget when no files in chat, shrinks when files added | Adaptive | Maximum awareness when exploring, precise when editing |
| 5 | **Context overflow recovery** | Assistant prefill retry on FinishReasonLength | Prevents hard crashes | Partial response recovered, not lost |

### OpenHands Techniques (adapted)

| # | Technique | Mechanism | Token Savings | Why It Matters |
|---|-----------|-----------|---------------|----------------|
| 6 | **Batch-aware condensation** | Compress action-observation pairs together, not individual messages | Better compression ratio | Preserves causal chain even when compressed |
| 7 | **Skills as immutable context** | Skills injected as structured blocks, never compressed away | ~500 tokens always available | Domain knowledge persists across compaction |
| 8 | **Event-sourced state** | Every action/observation is immutable event, conversation reconstructed by replay | Perfect audit trail | Can reconstruct any point in session history |

### little-coder's 12 Context Optimization Techniques

| # | Technique | Mechanism | Token Savings |
|---|-----------|-----------|---------------|
| 1 | KV-cache-preserving injection | Move blocks from system prompt to tail message | Re-processes 0 history tokens |
| 2 | Selective skill cards | 3-priority scoring + 300-token budget | 1400 → 150-300 tokens/turn |
| 3 | Scored knowledge injection | Keyword/bigram scoring + 200-token budget | 2400 → 0-200 tokens/turn |
| 4 | Sub-coder isolation | Separate processes, only report enters parent | 50K internal → 500 token report |
| 5 | Plan mode separation | Research in separate context, implementation fresh | 50K research → 1K plan seed |
| 6 | Mid-run compaction | 80% threshold watchdog, loop guard | Prevents overflow |
| 7 | Read guard | Trim large reads to 30 lines | 6000 → 200 tokens per overflow |
| 8 | Thinking budget cap | Abort thinking at 4096 tokens | Saves 2000-4096 tokens/firing |
| 9 | Write guard | Block whole-file rewrites, redirect to edit | Prevents 57% of exercises from burning full output |
| 10 | Evidence compaction | Bridge message preserves evidence across compaction | 50 tokens prevents re-gathering |
| 11 | Turn cap | Hard abort at N turns per exercise | Caps runaway token consumption |
| 12 | Controlled cold-start | 27 extensions, not pi's full set | 20K+ → 7K initial tokens |

### VITRIOL's Context Management

| Mechanism | Effect |
|-----------|--------|
| LULL attention-probe scoring | Identifies which KV pages matter per decode step |
| Sparse KV eviction | Evicts lowest-scored cells, enables 96K+ filled tokens on 16 GiB |
| TurboQuant KV (tq3_0) | 3.5 bpw vs 4 bpw = -22% KV memory |
| Slot checkpoints | Crash recovery in ~5s via disk persistence |
| Proactive bounce | Restart before OOM (MemAvailable < 250 MiB) |
| Frozen prompt caching | System prefix never reprocessed |

### Hermes's Context Management

| Mechanism | Effect |
|-----------|--------|
| Prompt caching (sacred) | Cached prefix reused every turn — never break |
| FTS5 session search | Cross-session recall without bloating active context |
| Skill lazy-loading | Skills loaded on-demand, not all at once |
| Trajectory compression | Offline: 50-96% of middle turns → 750 token summary |

---

## Enhancements Beyond the Blueprint

### 0. Repository Map (from Aider) — HIGHEST PRIORITY

**Problem:** little-coder requires the model to explicitly `read` files to understand the codebase. On a 54K budget, blind file reading wastes thousands of tokens.

**Solution:** Deploy the repo-map MCP server (https://github.com/noambinabout-boop/repo-map, Apache-2.0) as an MCP tool registered in Hermes.

```yaml
# In ~/.config/opencode/opencode.jsonc or hermes config
mcp_servers:
  repo-map:
    command: "python"
    args: ["/path/to/repo-map/server.py"]
```

**How it works:**
1. `index("/path/to/project")` — tree-sitter parses all files, builds dependency graph
2. `where_is("MyClass")` — PageRank-ranked lookup by symbol name
3. `outline("src/app.py")` — ~95% fewer tokens than reading the file
4. `get_symbol("src/app.py", "MyClass")` — full body of one symbol
5. `who_references("function_name")` — what might break if signature changes

**Token cost:** ~1K tokens for the structural overview. Replaces 5-10K of exploratory reads.

**Integration options:**
- **Option A:** Register as Hermes MCP server → available to all Hermes sessions + little-coder sub-coders
- **Option B:** Build as little-coder extension → native integration with plan mode
- **Option C:** Both — MCP for Hermes, extension wrapper for little-coder

### 1. Unified Context Monitor

**Problem:** little-coder tracks context usage, VITRIOL tracks KV fill, neither knows what the other is doing.

**Solution:** Lightweight sidecar polling:
- little-coder's context usage (via pi's compaction state)
- VITRIOL's `/slots` endpoint (KV fill percentage)
- Hermes' session token count

Triggers:
- little-coder > 70% AND VITRIOL KV > 80% → suggest plan mode
- little-coder > 85% AND VITRIOL KV > 90% → force compaction
- VITRIOL KV > 95% → alert, reduce context or switch quant

### 2. Adaptive Model Routing

Route tasks by complexity:
- Simple edits → skip agent, direct edit
- Complex refactoring → Qwen 3.8 27B via little-coder
- Research → sub-coders (isolated context)

### 3. Skill Cross-Pollination

Bidirectional converter:
- little-coder skill `.md` → Hermes `SKILL.md` (add frontmatter)
- Hermes skill → little-coder knowledge entry (extract keywords)

### 4. Output Quality Gate

Pre-commit validation:
- Syntax check generated code before writing
- Praetor contract check (if file has annotations)
- Diff review with side-by-side
- Auto-run affected tests after code changes

### 5. Context-Aware Prompt Engineering

Dynamic prompt sizing based on model context:
- 100k context: full tool schemas, detailed descriptions
- 16k context: abbreviated schemas, minimal descriptions

---

## Execution Order

**Superseded by REPORT-02-EXPANSION.md §7 (25 steps, final).** This table (Round 1, 13 steps) is retained for provenance; the 25-step order adds VITRIOL-side engine work (steps 3-4), OpenCode steals (10-11), OmniRoute steals (8, 18-21), and ReWOO last (22).

| Step | Task | Dependencies | Unlocks |
|------|------|--------------|---------|
| 1 | License change (VITRIOL + llama.cpp) | None | Legal compatibility |
| 2 | Clone + test little-coder against VITRIOL | Step 1 | Coding agent ready |
| 3 | Deploy repo-map MCP server | Step 1 | Codebase structure awareness (~1K tokens) |
| 4 | Run VITRIOL WITHOUT shim | Step 2 | scaffold owns context |
| 5 | Hermes plugin: VITRIOL model provider | Step 4 | Hermes uses VITRIOL for non-coding tasks |
| 6 | Hermes plugin: little-coder dispatch | Step 5 | `/lc` command + auto-dispatch |
| 7 | little-coder extension: hermes-bridge | Step 6 | Sub-coders query Hermes memory |
| 8 | little-coder extension: async compaction | Step 2 | Non-blocking compaction (from Aider) |
| 9 | little-coder extension: batch-aware condensation | Step 2 | Action-observation pair compression (from OpenHands) |
| 10 | Unified config | Step 7 | Single config for all three |
| 11 | Context monitor sidecar | Step 10 | Prevents cross-system overflow |
| 12 | Skill cross-pollination sync | Step 10 | Skills available everywhere |
| 13 | Observability dashboard | Step 10 | Prometheus + Hermes metrics unified |

---

## Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| little-coder pi framework drift | Extensions break | Pin `@earendil-works/pi-coding-agent` version |
| VITRIOL restart during task | Context lost | Slot checkpoints every 300s; little-coder loop-breaker |
| Hermes cache invalidation | Cost spike | All hooks must be cache-safe — no mid-conversation mutations |
| Local model quality | Bad code edits | Quality gate catches syntax; Praetor catches contracts |
| License change breaks distribution | Legal | Both repos owned — no external distributors to notify |
| repo-map stale after large refactor | Structural overview drifts | Re-index on workspace change (file watcher trigger) |
| Async compaction race condition | Summary applied at wrong boundary | Dual-buffer ensures current turn never mid-compaction |
| Batch-aware condensation loses nuance | Action-observation pair merged too aggressively | Keep last N pairs verbatim (OpenHands pattern) |

---

## License Compatibility Matrix

| Component | License | Compatible with |
|-----------|---------|-----------------|
| Hermes-Agent | MIT | Everything |
| little-coder | Apache-2.0 | MIT, Apache-2.0, GPL-2.0+ |
| VITRIOL (after change) | Apache-2.0 | MIT, Apache-2.0, GPL-2.0+ |
| llama.cpp fork (after change) | Apache-2.0 | Same |
| repo-map MCP | Apache-2.0 | Same |
| smolagents (reference) | Apache-2.0 | Same |

All components are Apache-2.0 or more permissive. No copyleft conflicts.

---

# Round 2: Additional Prior Art (Complete Research)

Second pass over the landscape found five more techniques worth mining, plus several explicitly rejected. This section is the complete record.

---

## R2.1 Tool-Result Clearing (Claude Code "context editing") — CRITICAL

**Source:** Anthropic `clear_tool_uses_20250919` beta, "Managing context on the Claude Developer Platform" (Sept 2025). Claude Code v2.1.x applies this internally at 83.5% context fill with a 33K reserved summarization buffer.

**The problem:** In agentic loops, the single biggest context waste is **stale tool results**. A file read at turn 3, a test-run output at turn 5, a directory listing at turn 7 — all verbose, all still sitting in context at turn 25, all already reflected in the model's subsequent messages and edits. The raw payload serves no purpose after consumption, but standard compaction treats it as normal conversation.

**Anthropic's measured numbers:** Combined with the memory tool: **84% token reduction** and **39% performance improvement** in a 100-turn agentic-search evaluation. Context editing alone: 29% improvement.

**Mechanism:** When accumulated context crosses a threshold:
1. Walk the conversation, find all tool_use / tool_result pairs
2. Keep the last N uses verbatim (N typically 3-6 — the model still needs recent results to reason about)
3. Replace older tool results with a stub: `[tool result cleared: 2,847 tokens — file was src/main.rs, subsequently edited twice]`
4. Optionally clear the tool *inputs* too (the full file content sent to a write tool)
5. **Exclude memory/todo tools from clearing** — their contents are the survival mechanism

**Key config parameters (Anthropic's API shape):**

| Parameter | Default | Purpose |
|-----------|---------|---------|
| `trigger` | 100K input tokens | When clearing activates |
| `keep` | 3 tool uses | Recent results kept verbatim |
| `clear_at_least` | — | Minimum tokens freed per pass |
| `exclude_tools` | — | Tools whose results never clear (memory, todo) |
| `clear_tool_inputs` | false | Also strip what was sent to the tool |

**Implementation in our harness:** New little-coder pi extension `tool-result-clearer`:
- Hook: post-compaction-trigger / pre-send
- Keep last 4 tool uses verbatim (tunable)
- Stub format includes: tool name, target, token count freed, and a one-line "disposition" (e.g. "file subsequently edited", "tests passed", "output consumed in turn 12")
- Exclusions: plan file reads, todo state, VITRIOL status checks (cheap anyway)
- Syncs with the 80% compaction watchdog (clearing runs FIRST — it's cheaper than LLM summarization)

**Why it's the top remaining priority:** It attacks waste that nothing else in the stack addresses. Repo-map prevents blind reads; read-guard prevents oversized entries; but nothing evicts *consumed* results. This is where Claude Code got its 84% number.

**Interaction with VITRIOL:** Clearing happens in the scaffold before the request hits llama-server, so the cleared KV pages simply stop being sent — no LULL eviction needed for them. Cleaner than engine-side eviction because the stub text preserves the causal record.

---

## R2.2 ReWOO: Reasoning Without Observation — HIGH

**Source:** arXiv:2305.18323 (Google, 2023); production pattern in agent-patterns catalog; Nhahan/mcp-agent implements it for MCP servers.

**The problem:** ReAct loops (reason → act → observe → reason) re-inject every observation into the prompt for the next reasoning turn. Token cost grows **quadratically** with step count: an 8-step task re-reads its own trace 7 times. On a frontier API model this is a cost problem; on an 11 t/s local model it is also a **latency catastrophe** — every redundant re-read is prefill compute at your slowest speed.

**Measured numbers:** Up to **5x token efficiency** vs ReAct on multi-tool benchmarks; **4% accuracy improvement** on HotpotQA (less context noise = better reasoning). Robust under tool-failure (modular trace lets solver compensate).

**Mechanism — Plan-Work-Solve:**

```
Planner (one LLM call):
  Emits complete DAG with placeholder variables:
    #E1 = read_file(src/config.rs)
    #E2 = grep("timeout", #E1)
    #E3 = run_test(--filter auth)
  Plan fully inspectable BEFORE any tool fires.

Worker (zero LLM calls):
  Executes the DAG topologically. Parallel where the DAG allows.
  Substitutes real outputs into placeholders.
  Tool outputs NEVER re-enter any prompt.

Solver (one LLM call):
  Reads query + plan + resolved trace ONCE.
  Produces the final answer / patch.
```

**When it fits:**
- Step structure is determined by the task, not by what previous steps returned
- Tools have stable signatures (file ops, test runners, builds)
- Chains are predictable: read → grep → edit → test → build

**When it does NOT fit:**
- Exploratory work where observations genuinely redirect planning (debugging mystery failures)
- Small local models produce lower-quality upfront plans — the plan must be right, or the savings are paid back with interest in rework
- Tool outputs are large AND the solver needs to reason per-step anyway

**Implementation in our harness:** Extend little-coder's plan mode:
- Plan mode already separates research from implementation — add a plan *format*: steps with placeholder variables referencing prior step outputs
- A `rewoo-dispatch` extension validates the DAG, checks every referenced tool exists (plan validation, as mcp-agent does), then executes without LLM round-trips
- Fallback rule: if a step's output contradicts the plan's assumption (file missing, test fails unexpectedly), ABORT to interactive mode — never let the solver hallucinate around a failed plan
- Composition note from the pattern catalog: ReWOO's win is token cost, LLMCompiler's win is latency via parallel independent steps — same DAG shape serves both. Execute independent branches concurrently (bounded by `max_concurrent_subcoders: 3`)

**Small-model mitigation:** Restrict ReWOO dispatch to a whitelist of chain shapes (edit+test, refactor+build, doc-update). Everything else stays interactive. The planner prompt for a whitelisted chain is small and templated — exactly where a 27B model is reliable.

---

## R2.3 LLMLingua-2: Small-Model Prompt Compression — MEDIUM

**Source:** Microsoft Research — LLMLingua (EMNLP'23, arXiv:2310.05736), LongLLMLingua (arXiv:2310.06839), LLMLingua-2 (ACL'24, arXiv:2403.12968). Apache-2.0 on GitHub (microsoft/LLMLingua). Production case study: $42K → $2.1K monthly on a RAG pipeline.

**The numbers:**

| Variant | Max compression | Speed | Best target | Accuracy impact |
|---------|-----------------|-------|-------------|-----------------|
| LLMLingua (v1) | 20x | baseline | General prompts | -1.5 pts |
| **LLMLingua-2** | 10-15x | **3-6x faster, CPU-viable** | Production volume | -1 to -2 pts |
| LongLLMLingua | ~4x | slower | RAG / long context | ~0, sometimes +21.4% |

Typical production compression is 4-10x, not the peak 20x. LLMLingua-2 is a BERT-level token-classification encoder distilled from GPT-4 labels — small enough to run on CPU alongside everything else (~100ms overhead per request).

**Mechanism:** A small encoder scores each token's importance to the task; low-scoring tokens are dropped. Compression is **lossy by design** — the bet is that 50-95% of prompt tokens are boilerplate/filler. LLMs can reconstruct compressed text surprisingly well; compressed prompts sometimes *improve* accuracy by removing distraction (context rot).

**Where it fits in OUR harness:**

| Target | Before | After (est.) | Notes |
|--------|--------|--------------|-------|
| Hermes memory retrieval injection | ~500 tok | ~150 tok | Hermetis/FTS5 results are prose-heavy — ideal |
| Old tool results pre-stub | — | optional 2-4x | Can compress THEN stub, preserving more signal in the stub |
| Sub-coder reports | ~500 tok | ~200 tok | Reports are summary-of-summary — safe |
| Long documentation reads | page-sized | 1/4 | Compress before the read-guard has to truncate |

**Where it does NOT fit (hard rules):**

| Target | Verdict | Reason |
|--------|---------|--------|
| System prompt | NEVER | Small local models need their scaffolding; compression noise breaks fragile instruction-following |
| Code being edited | NEVER | Extraction-critical; one dropped token = corrupted edit |
| Tool schemas | NEVER | Structural; dropping tokens breaks call syntax |
| Plan files | NEVER | The model must follow them exactly |

**Also do not generalize the Anthropic "remove 80% of the system prompt" finding (Claude 5, Aug 2026) to this harness.** That is a frontier-model result. Qwen 3.8 27B needs MORE scaffolding, not less.

**Implementation:** Hermes-side Python module using `llmlingua` pip package with LLMLingua-2's `xlm-roberta-large` (or `mbert` multilingual variant) encoder on CPU:
- Wraps the memory-injection path in the hermes-bridge extension
- Wraps sub-coder report intake
- Budget-aware: compression ratio chosen per-request to hit a token target
- Kill-switch config flag per target (start with memory only, expand if evals hold)

**Latency note:** ~100ms CPU per request is negligible against 11 t/s generation (a 500-token response takes ~45s). Prefill savings outweigh compression overhead on every long-context request.

---

## R2.4 External Task State (Claude Code TodoWrite pattern) — HIGH

**Source:** Claude Code reverse-engineering (yulonghe97/claude-code-explain, Yuyz0112/claude-code-reverse). TodoWrite writes `~/.claude/todos/<session>.json`; a system reminder re-injects the current todo list every turn.

**The problem it solves:** Mid-run compaction eats conversation history. After compaction, the model has a summary but may drift from the original task decomposition — it no longer knows precisely which sub-steps remain. Claude Code's answer: the task list is not conversation, it's **external state**, persisted to a file and re-injected fresh every turn.

**Mechanism:**
1. Task list lives in a JSON/markdown file, updated by the model as it works (tool call, not conversation)
2. Every turn, the current list is re-injected as a compact system-reminder tail message (~100-200 tokens)
3. Compaction can eat anything EXCEPT the todo file — it's re-read from disk each turn
4. On completion, checking off items gives the model a persistent sense of progress

**Fit:** little-coder's plan mode writes `approved-plan.md` ONCE at implementation start. After mid-run compaction the model can drift from it. Extending to a live todo file closes this gap:

**Implementation:** little-coder extension `task-state`:
- Tool: `update_tasks([{id, description, status}])` — writes `.pi/tasks/<session>.json`
- Each turn, inject current list as tail message in KV-cache-preserving position (last message, like skill cards)
- Cap at ~15 items / 200 tokens; hard-truncate with "[+N more]" beyond that
- Format: `[ ] 3. Fix timeout handling in config.rs` / `[x] 2. Add retry wrapper`
- On compaction: task state survives automatically (it's re-injected from disk, not from history)
- Hermes gets the same file path → cross-session visibility ("what was I doing on Tuesday?")

**Cost/benefit:** ~150 tokens/turn buys orientation that prevents the most expensive failure mode: post-compaction thrash, where the model re-does finished work or wanders off-task. On an 11 t/s model, one thrash episode costs more tokens than a month of todo injection.

---

## R2.5 Filesystem-First Memory (Letta benchmark finding) — VALIDATES EXISTING DESIGN

**Source:** Letta (formerly MemGPT), "Benchmarking AI Agent Memory" (Dec 2025), LoCoMo benchmark.

**The finding:** Plain filesystem + grep scores **74.0%** on LoCoMo memory tasks — beating specialized vector-store memory libraries. Letta Filesystem simply stores conversation histories as files and greps them. December 2025 survey (arXiv:2512.13564) confirms the field's fragmentation and the complexity-vs-practicality tension.

**Implications for this harness:**

| Decision | Consequence |
|----------|-------------|
| Hermes FTS5 + markdown skills = sufficient | CONFIRMED. Do not add vector DB layers |
| No Mem0 / Zep / Graphiti / GraphRAG | Skip. Complexity for complexity's sake at single-user scale |
| Agent-managed memory (MemGPT-style tools) | Partial adopt: give the agent a small `MEMORY.md` it edits itself (append/replace), Claude-Code-memory-tool style. First 200 lines inject into system prompt |
| Skill learning transfer | Letta finding: memory/skills generated by a large model transfer to Qwen2.5-class models with significant boost — relevant if you ever distill skills from a cloud model |
| Memory security | New attack surface noted: prompt injection via stored memories, context poisoning. Hermes memory writes should be tool-gated, not free-text |

**MemGPT's core insight, already absorbed:** treat context as RAM and disk as external storage, page between them. Hermes FTS5 (recall) + VITRIOL SSD archival (page-out) + slot checkpoints (swap) already implement the valuable 80% of MemGPT. The remaining 20% — the agent managing its own paging via tool calls — is the risky part on a small model and is explicitly NOT adopted.

---

## R2.6 Explicitly Rejected Prior Art

| Technique | Source | Verdict | Reason |
|-----------|--------|---------|--------|
| Full MemGPT OS-paging (agent manages own memory via tools) | arXiv:2310.08560 | **Skip** | Hermes FTS5 + VITRIOL archival cover the value; agent-managed paging = more failure modes than savings on a 27B model |
| Gist tokens (compress instructions into learned tokens, 26x) | awesome-llm-token-reduction list | **Skip** | Research-stage; requires training; fragile with local models |
| 80% system-prompt reduction | Anthropic, Claude 5 blog (Aug 2026) | **Skip** | Frontier-model finding (Claude 5 follows lean prompts well); Qwen 3.8 needs the scaffolding |
| Vector DB memory (Mem0, Zep/Graphiti, HippoRAG, GraphRAG) | Various | **Skip** | Letta filesystem benchmark: plain files beat them at 74%; we have FTS5 |
| Speculative decoding / MTP | VITRIOL VERDICTS.md | **Skip** | Already tried on your GTX 1070 Ti: zero measurable benefit, tombstoned |
| Chain-of-Agents / multi-agent debate | Papers | **Skip** | Multiplies token consumption; sub-coder isolation already gives the useful slice |
| Aider architect mode (two-model split) | Aider | **Deferred** | Requires a second capable model; revisit if you add a cloud fallback |
| SWE-agent ACI polish | arXiv:2405.15793 | **Covered** | little-coder's write-guard + edit-only invariant + read-guard already implement the core ACI principles |

---

## R2.7 Round 2 Priority Summary

| # | Technique | Source | Token cost | Savings | Priority | Implement as |
|---|-----------|--------|------------|---------|----------|--------------|
| 1 | Tool-result clearing | Claude Code | ~0 (stubs are tiny) | 30-50% of loop waste; Anthropic measured 84% w/ memory | **Critical** | little-coder ext `tool-result-clearer` |
| 2 | External task state | Claude Code | ~150 tok/turn | Prevents post-compaction drift/thrash | **High** | little-coder ext `task-state` |
| 3 | ReWOO plan-then-execute | arXiv:2305.18323 | One planner call | Up to 5x on multi-tool chains; latency win on 11 t/s | **High** | little-coder ext `rewoo-dispatch` (whitelisted chains) |
| 4 | LLMLingua-2 compression | Microsoft | ~100ms CPU/req | 2-4x on memory injection + sub-coder reports | **Medium** | Hermes module + hermes-bridge wrap |
| 5 | Agent-managed MEMORY.md | Claude Code memory tool + Letta | ~200 tok in system prompt | Cross-session continuity without vector DB | **Medium** | Hermes memory tool, gated writes |
| 6 | Filesystem-first validation | Letta | 0 | Validates design; blocks over-engineering | **Info** | No code — decision record |

---

## R2.8 Revised Token Budget (Round 1 + Round 2 combined, ~54K usable)

| Allocation | Round 1 | Round 2 | Managed by |
|------------|---------|---------|------------|
| System prompt | ~7,000 | ~7,200 (+MEMORY.md 200) | little-coder cold-start |
| Repo map | ~1,000 | ~1,000 | tree-sitter + PageRank (Aider) |
| Active conversation | ~18,000 | ~19,000 | async compaction (Aider) |
| Tool outputs | ~4,000 | ~2,500 (**-1,500: clearing**) | tool-result-clearer (Claude Code) |
| Task state (todo) | 0 | ~200/turn | task-state ext (Claude Code) |
| Skill cards | ~300/turn | ~300/turn | little-coder skill-inject |
| Knowledge blocks | ~200/turn | ~200/turn | little-coder knowledge-inject |
| Thinking budget | ~4,096 | ~4,096 | thinking-budget ext |
| Memory retrieval | ~500 | ~150 (**LLMLingua-2**) | Hermes FTS5 → compress → inject |
| Sub-coder reports | ~500 each | ~200 each (**LLMLingua-2**) | sub-agent isolation + compression |
| **Headroom** | ~19,400 | **~21,100** | Buffer for growth |

Effective usable work context improves another ~1.7K tokens, and the *quality* of every remaining token rises (less stale payload, causal stubs, task orientation).

---

## R2.9 Updated Execution Order (complete, 18 steps)

| Step | Task | Dependencies | Unlocks |
|------|------|--------------|---------|
| 1 | License change (VITRIOL + llama.cpp) | None | Legal compatibility |
| 2 | Clone + test little-coder against VITRIOL | 1 | Coding agent ready |
| 3 | Deploy repo-map MCP server | 1 | Structural awareness (~1K tok) |
| 4 | Run VITRIOL WITHOUT shim | 2 | Scaffold owns context |
| 5 | little-coder ext: tool-result-clearer | 2 | Biggest loop-waste cut (R2.1) |
| 6 | little-coder ext: task-state | 2 | Post-compaction orientation (R2.4) |
| 7 | little-coder ext: async compaction | 2 | Non-blocking compaction (Aider) |
| 8 | little-coder ext: batch-aware condensation | 2 | Pair-preserving compression (OpenHands) |
| 9 | Hermes plugin: VITRIOL model provider | 4 | Hermes uses VITRIOL |
| 10 | Hermes plugin: little-coder dispatch | 9 | `/lc` + auto-dispatch |
| 11 | little-coder ext: hermes-bridge | 10 | Sub-coders query Hermes memory |
| 12 | Hermes: agent-managed MEMORY.md tool | 10 | Cross-session continuity (R2.5) |
| 13 | Hermes: LLMLingua-2 compression module | 11 | 2-4x on memory injection (R2.3) |
| 14 | little-coder ext: rewoo-dispatch (whitelist) | 5, 6 | 5x on predictable chains (R2.2) |
| 15 | Unified config | 9-14 | Single config for all systems |
| 16 | Context monitor sidecar | 15 | Cross-system overflow prevention |
| 17 | Skill cross-pollination sync | 15 | Skills available everywhere |
| 18 | Observability dashboard | 15 | Unified metrics view |

Sequencing logic: all scaffold-side context extensions (5-8) come before Hermes integration (9-14) so the coding loop is efficient standalone from day one. ReWOO (14) comes last among extensions because it depends on clearing + task-state for a clean substrate and carries the highest plan-quality risk — measure steps 5-8 gains first.

---

## R2.10 Additional Risks (Round 2)

| Risk | Impact | Mitigation |
|------|--------|------------|
| Tool clearing removes a result still needed | Model hallucinates cleared content | Keep last 4 verbatim; stubs carry disposition line; exclusions for plan/memory tools |
| ReWOO plan wrong → wasted execution | Tokens burned on bad DAG, plus rework | Whitelist only predictable chain shapes; abort-to-interactive on first contradiction; never solver-guess through failures |
| ReWOO planner quality too low on Qwen 3.8 | Feature unusable | Measure plan validity rate; threshold: if <70% plans execute clean, drop feature (shadow-benchmark the decision) |
| LLMLingua drops task-critical token in memory result | Wrong fact recalled | Never compress below 2x; keep kill-switch per target; validate on real Hermetis queries before enabling |
| Todo file grows unbounded | Eats the savings it provides | 15-item / 200-token cap; hard truncation; curator merges completed items into session summary |
| MEMORY.md prompt injection via stored memories | Context poisoning, security | Writes tool-gated; content reviewed by curator; no free-text writes |
| Compression + clearing + compaction interact badly | Over-compressed context, quality collapse | Order: clear → compact → compress; each has independent kill-switch; budget monitor watches total reduction rate |
