# Officina exec & infrastructure batch

- Date: 2026-11-12 15:40
- Status: in progress (stages 1–7)
- Origin: post-Codex-batch gap analysis (8 items selected); GitHub via
  gh-CLI tools; fix-loop auto-commit default-off, enabled for this operator.

## Stages

1. **Session backups** — `scripts/officina-backup.sh`. `--install` writes
   user-scope `~/.config/systemd/user/officina-backup.{service,timer}`
   (daily 04:00, Persistent=true), daemon-reload + enable --now. Run mode:
   flock, `rsync -a ~/.pi/agent/sessions/ →
   ~/.local/share/officina-backups/sessions/<YYYY-MM-DD>/`, prune >14 days.
   `--dry-run` supported. Source is read-only (I2).
2. **Desktop notifications** — `notify-done` ext: `notify-send -a officina`
   when a turn ran longer than `OFFICINA_DESKTOP_NOTIFY_MIN_S` (default 20).
   Gated by `OFFICINA_NO_DESKTOP_NOTIFY=1` and DISPLAY/WAYLAND_DISPLAY (I3).
3. **`officina exec`** — `officina/scripts/exec.mjs` wired as
   `vitriol officina exec "task"`. Flags: `--lane auto|local|nimbus`,
   `--tools csv`, `--max-minutes` (default 30, hard kill), `--json`
   (pi --mode json passthrough), `--detach` (setsid + log/pidfile under
   `~/.local/share/officina/exec/<ts>-<slug>/`, notify-send + `.done`
   marker on completion), `--list`. Runs `pi --print` with the SAME
   extension auto-discovery as the TUI (lanes, prefs, permissions guard —
   headless asks fail closed). **Always a fresh session** (I1): exec never
   passes `--session`, so it can never resume/patch/quarantine a store file.
4. **`/sessions` cross-project modal** (Rust) — scans every store under
   `~/.pi/agent/sessions/`, decodes the store slug to a project path,
   type-to-filter, columns project/title/age/msgs. Current-cwd store:
   Enter resumes through the existing peer fence. Foreign stores:
   read-only — Enter prints `cd <proj> && officina` + the session path.
   Never mutates foreign files (I4).
5. **`/cost` modal** (Rust) — reads
   `~/.vitriol/officina/route-ledger-YYYYMMDD.jsonl` (today): per
   provider/model rows (requests, tokens in/out, failures) + budget bars
   from `caduceus-state.json`. Euro shown only where the ledger carries it
   (I5: no invented pricing).
6. **GitHub tools** — `github-tools` ext: `pi.registerTool` wrappers around
   the authenticated `gh` CLI — pr list/view/diff, issue list/view, CI runs
   status. Repo from `git remote get-url origin` in ctx.cwd. Read-only v1
   (I6); timeouts + output caps; injectable runner for tests.
7. **`/fix-loop [N]`** — `fix-loop` ext: verify-full → send a fix turn →
   re-verify, up to N rounds (default 3, `--max N` capped at 10). Imports
   verify's PURE functions (`runVerify`, `detectGates`, baseline types) —
   jiti isolation forbids module state, files are fine. Day-capped via
   `.officina/fixloop-state.json`; single-flight (a second invocation is
   refused while one is armed); ANY manual input disarms; kill switch
   `OFFICINA_NO_FIX_LOOP=1`. After a green re-verify it stops and reports;
   when `OFFICINA_FIXLOOP_AUTOCOMMIT=1` (default OFF; enabled in this
   operator's `~/.vitriol/officina/env`) it stages + commits
   `fix-loop: gates green (N rounds)` when anything is staged (I7).

## Files

| File | Stage |
|---|---|
| `scripts/officina-backup.sh` | 1 |
| `officina/.pi/extensions/notify-done/index.ts` + tests | 2 |
| `officina/scripts/exec.mjs`, `scripts/vitriol` wiring | 3 |
| `officina/tui/src/rpc/sessions.rs`, `tui/{state,layout,mod}.rs` | 4 |
| `officina/tui/src/tui/cost.rs` + modal wiring | 5 |
| `officina/.pi/extensions/github-tools/` | 6 |
| `officina/.pi/extensions/fix-loop/` | 7 |
| `officina/docs/officina-guide.md` | rolling |

## Invariants

- **I1 exec isolation** — exec spawns with a fresh session, never
  `--session`; it cannot resume, patch, or quarantine a store file. The
  permissions guard's headless fail-closed is preserved (no new bypass).
- **I2 backup safety** — the script never writes inside the session store,
  holds flock, retains exactly 14 daily snapshots, and restores are manual
  (documented in the script header).
- **I3 notify gating** — no notification without DISPLAY/WAYLAND_DISPLAY
  and the absence of the kill switch; threshold default 20 s.
- **I4 foreign stores read-only** — `/sessions` renders foreign sessions
  and prints a launch hint; it never switches into, patches, quarantines,
  or deletes a foreign store's file.
- **I5 honest cost** — tokens/requests/failures come from the route
  ledger; euro amounts only from fields the ledger itself carries.
- **I6 read-only GitHub** — v1 tools list/view only; no mutations (no PR
  create/comment) even if asked.
- **I7 fix-loop discipline** — explicit invocation only; N ≤ 10 hard cap;
  day-capped; single-flight; manual input disarms immediately; auto-commit
  defaults OFF and only fires after a GREEN re-verify with something
  staged; commit message names the loop.
- **I8 zero-env parity** — with no new env set, today's behavior is
  unchanged (notify off under threshold, autocommit off, exec unused).

## Gates

- Stage 1: `bash -n` + `--dry-run` + idempotent `--install`.
- Stages 2/6/7: `npm test` + `npm run typecheck`.
- Stages 3: `node --check` + a smoke exec (`--list`).
- Stages 4/5: `cargo test` + `cargo fmt --check` + `cargo build --release`
  + install to `~/.local/bin/officina`.
