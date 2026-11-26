---
schema: "codex-work-plan/v1"
created_at: "2026-11-26T11:27:38+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "ecf3ae97d9edf3febff6061f281a2c86fa28802f"
supersedes: []
superseded_by: []
related: []
---

# provenance-inventory

## Purpose

Give techniques used by VITRIOL traceable credits in the README and the detailed provenance registry.

## Approach and alternatives

Restore a concise README credit section and index concrete techniques in docs/PROVENANCE.md. Review implementation, local upstream copies, notices and experiments. Preserve historical credits and mark conflicting or uncertain attribution.

- Reprinting the old bibliography in the README would make the introduction long again and carry forward unverified credit. Put the detailed technique inventory at the linked destination and keep the old bibliography accessible at its historical revision. Use source and release notices for actual reuse, and mark reported inspiration and evaluated ideas where the record is weaker.

## Scope and documentation

Root README (below the author-confirmed opening), docs/PROVENANCE.md, and the session anchor. Preserve license texts and existing notices. The baseline registry predates the engine import and the Rust TUI. The old README bibliography is in `17b04bd67486297678dc840afa72f33ac1e9288e`. Inventory scope covers techniques documented in that README plus identifiable later work across inference, runtime, and Officina. This is an attribution audit of available records, with unresolved conflicts visible.

## Verification

- Compare the protected opening byte-for-byte, check every new local evidence link and anchor, review changed historical claims and run changed-scope documentation verification.
- Evidence directory: `/tmp/vitriol-provenance-inventory/`.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-26T11:27:38+01:00 — Plan created before implementation.

- 2026-11-26T11:31:56+01:00 — Reviewed the 12 historical bibliography sections and the in-tree implementations, notices, experiment reports and local Kintsugi source. Diffed all four original Pi example imports against the cached 0.83.0 tarball and checked the pinned MIT license. The older little-coder intermediary attribution remains unverified and is retained. The README points to the detailed technique inventory and its first 14 lines matched the saved baseline. Logs and evidence are under /tmp/vitriol-provenance-inventory/.
