# PROVENANCE — every borrowed thing, cited — 2026-08-31

Law of this tree (First-Party Mandate, AGENTS.md 2026-08-31; owner
directive): upstream projects are mined, never depended on as runtimes.
Every file that carries borrowed code or a ported design cites its origin
in a header comment. This registry is the index of those citations.

## License of this tree

VITRIOL is dual-licensed **`Apache-2.0 OR MIT`** (SPDX expression; see
`LICENSE`, `LICENSE-MIT`, and AGENTS.md "Licensing and Provenance",
2026-08-31). "our MIT" / "ours" in the tables below means: offered under
that same dual license. The intake rules are unchanged — the table in
AGENTS.md governs INCOMING third-party code only.

Layering note for the llama.cpp fork (`llama.cpp/` submodule): upstream
ggml/llama.cpp code remains under its original **MIT** license (copyright
The ggml authors, `llama.cpp/LICENSE`); VITRIOL's modifications and
additions to the fork are offered under this repo's `Apache-2.0 OR MIT`
dual license. Neither license term is withdrawn, so the combined work is
distributable under either.

## Libraries (declared dependencies — permitted, pinned)

| Dependency | License | Version | Use | Notes |
|---|---|---|---|---|
| `@earendil-works/pi-coding-agent` | MIT | 0.83.0 (pinned; corrected 2026-08-31 — earlier entries wrongly said Apache-2.0) | agent runtime: loop, tools, extension host, TUI, session manager | Library ONLY — never a shipped frontend. Upgrades are deliberate, tested bumps |
| `@sinclair/typebox` | MIT | latest compatible | tool parameter schemas (pi requirement) | — |
| typescript / vitest / @types/node | Apache-2.0 / MIT | dev | quality gates | — |

## Extension provenance

| Extension | Origin | License | Citation |
|---|---|---|---|
| llama-cpp-provider | itayinbarr/little-coder `.pi/extensions/llama-cpp-provider` @ 1a6ee8b (2026-08-31) | Apache-2.0 | header in `index.ts`; divergence: pkgRoot = officina root; `loadProviders`/`parseModelsFile` refactored to house flat-flow style |
| subagent, plan-mode, phase-model, deep-research, skill-inject, knowledge-inject | itayinbarr/little-coder `.pi/extensions/*` @ 1a6ee8b | Apache-2.0 | ported verbatim (P2 "load-bearing three" + injectors); layout divergence documented in `config.test.ts` |
| context-relay, small-lane, rewind, vitriol-checkpoint, permissions-guard, tool-result-clearer, rtk-output, task-state, snapshot, repo-map ext, diagnostics-loop, _shared | **Owner-authored** — written for trismegistus 2026-08-29..31 (commits 09fdac4..1a6ee8b era) | our MIT | no external provenance; ports of ourselves |
| memory | new (SS2a), successor to hermes-bridge contract (REPORT-02 step 16) + hermes memory-extractor concept | ours | header in `index.ts` |
| injection-guard | port of trismegistus/hermes-plugins/injection-guard/guards.py @ 237e424 (owner-authored, MIT) | MIT | header in `index.ts`; patterns + mode discipline identical |
| caveman | port of trismegistus/hermes-plugins/caveman-rules/compress.py @ 237e424 | MIT | header in `compress.ts`; ruleset identical, code-span protection identical |
| memory-extractor | port of trismegistus/hermes-plugins/memory-extractor/extractor.py @ 237e424 | MIT | header in `index.ts`; same rules/confidences, queue not auto-trust |
| ask | @earendil-works/pi-coding-agent `examples/extensions/question.ts` @ 0.83.0 | MIT | header in `index.ts`; verbatim except header + explicit param types + `@sinclair/typebox` import path |
| notify-done, inline-bash, session-name | @earendil-works/pi-coding-agent `examples/extensions/{notify,inline-bash,session-name}` variants @ 0.83.0 | MIT | headers in each `index.ts`; notify-done rebranded + require→ESM; all kill-switched. Audit: `.opencode/plans/pipeline-mining-audit-2026-08-31.md` |
| coupling, session-panel, officina-header, vitriol-decode, agent-mode | new, this repo | ours | — |

## NOTICE propagation (Apache-2.0 §4(d))

little-coder carries a NOTICE attributing its own upstream
(SafeRL-Lab/clawspring, Apache-2.0). The attribution is reproduced in
`.pi/extensions/NOTICE` and flows to all redistributions of the vendored
extensions. The runtime `@earendil-works/pi-coding-agent` and
`@earendil-works/pi-tui` are **MIT**; their patched vendor copies
(`runtime/patched/*.officina.js`) carry derivation headers stating source
and modifications (MIT §2-4: notices retained, changes stated).

## Designs and assets

| Asset | Origin | Cited at |
|---|---|---|
| Braille gradient bars (6-dot cells, fill order, ramps) | VITRIOL `vitriol-tui/src/braille.rs` (own, Apache-2.0) | `vitriol-decode/braille.ts` header |
| Vitriolum palette + ramps | VITRIOL `vitriol-tui/src/theme.rs`, roots in owner's Officina-dark VS Code theme | `theme/officina.json` vars |
| Extension palette module (single source; honey retargeted to antidote `#ff5f1f`) | VITRIOL `vitriol-tui/src/theme.rs` (own, Apache-2.0); parity-tested by `_shared/vitriolum.test.ts` | `_shared/vitriolum.ts` header |
| Shared engine telemetry poller | own work; parsers from `vitriol-decode/decode.ts` (own, Apache-2.0) | `_shared/engine.ts` header |
| Caveman ruleset (−65% measured) | owner's caveman skill; hermes plugin port | `caveman/compress.ts` |
| Small-model split, skills-lazy-loading, hook deny/halt, LSP-edit ideas | Crush v0.91.2 (FSL-1.1-MIT) — PATTERNS only, no code | `docs/CRUSH-MINING-PLAN-2026-08-31.md` |
| Legacy names | `refs/trismegistus/turns/*` git refs and `~/.local/state/trismegistus` retained deliberately for data continuity (SS4 decision) | docs/OFFICINA.md |

## Retired dependencies (fold-in record)

- little-coder: fallback removed (SS1); models.json parity → officina
- hermes-agent: memory (SS2a), chat (retired), guards (SS2a); caveman +
  extractor ports close SS2b; nothing references its runtime
- trismegistus repo: private archive; docs copied to
  `docs/officina-archive/`

Enforcement: `scripts/selfcheck.sh` fails the tree if any live file in the
table's first column loses its provenance header, or if external project
paths reappear in live code.
