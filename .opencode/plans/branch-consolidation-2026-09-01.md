# Branch Consolidation — 2026-09-01

## Status: DONE (2026-09-01)
## Date: 2026-09-01

## Results

- Outer: `main` ff → `7a95dd6`, pushed to origin (`25ce5d9..7a95dd6`);
  `officina`, `vitriol`, `bench/pre-optimization`, `feat/fate-prefetch`,
  `feat/mtp-experiment` deleted. Worktree on `main`; `lull-kv` worktree intact.
- Inner: `vitriol-ku` → `main` (`590a4bb09`), pushed `* [new branch]` with
  tracking; archive `vitriol` synced to randozart (`441ccd871..a3ee3be00`);
  `vitriol-mellum2`, `vitriol-tq`, `vitriol-working`,
  `feat/fewer-experts-output-cache` deleted (SHAs below, reflog-recoverable).
  `master`, `upstream`, `turbo-tan` untouched.
- Final branch sets — outer: `main*`, `lull-kv` (worktree); inner:
  `main*`, `master`, `vitriol`, `lull-kv` (worktree).
- AGENTS.md updated: "`main` is the canonical daily-driver branch in BOTH
  repos (consolidated 2026-09-01)".

## Why

AGENTS.md claimed "`vitriol` is the canonical daily-driver branch, `master`
tracks upstream ggml-org". Reality had drifted:

- **Outer repo (Randozart/VITRIOL)**: no `vitriol` branch existed (one was
  created 2026-09-01 as an alias during the sidebar session). `main` existed
  but was 46 commits BEHIND `officina` (strict ancestor — all feature work
  lived only on `officina`). Local `main` was also ahead 9 of `origin/main`.
- **Inner repo (Randozart/llama.cpp fork)**: no `main` at all. The live line
  is `vitriol-ku` (checked out; "port VITRIOL hooks and TQ3/TurboQuant stack
  onto upstream" — 1574 commits atop ggml-org upstream). The old `vitriol`
  branch is the pre-port archive (143 commits not in vitriol-ku). AGENTS.md
  description stale.

Owner decision: consolidate both repos onto `main`, less confusing.

## Owner decisions (recorded)

1. Inner old-`vitriol` line: **keep both names** (rename vitriol-ku → main;
   old vitriol stays as the pre-port archive).
2. **Push both** remotes as part of the change.
3. Stale branches: **kill the dead ones** — verified per-branch by ancestry
   and content greps (see verdict table).

## Verdict table (evidence)

### Outer (Randozart/VITRIOL)

| Branch | Tip | Verdict | Evidence |
|---|---|---|---|
| `officina` | 7a95dd6 | delete after main ff | the driver line; becomes main |
| `vitriol` | 7a95dd6 | delete | 2026-09-01 alias, zero unique history |
| `main` | c3c3ae8 | fast-forward → 7a95dd6, then canonical | strict ancestor of officina (0/46) |
| `bench/pre-optimization` | 62fa31f | delete | fully merged into officina (`git branch --merged`) |
| `feat/fate-prefetch` | f836dd9 | delete | fully merged |
| `feat/mtp-experiment` | df76e9a | delete | fully merged |
| `lull-kv` | ef6918b | KEEP, untouched | active worktree `VITRIOL-lull` |

### Inner (Randozart/llama.cpp)

| Branch | Tip | Verdict | Evidence |
|---|---|---|---|
| `vitriol-ku` | 590a4bb09 | rename → `main`, push `-u origin main` | live line; tq3_0 in ggml-common.h/convert.cu/cpy-utils.cuh; prefetch in vitriol-cuda-integration.{cpp,h}; dlopen via ggml-backend-dl.cpp |
| `vitriol` | a3ee3be00 | KEEP (archive of pre-port line) | owner decision; push to randozart (ahead 1) |
| `master` | be92cad54 | KEEP | published fork branch tracking origin/master |
| `vitriol-mellum2` | 441ccd871 | delete | ancestor of kept `vitriol` |
| `vitriol-tq` | 284a4be40 | delete | ancestor of kept `vitriol` |
| `feat/fewer-experts-output-cache` | 68517ef9a | delete | ancestor of kept `vitriol` |
| `vitriol-working` | 866b9bca4 | delete (SHA recorded) | NOT in old vitriol, but features ported to vitriol-ku; tip recoverable via reflog |
| `lull-kv` | 46893d105 | KEEP, untouched | active worktree |
| `feat/fewer-experts-output-cache` feature note | — | reimport note | DISK_OFFLOAD mode + cross-layer/temporal prefetch live on old `vitriol` line if ever needed |

## Deleted-tip SHA table (recovery via reflog until GC)

```
outer: bench/pre-optimization     62fa31f
outer: feat/fate-prefetch         f836dd9
outer: feat/mtp-experiment        df76e9a
outer: officina                   7a95dd6 (== main after ff)
outer: vitriol                    7a95dd6 (alias)
inner: vitriol-working            866b9bca4
inner: vitriol-tq                 284a4be40
inner: vitriol-mellum2            441ccd871
inner: feat/fewer-experts-output-cache 68517ef9a
```

## Steps

Outer:
1. `git branch -f main 7a95dd6` (main not checked out anywhere)
2. `git checkout main` (worktree switches; tree identical to officina)
3. Delete `officina`, `vitriol`, `bench/pre-optimization`, `feat/fate-prefetch`, `feat/mtp-experiment`
4. `git push origin main`
5. AGENTS.md: "main is the canonical daily-driver branch (consolidated
   2026-09-01 from officina)" + inner-fork branch reality

Inner (llama.cpp):
1. `git branch -m vitriol-ku main`
2. `git push -u origin main` (origin = Randozart/llama.cpp; adds branch,
   does not touch master/upstream/turbo-tan)
3. `git push randozart vitriol` (sync archive line, ahead 1)
4. Delete `vitriol-mellum2`, `vitriol-tq`, `vitriol-working`, `feat/fewer-experts-output-cache`

Records:
5. This doc updated with results
6. AGENTS.md branch text fixed (outer repo)
7. Commit AGENTS.md + this doc on main

## Verification

- `git branch` both repos shows expected sets (no officina/vitriol-ku/stale)
- Outer `main` tip = 7a95dd6; `git status` clean except untracked `llama.cpp`
- Inner `main` tip = 590a4bb09, tracking origin/main
- Worktrees intact (`git worktree list`)
- AGENTS.md grep: no stale "vitriol branch is canonical" claims
- Zero code changes → no rebuild/test needed (build-patch canaries unaffected)
