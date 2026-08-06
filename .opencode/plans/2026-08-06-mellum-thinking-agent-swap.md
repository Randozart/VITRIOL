# Mellum2-Claude Thinking — Agent Swap Experiment

Date: 2026-08-06.

## Model

`yuxinlu1/Mellum2-12B-A2.5B-Claude-4.6-4.8-Opus-Thinking-GGUF`
- Base: `JetBrains/Mellum2-12B-A2.5B-Thinking` (official). Distilled on Claude Opus
  4.6/4.7/4.8 reasoning traces. **Apache-2.0** (weights, not code — no GPL-2.0 conflict
  for running it with VITRIOL).
- Same `mellum2` arch → drop-in for the fork. 98K vocab (vs current 96K).
- Quants: Q2_K 5.0 GB, Q4_K_M 8.1 GB, Q6_K 10.9 GB, Q8_0 12.9 GB.

## Motivation

Target the agentic weakness seen in the Instruct model (echoing system prompts, wrong
directory, repeated compaction summaries). A reasoning distill should think before acting
→ more reliable tool-call decisions. Combined with the sliding window (no compaction) +
Hermetis (selective injection) → the closed-loop design.

## Config — reasoning flips ON

- `--reasoning-format deepseek` (thoughts → `message.reasoning_content`)
- `-fa on` (flash attention; the model card's recommended mode)
- `--jinja` (template)
- NOT `--reasoning off` (that is the Instruct fix; wrong for this model)

## Steps (gated on download + GPU free — avatar capture holds it)

1. Download Q2_K (5.0 GB, safest 8 GB fit) or Q4_K_M (8.1 GB — borderline, check VRAM).
2. Launch via `vitriol` overrides (`--model=... --reasoning-format deepseek --flash-attn on`).
3. Verify: loads (mellum2 arch), VRAM fits, `/v1/models` reports.
4. **A/B agentic tool-call test** (the exact failing scenario: system prompt + plan mode +
   tools + "inspect the repo"): Instruct vs Thinking.
5. Check `<think>`/`reasoning_content` handling through opencode — must NOT leak into
   visible output.
6. If solid → make it the opencode agent (Lapis Occultus model entry → this).

## Risks

- `reasoning_content` mishandling by opencode/@ai-sdk (thoughts leak or break the loop).
- VRAM fit (Q4_K_M 8.1 GB on an 8 GB card with the desktop).
- Thinking overhead → slower visible answers (hidden `<think>` tokens cost decode time).
