# Monorepo promotion — engine-monorepo → new VITRIOL main (2026-11-02)

## Goal

Promote the `engine-monorepo` branch (in-tree `engine/`, no submodule) to the
canonical `main` branch, without force-push and without losing remote work.

## Starting state

| item | value |
|---|---|
| current branch | `engine-monorepo` @ `6773f8b` |
| local `main` | `82bb43c` (ancestor of engine-monorepo) |
| `origin/main` (fresh fetch) | `dcba2f1` — "profiles: qwen38-ud + qwen38-ud-262k" |
| common ancestor | `98ca357` |
| branch vs local main | 9 ahead, 0 behind |
| branch vs origin/main | 9 ahead, 1 behind (`dcba2f1`) |
| tracked mods / staged | none |
| untracked | 1,106 files / 301 paths (in-source CMake artifacts) + profile + plan + stray file |

Engine import verification: `90416d1:engine` tree =
`058c67dc9245a540d91981d7db7fdd2d620672f4`, matching
`docs/ENGINE_PROVENANCE.md`. Current engine tree differs only by the intended
`tools/server/CMakeLists.txt +3` (vitriol-server OUTPUT_NAME).

## Owner decisions

1. **Remote reconcile:** fetch, then **merge** `origin/main` into
   `engine-monorepo` (no rebase).
2. **Rebis backups:** delete `engine/.rebis-bak-archive/`.
3. **Legacy worktrees:** keep `lull-kv` and `vllm-031`; leave
   `.git/modules/llama.cpp` (474 MB) in place.

## Phases

### Phase 0 — reconcile remote
- `git fetch origin`
- merge `origin/main` (`dcba2f1`, profiles only) into `engine-monorepo`
- resolve conflicts if any; re-verify engine provenance

### Phase 1 — clean working tree
- delete `engine/.rebis-bak-archive/`
- `git clean -nd engine` (preview) then `git clean -fd engine` (non-ignored only)
- delete empty `VITRIOL_EXPERT_COUNT=0`
- add in-source CMake artifact ignores to `engine/.gitignore`
- fix README clone/submodule instructions; optionally drop empty
  `engine/.gitmodules`
- commit `profiles/gemma4-12b-16k` (fix `created` to epoch), the AVX2 plan,
  and this report

### Phase 2 — promote
- `git checkout main && git merge --ff-only engine-monorepo`
- push `main` (fast-forward, no force); set upstream
- tag `engine-import-2026-10-23` at `90416d1`
- optionally delete `engine-monorepo` after push

### Phase 3 — keep legacy worktrees
- `lull-kv`, `vllm-031`, `.git/modules` untouched

### Phase 4 — smoke
- rebuild from promoted tree when GPU is free (currently owner gaming)
- start at 262K, verify health/fingerprint/slot restore

## Rollback

- Local: `engine-monorepo` remains intact until `main` is pushed.
- Remote: push is a fast-forward from `dcba2f1`; no force. If needed, the
  pre-promotion remote head is `dcba2f1`.
- Worktree branches are not modified by promotion.

## Risks

- Merge conflict if remote `dcba2f1` touches the same profile/docs paths.
- `git clean -fd engine` is scoped to `engine/` and excludes ignored build
  shims/dirs; dry-run first.
- Do not remove `.git/modules` while legacy worktrees exist.
