# Pymander + Ascensus — Reference Mind and Cloud Escalation

Date: 2026-08-06.

## 1. Goal

Extend the closed-loop local cognitive architecture (VITRIOL + Hermetis + sliding
window + reasoning model — see `2026-08-06-closed-loop-architecture.md`) with two
subsystems:

- **Pymander** — a static, curated, domain-specific reference mind: preloaded technical
  knowledge that biases the model toward better processes in a domain.
- **Ascensus** — the ability for the model to escalate genuinely-hard inquiries to a
  configured cloud model (Google Gemini) that "takes over the wheel."

Requirement: independence + reliability (not frontier-level). Local for the routine;
cloud only for the hard/novel; the system learns so escalation self-reduces.

## 2. Design decisions (all from discussion, 2026-08-06)

### Pymander — "a more static Hermetis, built with quality knowledge"

- **Nature**: episodic Hermetis remembers *what happened*; Pymander is the reference mind
  — *what we are / how we do a domain well*. Static, curated, authoritative.
- **Content**: hand-authored **atomic nodes** — little pieces of relevant knowledge tied
  directly to the thing being written (a pattern, a gotcha, a convention, a process).
  Small, focused, task-relevant.
- **Authoring**: **hand-seed + curated growth**. Hand-author the core doctrine (quality
  + license-clean — no sourcing minefield). A **promotion path**: good Ascensus answers
  and consolidated Hermetis nodes that prove reliable become *candidates* promoted into
  the Pymander corpus — the user curates what lands. Hand-built quality, machine-assisted
  scale, license-safe.
- **Store**: per-domain corpus `~/.vitriol/pymander/<domain>/` — reuse Hermetis node
  machinery (`db.store_node`: label, summary, git_rev, embeddings).
- **Selection**: per-project config picks the active Pymander + an opencode tool
  (`pymander`: list/switch domains, author, query).
- **Injection**: **the same way Hermetis knowledge is inserted** — the selective
  `/hermetis/context`-style retrieval path; doctrine at session start (bounded) +
  on-demand retrieval. Rides the Copula plugin's existing auto-inject machinery.
- **Bias mechanism**: context-based domain adaptation — the model applies the injected
  doctrine (structure, patterns, conventions) instead of generic behavior. No fine-tuning;
  the knowledge lives in the window + Hermetis.

### Ascensus — "the ability to call a configured cloud model to take over the wheel"

- **Trigger**: both — the reasoning model auto-calls an `ascensus` tool when it judges an
  inquiry beyond it / wants a second opinion; the user can force it (command/toggle).
- **Provider**: Google Gemini, configurable (model + key) later. `GEMINI_API_KEY` via env,
  never committed. Occasional use — cost is acceptable if it fires rarely.
- **Protocol — replace**: the tool returns the cloud answer; the local model emits it as
  its response (honest replace). Two-phase hardening: start with model-finalizes; if the
  model corrupts good answers, switch to direct-inject (the tool appends an already-final
  assistant part, no local re-generation).
- **Escalation payload**: the query + relevant Hermetis/Pymander context + the local
  model's reasoning attempt.
- **Learning loop**: good cloud answers are stored into Hermetis (episodic) and become
  candidates for Pymander promotion (curated) — escalation self-reduces over time.
- **Security**: flag when escalation fires (user visibility); never send secrets or
  credentialed code to the cloud.

## 3. Full subsystem map

| subsystem | role |
| --- | --- |
| VITRIOL | runtime + sliding-window owner |
| Hermetis | episodic memory (what happened) |
| **Pymander** | static curated domain knowledge (how to do a domain well) |
| **Ascensus** | cloud escalation for hard inquiries (reliability valve) |
| Spagyric | hardware autotuner |
| Copula | the plugin/bond wiring it all |

## 4. Phases

- **P0 — GATE: Hermetis verification** (GPU-blocked by the avatar capture). Retrieval +
  selective injection proven; window acceptance criteria (`/v1/models` n_ctx 32768, no
  compaction, injected context survives ctx-shift, opencode honors `limit.context`).
- **P1 — Pymander store + ingest**: per-domain node namespace (reuse Hermetis db) +
  an ingest/author tool (markdown -> atomic nodes, embedded, versioned) + per-project
  selection config.
- **P2 — Pymander injection + tool**: doctrine via the Hermetis context path + the
  `pymander` opencode tool (list/switch/author/query).
- **P3 — Ascensus**: `ascensus` tool + Gemini wiring + replace protocol + learning loop.
- **P4 — Sample corpus**: hand-author a systems-programming Pymander; prove the bias
  effect + a real escalation + the learning loop.

## 5. Licensing / security notes

- Hand-authored Pymander content is original (GPL-2.0-clean). No sourced material without
  permissive licensing — same care as the kimi situation.
- Ascensus calls a cloud service (API call, not code incorporation) — no GPL-2.0 issue.
- `GEMINI_API_KEY` only in env, never in the repo. Escalation flagged; no secrets/code
  with credentials leave the machine.

## 6. Cross-repo

Plans + docs in bitshaper-ai (canonical) + VITRIOL. Pymander/Ascensus code in VITRIOL
(plugin + Hermetis server). Both subsystems gated on P0.
