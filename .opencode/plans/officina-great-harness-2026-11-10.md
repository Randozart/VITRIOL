# Officina "great harness" pass — plan (2026-11-10 11:32)

Status: approved by operator ("Everything"). Execution started 2026-11-10 11:32.

## Execution status (2026-11-10 11:52)
- Phase 0: DONE — `/opencode` (no `/import` shadow), session-panel RPC select fallback, `/plan` + ctrl+q, F2 deep-research, `+grep,+find,+ls`, env-file loader, predicted-token tok/s, no debug stderr.
- Phase 1: DONE — full extension manifest (all index.ts dirs, project shadowing), `-a` removed (trust select), `TRIS_SNAPSHOT=1` default, subagent launcher ported to pi CLI + `tool-gating`.
- Phase 2: DONE — confirm/select/input/editor modals with RPC responses + tests.
- Phase 3: DONE — ask/auto/yolo modes, protected roots, bash command policy, `/permissions` modal, `/yolo`, sidebar `perm` chip, persistence.
- Phase 5: PARTIAL — steer (Enter while streaming) + Alt+Enter follow-up, `/rename`, `/export`, `/clone`. Remaining: `/tree`, `/fork` list.
- Phase 6: PARTIAL — subagent launcher + gate fixed; plan/deep-research can now run. Needs live orchestration test.
- Phases 4/7/8: snapshot/rewind carried + armed; diff-review, MCP, memory UI polish remain.

## Perf pass (2026-11-10 12:35)
Measured live: prefill 180–200 t/s, decode 16–20 t/s at 197k, MTP acceptance 68.6%,
queue histogram 45/531 >2s, engine swap 4.05 GiB, prompt cache 191k/197k.

Applied:
- pi compaction override for `llamacpp/Lapis Occultus`: reserveTokens 65536,
  keepRecentTokens 24000 (compact at ~196k; new sessions).
- `~/.vitriol/officina/env`: `LITTLE_CODER_CLEAR_KEEP=8` (fewer stale tool results).
- `~/.vitriol/config`: `cache_ram 12288 → 8192` for the next engine start
  (host swap trim; fingerprint will carry `cram=8192`).
- Profiles saved: `qwen38-buffedmod-262k-cache12-mtp3` (baseline),
  `qwen38-buffedmod-262k-cache8-mtp3` (new default), `qwen38-buffedmod-262k-cache8-mtp2` (A/B).
- Closed idle `vitriol-tui` (only remaining extra engine client).
- Sidebar ctx row turns gold ≥75%, violet ≥90%, and prints `compact soon: /compact`.

A/B recipe (only in a safe window — engine restart kills active turns):
```bash
vitriol config load qwen38-buffedmod-262k-cache8-mtp2
systemctl restart vitriol-server.service
# continue at a comparable depth; read tg lines:
grep "n_gen =.*tg =" /tmp/opencode/vitriol_gen.log | tail -20
vitriol config load qwen38-buffedmod-262k-cache8-mtp3
systemctl restart vitriol-server.service
# compare tg/tg_3s and draft acceptance; keep the winner.
```
Restart the TUI after edits so it loads the new env (clear-keep 8, YOLO mode, compaction).

## Current findings

### Running session (11:32)
- Engine PID 243473: Qwen3.8-27B BuffedMod IQ3_S + MTP n=3, `-ts 27,21`,
  ctx 262144, q4_0 KV, sparse/probe scoring, `--slot-save-path /scratch/kvtier`.
- Slot 0: 178,539 prompt tokens, 173,105 served from prompt cache, 5,430 processed.
- Prompt cache is healthy; `/scratch/kvtier` had grown to 85 GiB (now manageable via `/cache`).
- Officina TUI (PID 970182) is running a **deleted pre-change binary**; pi child is
  PID 970191 (`node`, renamed `officina`). Restart required for this session's work.
- **tok/s is understated ~2.6x**: `_shared/engine.ts` uses `llamacpp:n_decode_total`
  (decode steps, excludes speculative). Live counters: n_decode 109,875 vs
  tokens_predicted 289,345. Must use `tokens_predicted_total`.

### Harness gaps
1. Canonical path loads only 7 extensions (`tui/src/rpc/bridge.rs:43`); VITRIOL root
   has no `.pi/extensions`, so permissions-guard, snapshot, rewind, memory, repo-map,
   subagent, ask, plan-mode, skills, knowledge, quality gates are absent.
2. Rust TUI never reads `~/.vitriol/officina/env` (`officina.mjs:44` does), so
   routing/OmniRoute/snapshot flags do not apply.
3. RPC dialogs `confirm/select/input/editor` are auto-cancelled
   (`tui/src/tui/mod.rs:1760`) -> approvals, plan questions, ask, rewind, trust fail.
4. `-a` always passed (`bridge.rs:143`) -> project trust bypassed; bash ungated.
5. Snapshots default OFF (`TRIS_SNAPSHOT=1` opt-in); snapshot/rewind not carried.
6. Subagent/plan-mode/deep-research dead: `subagent/spawn.ts:108` expects missing
   `bin/little-coder.mjs`; plan-mode only binds `ctrl+q`, not forwardable from Rust;
   plan-mode refuses `--mode` sessions. No tool-gating extension exists.
