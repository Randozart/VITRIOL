# Reference 6 — the HTTP server surface

Everything that shapes how clients talk to llama-server.

PROVENANCE: arg.cpp semantics; REBIS gateway interplay notes 2026-08-22.

## Bind & identity

| flag | does | REBIS |
|---|---|---|
| `--host` | bind address; `.sock` suffix = unix socket | 127.0.0.1 loopback-only policy |
| `--port` | TCP port | Sol :8279 (Au), Luna :8247 (Ag) |
| `--api-prefix` | serve under a path prefix | unused |
| `--path` | static file root (WebUI) | unused |

## Auth & TLS

`--api-key` (comma-separated list) / `--api-key-file`: bearer-token
authentication; unauthenticated requests to protected endpoints fail.
`--ssl-key-file/--ssl-cert-file`: PEM TLS termination.

REBIS runs loopback without keys — adding keys would require updating every
client shim; loopback-only is our boundary instead.

## Concurrency

| flag | default | does |
|---|---|---|
| `-np, --parallel N` | 1 | number of slots; window divides across them in upstream (this fork reports full ctx per slot) |
| `--threads-http N` | HTTP worker threads | request handling parallelism |
| `-to, --timeout S` | read/write timeout | **relevant**: hermes timed out on steering latency through the gateway; know your client's budget |
| `--sleep-idle-seconds` | disabled | sleep GPUs when idle — conflicts with day-long always-on use |

## Endpoints enabled per-launch

| flag | endpoint | consumer |
|---|---|---|
| `--slots` | GET /slots | TUI progress bars, gateway introspection |
| `--metrics` | GET /metrics | prometheus counters/gauges |
| `--props` (+default) | GET /props | model/ctx info; draft acceptance |
| `--embedding(s)` / `--pooling` | /v1/embeddings | embedder service (separate head) |
| `--webui-config(-file)` | WebUI defaults | unused |
| `--tools` | built-in agent tools | experimental; do not enable untrusted |
| `--media-path` | file:// media for multimodal | unused |

## Chat template family

`--jinja` applies model metadata template (required for Thinking models;
enables `/apply-template`). `--chat-template(-file)` overrides entirely;
`--chat-template-kwargs JSON` passes extra variables — this is how
`enable_thinking:false` reaches Mellum's template.

`--reasoning-format none|deepseek|…` controls where `<think>` content lands
(content vs reasoning_content); `-rea/--reasoning on|off|auto` toggles it;
`--reasoning-budget N` caps thinking tokens (-1 unrestricted). Measured: an
unbudgeted Thinking drafter burned 8192 tokens reasoning before answering.

## Interactive leftovers

`-r/--reverse-prompt`, `-f/--file`, `--in-file`, `--context-file`,
`--system-prompt(-file)`, `-n/--predict`: mostly CLI-mode conveniences;
server ignores most, but `/completion` honors `n_predict`/`cache_prompt`
per-request.

## Router extras

`--models-dir/--models-preset/--models-max`: multi-model router mode — a
built-in cousin of what Mercury does externally. Untested here; if it ever
supports per-model audit hooks it could replace part of the gateway.
