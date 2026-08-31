# Report 02: Harness Expansion — Trismegistus

**Date:** 2026-08-28
**Status:** Approved (decision (a): steal patterns only, no OmniRoute deployment)
**Predecessor:** PLAN.md (Round 1 + Round 2)
**Companion:** This report covers everything decided after Round 2: the harness name, VITRIOL-side engine changes, OpenCode steals, OmniRoute steals, and the R3 "Beyond the Scaffold" program.

---

## 1. The Name: Trismegistus

The harness is named **Trismegistus** — Hermes Trismegistus, mythical father of alchemy and hermeticism. The name is thematically load-bearing, not decorative:

| Component | Alchemical resonance |
|-----------|---------------------|
| VITRIOL | Vitriol — one of the three primes of alchemy (vitriol, mercury, salt); V.I.T.R.I.O.L. is the classic alchemical acronym: *"Visita Interiora Terrae Rectificando Invenies Occultum Lapidem"* |
| alka-executor / alka-handoff | Alkahest — the universal solvent, the alchemist's dream reagent |
| Hermes-Agent | Hermes — messenger god and eponym of hermeticism |
| **Trismegistus** | "Thrice-great" Hermes — binder of the three roles: messenger (gateway), alchemist (engine), scribe (memory/skills) |

The name binds the stack's existing vocabulary into one identity. Project directory: `~/Projects/trismegistus/`. Config path: `~/.config/trismegistus/`.

**Collision note:** a small pentest/CTF toolkit shares the name on GitHub. Irrelevant for a personal project; revisit only if publishing.

---

## 2. VITRIOL-Side Engine Changes

**Principle:** VITRIOL is ours. Where the original plan adapted the scaffold around engine limitations, we now modify the engine to serve the harness. Loose coupling (OpenAI-compatible HTTP) remains the default interface; the following changes add narrow, deliberate coupling points.

### 2.1 Shim → config flag, default OFF for Trismegistus

**Was planned as:** run VITRIOL without the shim.
**Becomes:** the shim moves behind a config flag. Default OFF for Trismegistus sessions; retained for other profiles and legacy use, because disabling it entirely would orphan the Hermetis memory features those profiles rely on.

- Flag: `shim.enabled: false` in the engine profile used by Trismegistus
- Scaffold (little-coder) owns context management; engine owns KV efficiency
- Optional future work: scaffold-aware pass-through mode (header-gated `X-Context-Managed: true`) so shim memory features could coexist with scaffold-managed context. Not scheduled; recorded as a possibility since the shim code stays in-tree.

### 2.2 Checkpoint/restore HTTP endpoint

**Was planned as:** checkpoint API "hoped for".
**Becomes:** first-class engine feature. VITRIOL gains two HTTP endpoints:

```
POST /slots/{id}/checkpoint   → snapshot slot KV + decode state to disk (async, non-blocking decode)
POST /slots/{id}/restore      → restore snapshot (used after crash/OOM bounce)
```

Scaffold usage:
- little-coder extension `vitriol-checkpoint` triggers a snapshot before risky operations (bulk refactors, long autonomous runs)
- On crash detection (slot gone / server restarted), restore is attempted before session restart
- Pairs with per-turn git snapshots (see §3.2): "rewind to turn N" = git checkout + slot restore as one operation

### 2.3 Engine as scheduling authority

**Was planned as:** context monitor polls and guesses.
**Becomes:** VITRIOL's `/slots` + KV fill state is the *authority*. Sub-coder dispatch (little-coder `rewoo-dispatch`, `subcoder_concurrency`) queries the engine before spawning; dispatch blocks when KV fill > threshold. The context monitor sidecar becomes a *verifier*, not a guesser.

### 2.4 Cert-workflow wired into harness

**Becomes:** the VITRIOL certification suite (context-fill certification, VERDICTS-style benchmarks) is invoked by the harness config layer. Any model or engine config change (e.g. R3.2 model swap) must pass cert before the config is accepted:

```
trismegistus config set model Qwen3-30B-A3B-Q4
  → runs VITRIOL cert suite
  → records VERDICT entry
  → refuses activation if cert fails
```

No uncertified model/engine combinations can enter `~/.config/trismegistus/`.

---

## 3. Steals from OpenCode (first-hand source)

OpenCode's architecture validates several Trismegistus decisions and contributes two implementations.

### 3.1 Validated (no work)

| OpenCode pattern | Trismegistus equivalent | Status |
|------------------|------------------------|--------|
| Client/server split (headless server, swappable TUI/web/CLI frontends) | Hermes-as-gateway design | Already implicit — validated |
| Plan mode ↔ build mode as first-class states | little-coder plan mode | Present; formalization below |
| Permissions DSL (allow/ask/deny per tool + path) | `safety.approval_required` | Refined below |

