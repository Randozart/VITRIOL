---
name: dispatch-guidance
type: tool-guidance
target_tool: dispatch
priority: 6
token_cost: 140
user-invocable: false
---
## Dispatch Tool (sub-coders)
Delegate focused research to isolated child little-coder sessions ("sub-coders").
Each runs in its own context window, can READ the repo and BROWSE online
(read, grep, glob, webfetch, websearch, browser, read-only bash) but CANNOT
edit or write files. Each returns a concise report. Use this to gather
information without filling your own context with raw file dumps or web pages.

SINGLE: `task` (string) — one research question.
PARALLEL: `tasks` — array of `{label, task}`, up to 4, run concurrently.
OPTIONAL: `cwd` (defaults to the current working directory).

RULES:
- Give each parallel task a short, distinct `label` — it shows in the live tracker.
- Ask narrow, answerable questions ("how does auth work in this repo?", not "do everything").
- You receive only each sub-coder's short report; the full transcript stays in the
  tool's details (not in your context). Act on the reports.
- Sub-coders can't change files — do the editing yourself after they report back.

EXAMPLE (single):
```tool
{"name": "dispatch", "input": {"task": "Find where sessions are persisted in this repo and summarize the format."}}
```

EXAMPLE (parallel):
```tool
{"name": "dispatch", "input": {"tasks": [
  {"label": "repo auth", "task": "How does authentication work in this codebase? Cite files."},
  {"label": "lib docs", "task": "Look up the current recommended API for the jose JWT library online."}
]}}
```
