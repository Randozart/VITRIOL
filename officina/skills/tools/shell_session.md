---
name: shell-session-guidance
type: tool-guidance
target_tool: ShellSession
priority: 10
token_cost: 140
user-invocable: false
---
## ShellSession Tool
Stateful bash: cd, env vars, and job state persist across calls.

REQUIRED: command (one shell command, NOT a script)
OPTIONAL: timeout (seconds; default 30, use 120–300 for installs/builds)

RULES:
- ONE command per turn. Read the output before proposing the next.
- State persists: set vars / cd once, reuse later.
- Never run interactive commands (`vi`, `less`, `top`, `python` bare REPL).
  Use non-interactive equivalents (`cat`, `sed -i`, `python -c '…'`).
- Output ends with `[exit=N cwd=… timed_out=…]` — check exit=0 before claiming success.
- If timed_out=true, do NOT just retry; diagnose (longer timeout, narrower command).

EXAMPLE:
```tool
{"name": "ShellSession", "input": {"command": "cd /work && ls -la"}}
```

EXAMPLE with timeout:
```tool
{"name": "ShellSession", "input": {"command": "pip install -q requests", "timeout": 180}}
```