7. Only `read,bash,edit,write` active; `grep/find/ls` off.
8. Rust `/import` shadows session-panel's `/import` (`session-panel/index.ts:645`).
9. Pi RPC controls ignored: steer/follow_up, fork/clone, get_tree/navigate_tree,
   set_session_name, export_html, auto-retry, clear_queue, user bash.

## Operator decisions
- Implement everything, phased.
- Load the **full** extension set everywhere.
- Enable approvals + per-turn snapshots by default.
- Add a persistent YOLO toggle that still guards sensitive root/secret paths.

## Phase 0 — immediate corrections
- Rename OpenCode importer command to `/opencode`; keep session-panel `/import`.
- session-panel `/import`: RPC fallback from `ui.custom` to `ui.select`.
- Add `/plan` command to plan-mode; map `ctrl+q` -> `/plan`, `F2` -> `/deep-research`.
- Enable `+grep,+find,+ls` in the pi spawn.
- Fix tok/s: use `tokens_predicted_total` for rate and boot total.
- Load `~/.vitriol/officina/env` into the Rust process and pi child (`??=` precedence).
- Tests for command rename, env precedence, predicted-token rate.

## Phase 1 — full extension + trust surface
- Replace the hardcoded carry list with all officina extensions (dirs with index.ts,
  excluding `_shared`); skip names shadowed by `<cwd>/.pi/extensions` or
  `<cwd>/.officina/extensions`.
- Add `--extensions full|project|none` override (default full).
- Remove unconditional `-a`; rely on pi's project-trust `select` dialog.
- Startup budget + per-extension failure isolation; kill switches documented.

## Phase 2 — RPC dialog subsystem
- Modal queue for `select`, `confirm`, `input`, `editor`; correct responses.
- Keymap: arrows/j/k, Enter, Esc, y/n; text editing for input; opaque dialog surfaces.
- Cancel pending dialogs on abort/session switch/shutdown.
- Unlocks trust, permissions ask, plan questions, ask, rewind confirmation.

## Phase 3 — permissions, YOLO, sensitive roots
- permissions-guard modes: `ask` (default), `auto`, `yolo`.
- Protected roots always ask/deny in every mode (realpath-canonicalized):
  `/etc`, `/boot`, `/usr`, `/sys`, `/proc`, `/dev`, `/var`, `~/.ssh`, `~/.gnupg`,
  `~/.aws`, `~/.config/trismegistus`, `~/.vitriol/secrets`, `.env*`, `*.pem`,
  private keys.
- bash policy: hard-deny catastrophic (`rm -rf /`, `mkfs`, `dd of=/dev/...`),
  ask for risky (`sudo`, `git push`, `git reset --hard`, `curl|sh`, package managers).
- `/permissions` modal + persistent `perm ask|yolo` chip; YOLO stored in the
  Officina env file.
- Tests: protected globs, symlink escape, command patterns, YOLO does not bypass protected roots.

## Phase 4 — snapshots + rewind
- `TRIS_SNAPSHOT=1` by default in git repos (keep `0` kill switch).
- Carry snapshot + rewind; `/rewind` via dialogs; KV-slot pairing with honest half-restore.
- Skip clean trees, async, prune old refs.

## Phase 5 — RPC control parity
- Add steer/follow_up, fork/clone, get_tree/navigate_tree, export_html,
  set_session_name, set_auto_retry, clear_queue, user bash.
- UI: Enter while streaming = steer; Alt+Enter = follow-up; `/tree`, `/fork`,
  `/clone`, `/rename`, `/export`, `!cmd`; queue indicator.

## Phase 6 — subagents, plan, research
- Port subagent spawn off `bin/little-coder.mjs` to headless
  `pi --mode json -p --no-session` + portable extensions.
- Add `tool-gating` extension (wire `_shared/allowed-tools.ts`) for read-only children.
- Fix plan-mode orchestration and approve/implement flow; fix deep-research deps.

## Phase 7 — agent quality
- Load memory/skills/repo-map/knowledge; add missing commands/UI.
- Diff blocks for edit/write; approval prompts carry diff summaries where possible.
- Notifications, tool-result clearer, diagnostics-loop, format/import/verify gates.
- `/mcp` status + optional MCP server config; cost/ledger/auto-route parity.

## Phase 8 — hardening
- Full-load startup budget, failure isolation, kill switches, docs.
- Live smoke: trust, ask, YOLO protected root, plan, rewind, tree, steer,
  subagent dispatch, full-extension load, predicted-token t/s.

## Verification per phase
- `cargo fmt --manifest-path officina/tui/Cargo.toml`
- `cargo test --manifest-path officina/tui/Cargo.toml`
- `npx tsc --noEmit` (officina)
- `npx vitest --run extensions` (officina)
- Rebuild + atomic install to `~/.local/bin/officina`; restart TUI for live smoke.
