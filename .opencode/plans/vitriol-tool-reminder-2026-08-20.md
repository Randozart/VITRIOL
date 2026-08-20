# VITRIOL Tool Reminder Injection — keep tools in SWA reach for Mellum2

Status: complete (verification passed 2026-08-20)
Date: 2026-08-20

## Problem

Hermes Agent (Nous Research, MIT) driving Mellum2-Claude-Thinking Q8_0 via
VITRIOL: after ~1024 tokens of conversation, the agent **forgets it has tools**.
Root cause: Mellum2 has SWA-1024 on 21/28 layers — those layers can only attend
to the last 1024 tokens. Hermes sends the full tool schemas every request
(`conversation_loop.py:2485`), but VITRIOL's chat template renders them into the
**system message at prompt start** (Mellum2 GGUF `tokenizer.chat_template`).
As history grows, the tool block drifts >1024 tokens from the generation point
and 21/28 layers physically cannot see it. The lazy tool-call grammar
(`common/chat.cpp:913`, triggers only on `[TOOL_CALLS]`) can't help if the model
never attempts a call.

Not a VITRIOL bug, not a hermes bug — a model-architecture limitation surfaced
through positional placement.

## Fix — VITRIOL fork tail-injection

Keep full context (131072). In `tools/server/server-common.cpp`, after
`common_chat_templates_apply()` builds `chat_params.prompt`, inject a compact
**tool reminder** right before the final `<|im_start|>assistant` generation
marker. The reminder lists tool names AND teaches the call envelope
(`To call a tool, emit <tool_call> followed by {"name": "<tool>", "arguments": {...}}`),
~100-150 tokens → always within the last 1024 tokens of the prompt → all 28
layers see "you have tools" and know how to call them every turn.

The trigger marker is auto-detected from the template source
(`common_chat_templates_source()`): `<tool_call>` for the hermes/Mellum2
template, `[TOOL_CALLS]` for Ministral/Qwen. This matches the template's
per-call trigger marker used by the lazy grammar.

Safety:
- The chat PEG parser parses only generated output (`server-task.cpp:155`), not
  the input prompt → injected text cannot corrupt tool-call parsing.
- Full JSON schemas stay at prompt start (7 full-attn layers + lazy grammar).
- Gated on `!inputs.tools.empty()` → raw completions untouched.
- Applies to any agent (hermes, opencode, curl), not a hermes hack.
- Disable with `VITRIOL_TOOL_REMINDER=0`.

## Components

1. **Tool reminder** (server-side, this fix): `<|im_start|>system
   Available tools: name1, name2, ... To call a tool, emit <tool_call> followed
   by {"name": "<tool>", "arguments": {...}}.<|im_end|>` block injected before
   the generation marker. Gives awareness + callable names + call envelope.
2. **Capability tool** (hermes-side, optional): a `get_tool_capabilities` tool
   that returns full schema/args for a requested tool name on demand. Since full
   schemas sit >1024 tokens away at prompt start, this lets the model pull exact
   call signatures into the recent window when it needs them.

## Files

- `llama.cpp/tools/server/server-common.cpp` — reminder injection (this change)
- `llama.cpp/common/chat.cpp` — `common_chat_tools_reminder()` helper
- `llama.cpp/common/chat.h` — declaration
- hermes-agent — capability tool (separate, hermes side, optional/not built)

## Verification (2026-08-20)

1. Build (server-common.cpp recompile only, fast); `sudo vitriol setup` ✓
2. **9k-token / 15-tool A/B** (curl, port 8091): reminder ON → structured
   `tool_calls: list_dir {"path": "/home/randozart"}` ✓; OFF → plain-JSON content
   stall `{"tool": "list_dir", "path": "/home/randozart"}`. **Decisive.**
3. **21k-token / 18-tool A/B**: both ON and OFF make structured calls — at that
   depth the 7 full-attention layers still reach the tools.
4. **Real hermes session** (`hermes -z` coding toolset, port 8279): session
   `20260820_145224_08eba4` shows 4 proper structured calls — `search_files` ×3
   then `terminal ls -la` after a tool error (agentic adaptation) — at 20k+
   prompt tokens. Server log confirms reminder injected (9x, 191 bytes before
   the `<|im_start|>assistant` marker).
5. Reminder lands 346 bytes before the generation marker → inside SWA-1024.

## Result

The reminder converts the "writes JSON as content instead of triggering the
tool-call grammar" stall into proper structured `tool_calls`. Hermes now keeps
tool awareness across long sessions. Capability tool deferred — the reminder
(awareness + envelope) proved sufficient in the real hermes flow.