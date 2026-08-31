# Crush Mining Plan — 2026-08-31

**Status:** PROPOSED — awaiting owner approval before any Tier B item is
reordered to include it. Dated document per Rule 14; dispositions recorded
here as items complete.
**Sources examined:** live Crush v0.91.2 runtime (own config + skills +
hooks contract), upstream README + docs (charmbracelet/crush @ main,
2026-08-31), our scaffold extension surface
(`~/Desktop/Projects/little-coder/.pi/extensions/`, 45 dirs), hermes plugin
surface, `~/.config/crush/{crushrc,crush.json}`.
**Goal restated:** make `tris` the harness the owner prefers over Crush,
hermes-agent, OpenCode, and little-coder for daily local work — by mining
what makes Crush feel snappy and combining it with what only Trismegistus
has (hard budget law, KV continuity, cert gates, governance).

---

## §0 Licensing reality (corrects an earlier statement)

Crush is **FSL-1.1-MIT**, not plain MIT. FSL grants everyone the right to
use, copy, modify, and redistribute the code without restriction EXCEPT
providing a competing product as a service before each file's
License-Change date (2 years), after which it converts to MIT. A personal
local harness is squarely permitted — **we may mine literal code.**
Attribution is still house style: record provenance in each adopted file
header (`// from charmbracelet/crush <sha>, FSL-1.1-MIT`) so the provenance
discipline (Rule 4/13) extends to borrowed code. Upstream stays upstream
(Rule 9): adopted code is vendored INTO our extension/plugin surfaces, never
submodule-patched.

## §1 Live findings from this investigation (fix regardless of plan)

| # | Finding | Severity | Action |
|---|---------|----------|--------|
| L1 | Z.ai API key stored in plaintext in `~/.config/crush/crush.json` | SECURITY | Rotate key; move to `ZAI_API_KEY` env (Crush reads it natively); add the file to any backup-exclusion policy (feeds F6 retention work) |
| L2 | Crush small-model lane (`127.0.0.1:8287`, Mellum2 via VITRIOL) conflicts with the hermes server on :8279 — both want the GPUs; crushrc comments admit one must die | OPERATIONAL | M8 (below) resolves it structurally. Until then: `tris down` before heavy Crush use, or a `tris lanes crush` helper that performs the documented swap |
| L3 | Crush background tasks (titles, `auto_summarize` compaction) silently fail while the small lane is down — observed live via agentic_fetch connection refused | OPERATIONAL | Same fix as L2; also a UX lesson — degrade LOUDLY (see §4 UX-3) |

## §2 Why Crush feels snappy — mechanism decomposition

Not one feature; seven compounding budget decisions:

1. **Small-model split.** Cheap local model does titles, summarization,
   compaction; the expensive model only does agent turns. The 27B never
   spends decode seconds on bookkeeping.
2. **Lazy procedure loading (skills).** ~20-token trigger descriptions live
   in the system prompt; 1-2K-token procedure bodies load only on match.
   Fixed prompt stays tiny; capability is unbounded.
3. **Deterministic hooks, not model judgment.** `PreToolUse` shell hooks
   gate/rewrite/inject with a 3-exit-code contract (0 allow/inject/rewrite,
   2 block tool + model retries with a one-line reason, 49 halt turn).
   Governance costs ZERO tokens — the model never reasons about policy it
   can memorize as a rule.
4. **LSP-grounded tools.** Edits go through symbol-level operations
   (replace symbol, semantic rename) plus diagnostics; fewer failed edits
   means fewer retry round-trips — on an 11 t/s rig a failed edit is
   minutes, not seconds.
5. **Subagent fan-out.** Search/read-heavy exploration runs in a subagent;
   only the digest enters the parent context.
6. **Native TUI + instant permission UX.** Ratatui-class rendering, typed
   permission prompts, session picker, zero-latency local tools.
7. **Bash-native config (crushrc).** Executable, cross-platform, `$(...)`
   expansion; config IS documentation.

Every one of these is either already built in Trismegistus (5: subagent,
ReWOO; 3: permissions-guard partially; 6: tris-watch exists), or is a
bounded addition (1, 2, 4, 7-partial).

## §3 Mining map — feature → disposition → landing spot

Each item: what we take (pattern / code / standard), where it lands (Rule 9
surfaces), kill switch + budget (Rules 15/3), and measurement.

### M1 — Small-model compaction lane (Pattern; owner already prototyped it)