### 3.2 Per-turn workspace snapshot + rewind-to-turn-N (ADOPT)

OpenCode snapshots the workspace per message and can rewind to any point.

**Implementation:** little-coder extension `snapshot`:
- Git commit per turn (Aider-style), tagged with turn number
- Combined with VITRIOL checkpoint/restore (§2.2): "rewind to turn N" = `git checkout <turn-tag>` + `POST /slots/{id}/restore` — one operation, reverses both code and conversation state

### 3.3 LSP diagnostics auto-injection (ADOPT)

OpenCode feeds LSP/compile diagnostics back into the agent loop automatically.

**Implementation:** little-coder extension `diagnostics-loop`:
- Post-edit hook runs the relevant linter/compiler/Praetor check on the edited file
- Errors/warnings are injected as a compact tail message (`[diagnostics: 3 errors, 1 warning — file.rs:42 …]`)
- Model auto-repairs without a human round-trip
- Sharpens the planned quality gate from "check" to "check → auto-repair → re-check" loop
- Budget: diagnostics capped at ~300 tokens per injection

### 3.4 Formal plan↔build state machine (ADOPT, small)

- Plan mode cannot write files; build mode cannot re-research (transitions logged)
- Mode transitions become explicit contract, preventing the "plan mode quietly starts editing" failure

### 3.5 Permissions DSL refinement (ADOPT, small)

- `safety.approval_required` becomes path-aware: `ask` on `~/.ssh/**`, `deny` on `.env` writes, `allow` on `src/**` for edit tool, etc.
- Config shape borrowed from OpenCode's permission rules

---

## 4. Steals from OmniRoute (pattern-only, decision (a))

OmniRoute (MIT, github.com/diegosouzapw/OmniRoute) is a local AI gateway: 350+ providers, 4-tier fallback, RTK + Caveman compression, MCP server, memory system, guardrails. Built for multi-provider cloud setups; Trismegistus is local-first with one engine path. **Decision: steal patterns, do not deploy OmniRoute.** No proxy hop, no extra daemon.

### 4.1 RTK-style command-output filtering (ADOPT — High)

OmniRoute's RTK (Reduce To Knowledge) transforms tool/command output at *ingestion*: bash/test/build output → exit status + errors + structured summary, before it enters context.

**Complements R2.1 tool-result clearing — different layers:**

| Technique | Layer | Effect |
|-----------|-------|--------|
| RTK filtering | Entry — output transformed as it arrives | Prevents bloat from ever entering |
| Tool-result clearing (R2.1) | Eviction — consumed results stubbed later | Removes payload after consumption |
| Read guard (little-coder) | Entry — file reads truncated | Covers reads, not command output |

**Implementation:** little-coder extension `rtk-output`:
- Wraps bash/test/build tool results: capture exit code, extract error lines, tail of output, structured summary
- Full raw output written to disk (`.pi/rtk/<turn>.log`) and referenced by path — recoverable if the model needs more
- Target: 60-90% reduction on typical test/build output

### 4.2 Caveman rule-based prose compression (ADOPT — Medium)

OmniRoute's Caveman compression: language-pack rules compress prose with zero latency and no model.

**Implementation:** Hermes module `caveman-rules`:
- Rule-based compression for sub-coder reports and non-critical prose
- Deterministic, zero-overhead alternative to LLMLingua-2 on paths where an encoder is overkill
- LLMLingua-2 (R2.3) stays for memory retrieval (semantic compression); Caveman rules cover report/prose paths

### 4.3 Context Relay on model switch (ADOPT — Medium)

OmniRoute generates a handoff summary when switching models mid-session (`handoffThreshold`, `handoffModel`).

**Implementation:** Hermes-side `context-relay`:
- On adaptive-routing model switch mid-task: generate compact handoff (task state + key decisions + open items) → inject into new model's fresh context
- Direct support for R3.2 model swaps and adaptive routing
- Handoff budget: ~500 tokens

### 4.4 Gateway-layer memory extraction (ADOPT — High)

OmniRoute extracts facts from conversations at the proxy layer (Mem0-shaped), injecting relevant memories into future requests.

**Why it matters here:** Hermes IS the gateway. Memory extraction at gateway level works across ALL clients — little-coder sessions, Telegram chats, CLI — with zero per-client code.

**Implementation:** Hermes module `memory-extractor`:
- After each session: extract durable facts/decisions → append to Hermetis DB + agent MEMORY.md
- Hybrid with R2.5 MEMORY.md: auto-extraction (gateway) + model-managed edits (memory tool) + curator review (existing)
- Injection: retrieval-then-compress (LLMLingua-2 path from R2.3)

