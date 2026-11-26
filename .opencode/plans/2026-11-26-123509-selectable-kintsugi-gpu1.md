---
schema: "codex-work-plan/v1"
created_at: "2026-11-26T12:35:09+01:00"
timezone: "Europe/Amsterdam"
status: "active"
baseline_commit: "839173ce5924bc1af67212365801876a316e8218"
supersedes: []
superseded_by: []
related: []
---

# selectable-kintsugi-gpu1

## Purpose

Select qualified Kintsugi GPU1 through config and activate as the workstation daily driver.

## Approach and alternatives

Route a bundled Kintsugi profile from the existing service entry point to the qualified launcher, with separate checkpoints. Retain the existing service and ordinary launcher paths.

- Keep the qualified argument vector in its existing launcher. Duplicating the arguments as generic settings would add launch knobs and invite drift. Use the installed forking service entry point so profile switching needs no new root-owned service files. Resolve the sidecar's Kintsugi directory from the running server, so loading a profile ahead of a restart cannot redirect saves for the previous process.

## Scope and documentation

Add the bundled `kintsugi-gpu1` config, Bash dispatch and dry run, and sidecar checkpoint selection. Keep the former config as a named rollback profile. Existing service files and startup type remain in use. Update the config reference and activation evidence. Retain the validated 65,536-token window, n7 DFlash, GPU1 target and draft, and snapshot mode. Use a private per-user checkpoint directory.

## Invariants

1. Configs without the new launcher selector use the existing path and produce the same fingerprint. Verify against an identical legacy config on the baseline and edited script.
2. The Kintsugi profile resolves the same GPU and model argv as the qualified launcher, except for the isolated checkpoint directory. Verify by comparing dry-run fingerprints with matching inputs and checking the live daemon.
3. A running generation uses its own checkpoint path even when the selected config changes. Verify the sidecar against both profiles and synthetic and live launch arguments.
4. Loading a profile persists the selection without starting a server. A restart uses the selected profile. Verify through the existing CLI and installed unit when authorized.

## Verification

- Compare fingerprints and full native argv, test profile list/load/save/dry-run and legacy parity, validate sidecar isolation and restart readiness, and verify live GPU1 chat when authorized.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-26T12:35:09+01:00 — Plan created before implementation.

- 2026-11-26T12:40:52+01:00 — The baseline and edited legacy launch fingerprints matched. The bundled profile round-tripped through load, save, reset and reload with identical Kintsugi argv. The workstation now selects kintsugi-gpu1. The prior config is saved as pre-kintsugi-gpu1-20261126-123509 and the before/after flag diff is in /tmp/vitriol-selectable-kintsugi/active-profile-fingerprint.diff. The existing unit's detached entry point passed health, model alias and path checks, chat, slots, metrics and isolated save on physical GPU1 using a temporary port. The temporary process was stopped. Systemd denied the production restart and sudo requires password authentication. Final source-bound evidence is recorded under /tmp/vitriol-selectable-kintsugi/verification/.
