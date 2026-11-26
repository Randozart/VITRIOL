---
schema: "codex-work-plan/v1"
created_at: "2026-11-26T16:35:33+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "f21b315da0dcfd97d7501d3cbfa29e0a3cdc6d6d"
supersedes: []
superseded_by: []
related: []
---

# readme-theme-screenshots

## Purpose

Show real VITRIOL and Officina terminal screens with different themes in the GitHub README.

## Approach and alternatives

Capture the current release binaries in isolated terminal sessions. Use a demo workspace and representative fixture telemetry, rasterize the terminal frames at a readable size, and link local PNG assets from the Markdown.

- Fixture sessions avoid exposing private prompts, keys, or machine paths. Generated mockups would not verify what the current binaries render.

## Scope and documentation

Root README and Officina README source, portable images in their respective asset trees. Preserve the author's opening verbatim. No standalone release.

## Verification

- Inspect all PNGs, verify example-only content, check relative links in both README contexts and run the repository's changed-scope evidence gates.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-26T16:35:33+01:00 — Plan created before implementation.

- 2026-11-26T16:39:59+01:00 — Captured four current release-binary terminal frames from isolated demo telemetry and Pi RPC history. Checked all images at full size, repaired terminal font fallback for alchemical and gauge glyphs, and confirmed no private paths or session data in the frames. All four RGB PNGs decode and resolve in the appropriate README contexts. The authored opening matches the baseline byte-for-byte. Source-bound evidence and terminal captures under /tmp/vitriol-readme-captures/.