### 4.5 Prompt-injection guard + secret masking (ADOPT — High, small)

OmniRoute guards every route against prompt injection; masks secrets in logs.

**Implementation:**
- `injection-guard` at Vitriol-ingestion boundary (PDFs, web content, VITRIOL conversion outputs) — patterns + heuristics before content enters context
- Secret masking in observability pipeline: never log API keys, tokens, `.env` contents; applies to context monitor + dashboard + session logs

### 4.6 Rejected from OmniRoute (record)

| OmniRoute feature | Verdict | Reason |
|-------------------|---------|--------|
| Deploy OmniRoute itself as router | **Rejected** (decision (a)) | Local-first, one engine; proxy hop + second Node daemon not justified. Revisit only if cloud fallback becomes a requirement |
| 4-tier fallback routing | **Deferred** | Local equivalent = VITRIOL Tier 1 + cloud Tier 2. Requires cloud provider keys; revisit with architect-mode (R2.6 deferred item) |
| MITM proxy / TLS stealth | **Rejected** | Not applicable to local-only harness |
| Cloud Agents (codex-cloud, devin, jules) | **Rejected** | Out of scope for local-first |
| A2A protocol | **Rejected** | Hermes + little-coder cover agent-to-agent needs |
| Gamification | **Rejected** | No |

---

## 5. R3: Beyond the Scaffold

**Gate:** R3 work starts ONLY after the main harness (execution steps 1-16) is proven functional AND baselines are measured (t/s, tokens/task, task success rate). Without baselines, R3 gains are unprovable.

**Context:** Round 1 + Round 2 exhaust scaffold-level context engineering. The remaining levers are below the scaffold: engine, model choice, hardware. Each is bigger than any remaining software trick.

### 5.1 R3.1 Speculative decoding re-test (engine)

VERDICTS.md tombstoned speculative decoding — but *which config was tested?* A 3B Q8 draft resident on the 1070 Ti + 27B target on the 3060 is a different architecture from a same-GPU or CPU-draft setup.

**Protocol:**
1. Draft model: 3B-class, Q8, resident on GTX 1070 Ti (8GB)
2. Target: 27B Q3_K_M on RTX 3060
3. Measure: acceptance rate, effective t/s across 3 task types (edit, test-run, doc)
4. Shadow-benchmark vs current single-stream
5. Machine decides: <1.2x gain → tombstone stands, record verdict

### 5.2 R3.2 Model swap study (biggest lever)

Q3_K_M on 27B does real damage — Q3 loses disproportionately more reasoning and instruction-following than file size suggests. Three candidates, all within 20GB VRAM:

| Option | Quality | Speed | Notes |
|--------|---------|-------|-------|
| 27B Q3_K_M (current) | degraded | ~11.3 t/s | baseline |
| 27B Q4_K_M, context capped ~54K | better | ~10 t/s | ~16.4GB weights + ~2.3GB KV at practical window ≈ 19GB, tight fit. Q3 quality rot begins at the same 54K anyway — nothing used is lost |
| Qwen3-30B-A3B MoE Q4 | comparable-or-better | ~30-40 t/s (3B active) | ~18GB; 3x throughput; MoE quants degrade differently — needs cert |

**Protocol:** each candidate runs the VITRIOL cert suite (§2.4) → VERDICT entry → decision recorded. Cert-first is mandatory.

### 5.3 R3.3 1070 Ti role reassignment

Stop pipeline-parallel deadweight (Pascal: no FA2, 256 GB/s bandwidth, slows every forward pass). The 8GB card becomes, sequentially:
1. Speculative-decode draft model (R3.1, if verdict positive)
2. Dedicated LLMLingua-2 encoder host (R2.3 offload — zero contention with main model)
3. Embeddings model for the offline RAG indexer

### 5.4 R3.4 Hardware path (recorded, not scheduled)

Used RTX 3090 24GB (~$500-700): 27B at Q5/Q6, KV 8bpw, single-GPU (no pipeline stall), 2-3x t/s, 1070 Ti freed for R3.3 roles. The only lever that raises the ceiling itself rather than reducing waste beneath it. No purchase decision in this plan.

### 5.5 R3.5 Parallel slots for sub-coders

`subcoder_concurrency: 2` + llama-server parallel slots: aggregate throughput gain when sub-coders interleave LLM calls with I/O. Free; test with R3 baselines.

### 5.6 What nothing can fix (recorded)

