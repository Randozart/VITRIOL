# Jinja `tojson(sort_keys=True)` — unblock the Inkling tool-call parser

**Date:** 2026-11-23 · **Status:** implementing · **Supersedes nothing** (new gap)

## Why

With Inkling 2 Mini live, any request that carries `tools` fails:

```
400 Unable to generate parser for this template. Automatic parser generation failed:
    NotImplemented: tojson sort_keys=true not implemented
```

Chain, both ends:

1. `common/chat.cpp:3765` — the server derives a tool-call parser by *running* the
   model's chat template; exceptions are rethrown as "Unable to generate parser".
2. `common/jinja/value.cpp:251` — `tojson` throws `not_implemented_exception` when
   `sort_keys=true`, and the Inkling template calls
   `| tojson(separators=(",", ":"), sort_keys=true)`.

Plain chat requests never touch this path, which is why smoke tests passed and agent
turns failed.

## Scope evidence

The template's real construct set (string literals stripped — an earlier reading that
counted `|content_thinking` and `|message_user` as filters was my own regex artifact;
those are literal text):

| construct | supported today |
|---|---|
| filters `tojson`, `dictsort` | `dictsort` yes; `tojson` **not with sort_keys** |
| `if/elif/else/for/macro/set` | yes |
| `generation` / `endgeneration` (HF extension) | yes (`common/jinja/parser.cpp:207`) |
| globals `namespace`, `raise_exception` | yes |

So one gap, not an unrunnable template.

## Design decision: port, don't invent

Upstream master already implements this — `common/jinja/value.cpp` `tojson` (274-290)
and the object branch of `value_to_json_internal` (1767+): collect `(key, value)` pairs,
`stable_sort` by key when `sort_keys`, and **recurse with the flag** so nested objects
sort too. Its comment states the semantic:

```
// same order as python json.dumps(sort_keys=True): utf-8 byte order is code point order
```

Adaptation: upstream formats float keys with a `float_json()` helper that this fork does
not have (0 hits). Our serializer already stringifies keys via `as_string()`, and tool
argument keys are always strings, so we keep `as_string()` and record the deviation.
Keys are compared as UTF-8 byte strings, which equals code-point order — the property
Python guarantees.

## Stages

| stage | work | gate |
|---|---|---|
| **J1** | port `sort_keys` into `common/jinja/value.cpp` (+ `PROVENANCE` note); add `test-jinja` cases | `test-jinja` and `test-jinja-py` green |
| **J2** | rebuild, restart the Inkling engine, POST a request carrying `tools` | **200**, not 400 — or the next named `NotImplemented`, which becomes its own sub-step |
| **J3** | inspect the generated parser's verdict on a real completion | tool call parses; non-tool golden unchanged |
| **J4** | commit, restart, report | I1-I6 |

## Invariants

- **I1 — no behaviour change without the flag.** `tojson` without `sort_keys` is
  byte-identical to today. *Pinned by:* the existing `tojson` cases in
  `tests/test-jinja.cpp` (they assert exact strings) staying green.
- **I2 — Python parity.** `sort_keys` matches `json.dumps(sort_keys=True)`, recursively,
  including nested objects and arrays of objects. *Pinned by:* three new `test-jinja`
  cases with literal expected strings, plus the differential `test-jinja-py` target
  which compares against real Python.
- **I3 — insertion order preserved by default.** Objects keep their declared order when
  `sort_keys` is absent (the serializer's `IMPORTANT: need to keep exact order`
  contract). *Pinned by:* a new control case whose insertion order is deliberately not
  alphabetical.
- **I4 — plain chat unchanged.** A fixed non-tool prompt returns byte-identical text
  before and after. *Pinned by:* md5 of the completion on the live engine.
- **I5 — the actual goal.** A tools-carrying request on the Inkling template returns
  200. *Pinned by:* the J2 curl assertion.
- **I6 — shared code stays safe.** `value.cpp` is used by every model; the 27B path must
  be unaffected. *Pinned by:* I1 (no flag ⇒ no change) plus the non-tool golden.

## Named tests

- `test-jinja` / `test-jinja-py`: `tojson sort_keys=true flat`,
  `tojson sort_keys=true nested object`, `tojson sort_keys=true array of objects`,
  `tojson insertion order preserved (control)`.
- `J2` runtime gate: `curl` with a `tools` array → HTTP 200.

---

## Results (2026-11-23)

### J1 shipped

`tojson(sort_keys=True)` ported into `common/jinja/value.cpp` (+ header param, defaulted so
every existing caller is unchanged): collect `(key, value)` pairs from
`as_ordered_object()`, `stable_sort` when the flag is set, recurse with the flag so nested
objects sort too. Keys compare as UTF-8 bytes == code point order, which is the property
`json.dumps(sort_keys=True)` guarantees. Deviation from upstream recorded: upstream formats
float keys with a `float_json()` helper this fork lacks; our serializer already uses
`as_string()` for keys and tool-argument keys are always strings.

Tests: 4 new `test-jinja` cases (flat, nested, array-of-objects, insertion-order control).

```
test-jinja        329 tests / 1450 assertions / 0 failures
test-jinja -py    296 tests /  282 assertions / 0 failures   (Python differential)
```

### Gate J2: the 400 is gone

A `tools`-carrying request on the Inkling template now returns **HTTP 200** (was 400).

### A second, larger bug found behind it — ours

With the 400 fixed, the model produced garbage: `<|im_start|>system>\n(OpenAI)` repeated,
then a malformed echo. Cause was **not** the template:

`tools/server/server-common.cpp:1342-1370` is a VITRIOL patch (added for Mellum2's SWA
tool-awareness) that injects a hardcoded **ChatML** reminder
(`<|im_start|>system\nAvailable tools: ... <|im_end|>`) into any prompt carrying tools, and
for templates without a ChatML generation marker it *appended* it to the prompt end. On a
channel-format template that splices foreign text into the model's input.

Fix: inject only when the `<|im_start|>assistant` marker actually exists — the reminder is
ChatML by construction, so a template without that marker must not receive it. Verified
through the service path with no env var: output is now the model's native channel format.

### Remaining gap (named, not hidden)

The template's assistant tool-call envelope is:

```
<|message_model|><name><|content_invoke_tool_json|>{"name":<name>,"args":{...}}
```

The model emits the **correct shape but omits both markers**:
`list_files{"name":"list_files","args":{"path":"."}}`. Because the generated parser is
keyed on those markers, `tool_calls` stays `null` and the call arrives as content. So
Officina would see text, not a tool call.

Next step is one of: force the envelope on the generation prompt, make the parser tolerate a
markerless call, or constrain decoding with a grammar built from the envelope. Each is a
separate change with its own gate.
