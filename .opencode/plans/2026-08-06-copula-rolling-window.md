# Copula Rolling Window over a Database

Date: 2026-08-06.

## 1. Goal

Turn the OpenCode context window into a **rolling window over Hermetis**: everything
streams into memory, compaction is lossless, and each turn the window is reassembled
from what matters. The user should rarely *feel* compaction and nothing is ever lost.

## 2. Decisions (2026-08-06, approved)

- **Auto-injection**: ON by default, behind a `COPULA_AUTO_CONTEXT` toggle.
- **Injection budget**: ~3000 tokens per turn (rich, inside the measured ~32K fast zone).
- **Labeling**: injected memory prefixed `[Hermetis context]` so the model distinguishes
  recalled memory from live conversation.
- **Verify installed opencode API first** — DONE (opencode 1.15.13): `session.compacted`,
  `session.prompt({noReply, parts})`, `session.messages`, `event`, `tool.execute.after`,
  `experimental.session.compacting`, and the bonus **`chat.message`** hook all present.

## 3. Components

### A. Lossless compaction sweep (plugin)
- Primary capture: existing per-message ingest + **`chat.message`** hook (full
  UserMessage + parts — closes the streaming gap).
- Lossless guarantee: **`experimental.session.compacting`** hook (fires *before* the
  context is replaced) -> `session.messages()` -> store any un-stored part (dedupe).
- Result: compaction can never lose anything the model saw.

### B. Per-turn auto-injection (plugin)
- Trigger: **`chat.message`** (new user message) -> Hermetis `/hermetis/context` ->
  inject via `session.prompt({ noReply: true, parts: [text] })`, labeled
  `[Hermetis context]`.
- Toggles: `COPULA_AUTO_CONTEXT` (default on), `COPULA_CONTEXT_BUDGET` (3000),
  `COPULA_CONTEXT_TOP_K` (5).
- Guards: dedupe injected content hashes; skip re-ingesting synthetic injected parts
  (`TextPart.synthetic`); budget cap.

### C. Hermetis `/hermetis/context` (Hermetis server + retrieval)
- `POST /hermetis/context {project, recent_text, budget_tokens}` -> budget-capped,
  recency+relevance-ranked context block (episodes + current nodes) ready to inject.
- Reuses the existing retrieval + scorer; `superseded=0` current nodes.

## 4. Files

- `plugins/copula.ts` (both copies: `VITRIOL/plugins/` + `~/.config/opencode/plugins/`): A + B.
- `libvitriol/hermetis_server.py` + `libvitriol/hermetis/retrieval.py`: `/hermetis/context`.
- `docs/copula.md`, this plan.

## 5. Risks

- Injection pollution -> dedupe + budget + label.
- Feedback loops -> recency weighting dampens.
- noReply injection timing vs the model call -> verify in validation.

## 6. Validation

Multi-turn session: everything retrievable after compaction; labeled per-turn injection
present; window stays in the fast zone; compaction transparent.

## 7. Cross-repo

Plan + code mirrored in bitshaper-ai (canonical) and VITRIOL; commits per logical step.
