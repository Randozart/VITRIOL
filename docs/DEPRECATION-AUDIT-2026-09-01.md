# Deprecation audit — VITRIOL-side systems vs Officina — 2026-09-01

**Question (owner):** with Officina as the primary endpoint, which
VITRIOL-side systems no longer have a right to exist? Several were built
to serve *other* endpoints (hermes, Crush, little-coder) and now clutter.

**Rule applied:** a system earns its place by who consumes it *today*.
"Was built for the roadmap" is not a consumer. Engine-ops surfaces are
consumed by the operator, not the agent — different bar, judged on
duplication.

## Dispositions

### RETIRE — built for other endpoints, unconsumed by Officina

| System | Built for | Evidence | Action |
|---|---|---|---|
| `hermes-gateway.service` | hermes messaging-platform gateway (Telegram etc.) | runs hermes-agent venv directly; Officina never talks to it | Owner call: retire if messaging integrations are unused; else it stays as the ONE sanctioned hermes runtime until a messaging replacement exists |
| `mongod-vitriol.service` + `vitriol-rag.service` + **pymander** (`vitriol pymander *`, scripts/vitriol_rag.py :8282) | hermes-era semantic recall; RAG experiments | no Officina extension consumes :8282 or the mongo mirror; officina memory is markdown + session scan | RETIRE (stop units, `vitriol pymander` → deprecation notice, keep data dirs one release). Officina memory + memory_search is the successor |
| `tris chat` | hermes `-z` | already retired (SS2a) | DONE |
| hermes-bridge ext | hermes state.db reads | already removed (SS2a) | DONE |

### CONSOLIDATE — duplicated by Officina, shrink to one source

| System | Duplication | Action |
|---|---|---|
| `tris` CLI vs `vitriol` | two front doors: up/down/smoke/lanes exist in both worlds; `tris code` delegates to `vitriol officina` | Fold the unique tris value (validate, lanes, budget, ledger-ingest, perms-sync, selfcheck) into `vitriol` subcommands; then retire `tris` + `~/.local/bin/tris`. Officina/cli stays as the implementation library |
| decode/`slots` widget vs `vitriol tui` gauges | same numbers, two renderers | acceptable short-term; after layout fork, consider a `--minimal` vitriol-tui or moving GPU/service rows into Officina's panel for engine-ops-lite |
| autosave policy | `vitriol-autosave.service` writes on ITS schedule; officina vitriol-checkpoint writes per turn-key (F7 dual-writer) | Move policy Officina-side (when/what), keep serialization engine-side (boundary rule, recorded SS4/2026-08-31) |

### KEEP — genuinely engine-side (boundary rule)

`vitriol-server.service`, serve/stop/run/bench/calibrate/config/setup,
profiles, cert suite, `/slots` `/metrics` `/health` `/props`, checkpoint
endpoints, fingerprint emission, GPU lane scheduling, `vitriol-tui` (as
operator console), `install`/`uninstall`, oom-shield, sidecar/watchdog.

### KEEP but RENAME-in-place (served Crush, now serve Officina)

- `profiles/mellum2-crush-small` → conceptually "officina small lane"
  (rename the profile dir only if cheap; the coupling key `crush-small`
  in config `lanes:` may be re-labelled `small` with a migration note).

## Sequencing


### Owner clarification (2026-09-01): the two gauges serve different masters

`vitriol tui` is the BACKGROUND ops console (machine state while you work
elsewhere) - it stays. Officina's gauges exist so you never swap to it
mid-session. Different moments, not duplication; the earlier
"consolidate tui gauges" note is withdrawn.


1. pymander + rag + mongo: launcher notice DONE 2026-09-01. OWNER still
   stops the units: systemctl --user disable --now vitriol-rag.service
   mongod-vitriol.service (data dirs kept one release).
2. Owner decision on hermes-gateway.service (messaging: keep or kill).
3. DONE 2026-09-01: validate/lanes/budget/ledger-ingest/perms-sync/status
   exec the officina CLI; tris symlink removed.
4. autosave policy hand-off with the F7 checkpoint consolidation.
5. Re-run selfcheck + `vitriol officina` smoke after each removal.

### TUI cleanup (2026-09-01, disable-not-delete)

vitriol-tui tab bar trimmed 10 → 8: HERMETIS and REBIS tabs disabled
(hermes-era memory server + gateway routes — both replaced by the SS2
fold-in). Variants, renderers and pollers retained for one-keystroke
restore (re-add to Tab::ALL); enum carries a dated allow(dead_code) note.
Officina tab was already disabled (model-surgery REPL not ready).
Subsystems tab stays — it still reports the services' liveness honestly.
Guide tab's Pymander corpus reference is next (SS3-adjacent).