Context rot beyond ~54K on Q3-quantized 27B is model-internal — no scaffold technique touches it. 11 t/s decode is physics at this VRAM; scaffold tricks cut round trips, not decode speed.

---

## 6. Ceiling Analysis (final)

| Layer | Status | Remaining gain |
|-------|--------|---------------|
| Scaffold (R1 + R2) | **Saturated** | ~1K tokens, diminishing returns |
| Engine (R3.1 spec-decode, R3.5 slots) | Untested | 1.5-2x t/s possible |
| Model (R3.2 swap study) | Unexamined | 3x t/s and/or quality recovery |
| Hardware (R3.4) | Bound | 2-3x + real quality headroom |

Trismegistus' scaffold work optimizes waste out of a constrained engine — the right first move. R3 is the honest ceiling-breaker, gated behind a proven harness and measured baselines.

---

## 7. Consolidated Execution Order (final, 25 steps)

| Step | Task | Dependencies | Source |
|------|------|--------------|--------|
| 1 | License change (VITRIOL + llama.cpp → Apache-2.0) | None | PLAN Phase 1 |
| 2 | Clone + test little-coder against VITRIOL | 1 | PLAN Phase 2 |
| 3 | VITRIOL: shim config flag (default off for Trismegistus) | 1 | Report §2.1 |
| 4 | VITRIOL: checkpoint/restore endpoints | 3 | Report §2.2 |
| 5 | Deploy repo-map MCP server | 1 | Round 1 |
| 6 | Run VITRIOL WITHOUT shim (flag off) | 3 | PLAN Phase 3 |
| 7 | little-coder ext: tool-result-clearer | 2 | R2.1 |
| 8 | little-coder ext: rtk-output | 2 | OmniRoute §4.1 |
| 9 | little-coder ext: task-state | 2 | R2.4 |
| 10 | little-coder ext: snapshot (git per turn) | 4, 7 | OpenCode §3.2 |
| 11 | little-coder ext: diagnostics-loop | 7 | OpenCode §3.3 |
| 12 | little-coder ext: async compaction | 2 | Aider |
| 13 | little-coder ext: batch-aware condensation | 2 | OpenHands |
| 14 | Hermes plugin: VITRIOL model provider | 6 | PLAN Phase 3a |
| 15 | Hermes plugin: little-coder dispatch (`/lc`) | 14 | PLAN Phase 3b |
| 16 | little-coder ext: hermes-bridge | 15 | PLAN Phase 3c |
| 17 | Hermes: agent-managed MEMORY.md tool | 15 | R2.5 |
| 18 | Hermes: memory-extractor (gateway-layer) | 16, 17 | OmniRoute §4.4 |
| 19 | Hermes: LLMLingua-2 + caveman-rules compression | 16 | R2.3 + §4.2 |
| 20 | Hermes: context-relay | 19 | OmniRoute §4.3 |
| 21 | Hermes: injection-guard + secret masking | 18 | OmniRoute §4.5 |
| 22 | little-coder ext: rewoo-dispatch (whitelisted chains) | 7, 9 | R2.2 |
| 23 | Unified config + permissions DSL + cert wiring | 14-22 | PLAN 3d + §2.4 + §3.5 |
| 24 | Context monitor sidecar (verifier role) | 23 | Enhancement 1 |
| 25 | Skill sync + observability dashboard | 23 | Enhancements 3, 10 |

**Sequencing logic:** license first (legal), engine work next (flag + endpoints unlock scaffold extensions), scaffold context extensions (7-13) before Hermes integration (14-21) so the coding loop is efficient standalone from day one, ReWOO (22) late (highest plan-quality risk on 27B), R3 (§5) after everything with baselines measured.

---

## 8. New Risks (Report 02 additions)

| Risk | Impact | Mitigation |
|------|--------|------------|
| Checkpoint/restore endpoint introduces engine instability | Decode corruption on restore | Endpoint behind feature flag; restore validated against cert suite; scaffold falls back to session restart on restore failure |
| RTK filter drops a signal the model needed | Model works from incomplete output | Raw output always written to disk, referenced by path; model can request full log; filter rules versioned and reviewed |
| Memory-extractor stores wrong facts | Context poisoning across all clients | Curator review queue; extraction confidence threshold; MEMORY.md remains tool-gated |
| Injection-guard false positives block legitimate content | PDF/web ingestion degraded | Guard logs-but-allows by default; block mode opt-in; tune on real corpus |
| Cert-workflow too strict → blocks experiments | R3.2 study slowed | Cert runs in "advisory" mode during study; enforcement mode only for production config |
| R3 work started without baselines | Gains unprovable, effort wasted | Hard gate: baselines script must exist and have run; tracked in step 25 → R3 handoff |