Crush's `model small` split, formalized as scaffold law. Today our
`context-watchdog` kicks `ctx.compact()` on the MAIN model. Change: all
compaction/summarization/title/tasks go to a configured `lane: small`
endpoint; the 27B lane only sees agent turns.
- **Landing:** little-coder ext `small-lane` (wraps compaction calls;
  config `lanes.small.base_url` from unified config). Falls back to main
  lane with a loud notice if small is unreachable (L3 lesson).
- **Kill switch:** `lanes.small.enabled` (default ON — this is the point).
- **Measure:** compaction latency + tok cost main-vs-small A/B; expect
  main-lane decode seconds saved per compaction event.
- **Note:** the owner's crushrc already proves the lane works (Mellum2
  12B-A2.5B Q5 on :8287, 131K ctx). M8 fixes the GPU contention.

### M2 — Skills in agentskills.io format (Standard + some code)

Trigger-description + `SKILL.md` body, loaded on match. Adopt the OPEN
STANDARD (Crush, Claude, Cursor all read it) so our skills are portable
and we inherit the ecosystem's skill libraries.
- **Landing:** new little-coder ext `skills-loader`: scans
  `.agents/skills` + `~/.config/agents/skills` + unified-config paths;
  injects ONLY frontmatter (name+description, ~20-30 tok each) into the
  frozen-adjacent tail (cache-safe, Rule 7); on trigger match, appends the
  body as a one-shot tail message. Existing `skill-inject` ext is the
  seed — extend, don't duplicate.
- **Rules-index compiler synergy (RULES-CACHE-POLICY §3):** compile our
  AGENTS.md rules into skills — a rule becomes a trigger + a procedure,
  paying 0 tokens until relevant. This is the mechanism that gets fixed
  overhead from ~20K toward ≤8K.
- **Kill switch:** `skills.enabled`; per-skill `disable-model-invocation`.
- **Measure:** prompt-size --json before/after the diet (P4 pillar
  evidence); trigger precision (false-positive body loads logged).

### M3 — Hook contract: deny-with-reason + halt levels (Pattern; code portable)

Our permissions-guard is fail-closed allow/deny. Adopt Crush's semantics:
- **exit 2 / deny-with-reason:** block the ONE tool call, give the model a
  one-line reason, let it retry. On a 27B this converts most violations
  into self-corrections instead of dead turns.
- **exit 49 / halt:** end the turn (secrets, policy) — user takes over.
- **updated_input shallow-merge rewrite:** hooks can patch tool input
  (e.g. force `--target <dir>` on praetor, rewrite file targets) without
  replacing the rest.
- **Landing:** extend `permissions-guard` + `tool-gating` exts and the
  Hermes DSL with `decision: allow|deny|rewrite` + `halt` + `reason` +
  `patch`. Aggregate multiple gates: deny > allow > no-opinion, halt sticky,
  patches merge in config order.
- **Kill switch:** existing per-gate switches unchanged.
- **Measure:** dogfood log — violations resolved-in-place vs dead turns.

### M4 — LSP-symbol editing (Pattern; largest code-adjacent lift)

Route edits through LSP: symbol-level replace/rename with diagnostics
verification after each edit.
- **Landing:** little-coder `write-guard`/`read-guard-edit` gain an
  LSP-assisted path: on edit failure (whitespace mismatch), try a
  document-symbol-scoped replacement before returning the error to the
  model. Success = fewer retries = direct tok/time saving (Rule 5 numbers
  apply: a retry at 12 t/s on a 400-token thinking budget is ~a minute).
- **Kill switch:** `lsp_edit.enabled` (default ON with fallback to plain).
- **Measure:** edit-retry rate before/after on a fixed task set (A/B, same
  fingerprint).

### M5 — Subagent digest discipline (Pattern only — already built)

Our `subagent` + `deep-research` + ReWOO verdict already match Crush's
shape. Addition from Crush: **sub-agents bypass hooks by default, only the
spawning tool call is gated** — document this in the dispatch-roots
security model (F11), and ensure subagent outputs enter context as
size-capped digests (verify `subagent` ext caps; if not, cap it — ~2K tok).
- **Measure:** parent-context tok per research fan-out, ledger.

### M6 — Workspace/session semantics (Pattern; feeds T3)

Crush: multiple clients attach to one workspace keyed by cwd; live-mirror
an in-progress session; sessions carry busy/attached state.
- **Landing:** tris-watch T3 (cockpit control) + Hermes: the cockpit and
  the gateway should be two views of ONE session ledger, not two
  conversations. Minimum viable: `tris watch` attaches read-only to the
  live Hermes/`tris go` session via the events stream we already have.
- **Measure:** none needed beyond T3 acceptance.

### M7 — Config ergonomics (Pattern only)

