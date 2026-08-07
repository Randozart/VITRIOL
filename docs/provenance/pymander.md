# Pymander — provenance

**Kind**: `user-repo` — VITRIOL's own architecture, re-derived from this
project's design docs (`.opencode/plans/2026-08-06-pymander-ascensus.md`,
`docs/CURRENT_ARCHITECTURE.md`).

**Source**: no third-party source consulted. Pymander is a *static curated*
reference mind built on the project's own Hermetis memory machinery
(`libvitriol/hermetis/db.py:317 store_node`, versioned `git_rev` supersede).

## What was re-derived and how

- **Store**: reuses `hermetis.db` unchanged. Each domain is a Hermetis memory
  root `pymander/<domain>` → `~/.vitriol/pymander/<domain>/memory.db`
  (`db._get_db_path` already joins `MEMORY_DIR/<project_id>`). No fork.
- **Versioning**: node versioning comes from `db.store_node` (same git_rev →
  refresh in place; new rev → supersede old, never hard-discard).
- **Embedding**: `hermetis.embed.encode` (GPU GGUF embed server), best-effort;
  keyword scoring is the fallback when the server is down — same contract as
  Hermetis retrieval.
- **Ingest**: markdown `## heading` → atomic node, authored format defined in
  `docs/pymander/systems-programming.md`.

## License

Part of VITRIOL (GPL-2.0). Original content authored for this project; no
borrowed code or assets.