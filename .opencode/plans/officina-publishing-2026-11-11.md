# Officina publishing — standalone snapshots — 2026-11-11

Driver: **some people may want Officina without Vitriol.** The operator
frontend (Rust TUI + 51 pi extensions + caduceus gateway integration +
resilient routing) is useful against any llama-server, not just the
VITRIOL fork. Research verdict (this session): `officina/` is ~95%
self-contained — 44/51 extensions never touch engine endpoints, the
backend surface it uses (`/slots` save/restore, `--slot-save-path`) is
*upstream llama.cpp*, and VITRIOL-only features degrade/kill-switch
cleanly.

Decisions (operator, this session):
- **Vehicle**: publish snapshots — VITRIOL stays canonical, zero daily
  workflow change. `git subtree split -P officina` → tag → push to a
  public `officina` repo. Contributions return as patches to the
  monorepo.
- **Boundary**: publish the tree AS-IS — VITRIOL-aware extensions stay
  in; they degrade against plain llama-server. No split-brain.
- Licensing: officina is already `(Apache-2.0 OR MIT)` (package.json);
  deps MIT (pi-coding-agent 1.5.0, pi-tui 1.5.0, typebox); OmniRoute is
  an EXTERNAL vendor install (`~/.vitriol/vendor/omniroute`, MIT,
  documented in `officina/THIRD_PARTY_NOTICES.md`) — caduceus ships as
  an optional integration, not vendored code.
- Reverse coupling is nil: extensions never reference `scripts/vitriol`
  or `PROJECT_DIR` (verified). The `vitriol` CLI coupling is
  one-directional and stays in the monorepo.

## Stages

### Stage 0 — make the tree publishable (monorepo commit)
- `docs/officina-guide.md` → `officina/docs/officina-guide.md`
  (canonical with the product; `docs/` keeps a pointer page). `OFFICINA.md`
  (2026-08-31 standing TODO) stays in VITRIOL `docs/` — it is
  repo-history, not upstream documentation.
- `LICENSE` + `LICENSE-MIT` copies into `officina/` root (published tree
  must be self-licensing; `NOTICE` + `THIRD_PARTY_NOTICES.md` already
  live there).
- New `officina/README.md`: standalone quickstart (npm install → cargo
  build in tui/ → run binary), backend options table (any llama-server /
  caduceus gateway / full VITRIOL), degradation table (what changes
  without VITRIOL: decode gauges, governor `/kv-policy` + `/ctx/resize`,
  launcher-wired flags like `--slot-save-path`), kill-switch index,
  licensing.

### Stage 1 — release pipeline (monorepo commit)
`scripts/publish-officina.sh`:
1. Refuse on dirty tree / unpushed main.
2. Gates: `npm test` + `npx tsc --noEmit` + build-patch canaries +
   `cargo test` (tui).
3. **Secrets canary**: grep the subtree for `sk-…`/token patterns and
   refuse — subtree split carries the FULL path history, so nothing
   sensitive may ever have touched `officina/`.
4. `git subtree split -P officina -b publish/officina` → tag
   `officina-v<version from officina/package.json>` → push branch+tag
   to the `officina` remote.
5. Print published tag + split commit hash (fingerprint discipline).

### Stage 2 — one-time operator setup + first publish
- `gh repo create Randozart/officina --public --description …`
  (gh authenticated as Randozart ✓).
- `git remote add officina https://github.com/Randozart/officina.git`.
- Version bump in `officina/package.json` + `tui/Cargo.toml` (keep both
  in lockstep) → first publish → **acceptance test: fresh clone of the
  public repo builds** (npm install + cargo build).

### Stage 3 — docs + doctrine (monorepo commit)
- AGENTS.md: publishing doctrine (canonical = monorepo; public repo =
  tags only; never hand-edit the public repo; publish only through the
  script).
- Verification record appended here with tag + hashes.

## Invariants

- **I1**: monorepo behaviour unchanged — publishing adds files + one
  script; no build/test path changes. Pinned by the existing suites
  staying green without modification.
- **I2**: public tree is always an exact subtree of a stated monorepo
  commit (no hand-edits, no divergence). Pinned by tag↔split-commit
  provenance printed by the script.
- **I3**: no push without green gates + secrets canary. Script-enforced,
  not advisory.
- **I4**: published tree is self-licensing (LICENSE, LICENSE-MIT,
  NOTICE, THIRD_PARTY_NOTICES.md at `officina/` root).

## Explicitly deferred

- Env-var aliasing / brand divergence (only if the public product gets
  its own identity push).
- CI on the public repo; contribution intake flow (patches for now).
- `TRIS_`/`vitriol-*` identifier renames (heritage naming, user-facing
  config — churn without demand).