`tris` already matches the spirit (up/down/smoke/code/chat/go). Borrow:
`tris logs --follow`, `tris status --json`, and crushrc-style
`if [[ $HOSTNAME == … ]]` conditionals in `tris` profile selection (the
DEV vs master profile switch is currently manual).
- **Kill switch:** n/a (pure CLI).
- **Rule:** config comments carry when/why (Rule 13) — same as crushrc's
  documented swap commands.

### M8 — Unified GPU lane arbiter (NEW architectural item, from L2/L3)

The real fix for the Crush/Hermes GPU collision: ONE arbiter owns the
lanes. The engine is the scheduling authority (Layer Interface Protocol) —
so the arbiter belongs in `tris` + the VITRIOL supervisor, not in two
competing frontends:
- `tris lanes` — show current allocation (which model on which GPU/port,
  which frontends are served);
- `tris lanes crush` — swap to Crush mode: load `mellum2-crush-small` on
  the free VRAM, serve :8287, keep :8279 as master OR park it with an
  honest banner;
- `tris lanes master` — back to qwen38-master + Hermes.
- Honesty rule: the lane state goes in the fingerprint (Rule 4) so no
  benchmark ever runs against the wrong lane silently.
- **VITRIOL-side open question:** can one llama-server instance serve two
  models (master + small) with a tensor_split, or must the small lane ride
  the GTX 1070 Ti alone? Decides whether crush-mode is concurrent or
  swap-based. Measure, don't guess (Rule 3).

### M9 — UX hygiene bundle (small, all worth taking)

- **Degrade loudly:** failed background/small-lane calls surface a visible
  notice (Crush fails silently — observed, L3). Banner in tris-watch.
- **`.trisignore`:** context-search surface respects a project ignore file
  (checkpoints, ledger.jsonl, .pi/rtk/) — mostly free after repo-map.
- **Permission notifications:** tris-watch already has panes; add the
  focus-aware notify hook when T3 lands.
- **Attribution + initialize:** `tris init` that generates AGENTS.md from
  the repo (we have repo-map; compose it).

## §4 Gap analysis — what "prefer over Crush" requires

Crush wins today on: snappiness (native TUI, small-model split), ecosystem
(skills standard, Catwalk model DB, OAuth MCP), polish. Trismegistus wins —
or must win — on:

1. **Hard budget law** (P1): Crush OBSERVES cost; we ENFORCE it. No Crush
   equivalent of cert gates, kill switches, or §R2.8.
2. **KV continuity** (P2): warm checkpoint/restore + `/rewind` of worktree
   AND context. Crush has sessions, not KV.
3. **Governance** (P3): executable gates (M3 makes them cheaper to honor).
4. **Sovereign local-first**: fully local loop incl. the small lane; Crush
   defaults to cloud (Hyper/ZAI).
5. **The diet** (P4): pi base prompt 6,427 tok vs Hermes 19,953 — M1+M2 is
   the plan that makes the harness FEEL like Crush's snappiness at a
   fraction of the fixed cost, which Crush never has to think about.

Honest risk: if the 27B turn quality or 11 t/s latency annoys daily, no
governance feature compensates. That is what the dogfood day is for — and
Crush (cloud, fast) is now the control condition for that experiment.

## §5 Execution order (inserted into Tier B; Tier A untouched)

Tier A audit fixes proceed as pre-registered. Tier B becomes:

1. **B0 (new, blocking):** L1 key rotation; M8 lane arbiter MVP
   (`tris lanes` read-only first).
2. M1 small-lane compaction (standalone ext → test → wire, per Extension
   Development Protocol).
3. M2 skills-loader + rules-index compiler (the diet — biggest P4 lever).
4. M3 hook deny/halt semantics (permissions-guard v2).
5. M4 LSP-assisted edit fallback.
6. M5-M7, M9 as hygiene batch.
7. M8 swap-mode completion (after the two-models-on-one-instance question
   is measured).

Every item: typecheck + tests + smoke on request-path changes + praetor on
the changed dir + dated ledger entry. Every perf claim: baseline + after
table, same fingerprint (Rule 3). Everything ships with a kill switch
(Rule 15).

## §6 Open questions for the owner

1. Approve M-items into Tier B in the §5 order? (Tier A untouched.)
2. M8: is swap-based crush-mode acceptable initially, or do we hold out
   for concurrent two-model serving (needs VITRIOL-side measurement)?
3. M2: adopt agentskills.io format as THE skill format for tris (dropping
   any bespoke format in skill-inject), yes/no?
4. Rotate the Z.ai key now? (L1 — one command, recommended regardless.)
