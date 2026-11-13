# Release + docs sweep (no GPU needed)

- Date: 2026-11-13
- Status: IMPLEMENTED 2026-11-13.
  - Stage 1: docs sweep — guide already current from the rename pass.
  - Stage 2: v0.1.2 published, then the fresh-clone acceptance caught a
    REAL bug (vitriolum parity test read a legacy monorepo-only path) —
    fixed, republished as **v0.1.3** (f3647e3..387b99c lineage; palette
    watermark drift #1c2634→#2a3a52 fixed; canary allowlist extended for
    retired test fixtures; release gate gained testTimeout headroom for
    loaded hosts).
  - Fresh-clone acceptance v0.1.3: version ✓, 57 extensions, uniqueness
    + parity tests green, no legacy-path references.
  - Stage 3 (backup restore drill): PENDING — next session.
- Context: ~30 commits of operator-visible behavior since
  `officina-v0.1.1` (renames, /fx, watchdog card, voice suggestions,
  longjob, crash black-box, STEER, evidence batch). The public snapshot
  is stale. GPU is busy training — everything here is CPU-only.

## Stages

1. **Docs sweep** — rename pass left stale command names in the docs;
   update every reference to the new family names (`route-*`, `model-*`,
   `provider-*`, `import-*`, `/budget`), fix the
   `command-renames-fx-2026-11-13.md` plan status, and note the E1
   verdict in the drift table of the settings-codex plan.
2. **Snapshot release** — bump `officina/package.json` +
   `officina/tui/Cargo.toml` in lockstep (0.1.1 → 0.1.2), commit, run
   `scripts/publish-officina.sh` (runs its own gates + secrets canary +
   tracked-tree snapshot + tag `officina-v0.1.2`), verify the public
   repo and fresh-clone acceptance.
3. **Backup restore drill** (follow-up, after release) — restore the
   newest nightly session backup to a scratch store, confirm sessions
   enumerate/load from the restored store, leave nothing broken.

## Invariants

- I1 release goes ONLY through `scripts/publish-officina.sh` — never
  hand-edit the public repo.
- I2 version lockstep: package.json and tui/Cargo.toml carry the same
  version.
- I3 docs reference only commands that exist post-rename.
