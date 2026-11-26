---
schema: "codex-work-plan/v1"
created_at: "2026-11-26T11:06:24+01:00"
timezone: "Europe/Amsterdam"
status: "active"
baseline_commit: "f20d1679348ba20c002df68015b5d5d101baa6ad"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-26-101857-kintsugi-replay-z440.md", ".opencode/plans/2026-11-26-104921-z440-kintsugi-inkling-experiments.md"]
---

# Kintsugi daily driver on GPU1

## Purpose

Serve the measured Kintsugi configuration as Lapis Occultus on physical GPU1 through the existing system service.

## Approach and alternatives

Use additive systemd drop-ins around the qualified foreground launcher. Give Kintsugi and the autosave sidecar a dedicated state directory. Retain the existing settings and service unit for rollback. Keep the experimental checkpoint and GPU0 draft changes outside the deployment.

- The systemd overrides track the qualified foreground launcher. A generic config load would need additional launcher support for full draft cache retention and the exact placement flags.

## Scope and documentation

The dedicated launcher exposes the existing slot and metrics APIs. Two Kintsugi-only drop-ins connect it to the existing system service and sidecar. The current user profile is rollback state.

## Verification

- Validate merged units, real CUDA GPU1 placement, health, model alias, slots and metrics, and a slot save/restore. Verify the effective launch fingerprint, settings digest and service state. Activate after administrator authentication.

## Results and evidence

The merged unit files passed static verification. The GPU1 smoke run exercised chat and a save, erase, restore round trip over the same API. The local sudo probe requires a password. Runtime activation is pending workstation-owner authentication.

## Change history

- 2026-11-26T11:06:24+01:00 — Plan created before implementation.

Baseline at f20d167: system engine inactive and enabled. Autosave and governor inactive and disabled. The primary service still names the previous detached launch and pulls in autosave. GPU1 (PCI 03:00.0) had 23 MiB in use and GPU0 had 925 MiB. Noninteractive sudo reports that a password is required.

Related: [Kintsugi launch qualification](2026-11-26-101857-kintsugi-replay-z440.md) and [Z440 experiments](2026-11-26-104921-z440-kintsugi-inkling-experiments.md).

## Invariants

1. The system unit tracks the foreground server process with its existing user, restart policy, and OOM adjustment. The combined unit passes systemd validation.
2. Physical GPU1 serves both target and draft. Record the launch fingerprint, PCI bus and actual CUDA process placement.
3. `Lapis Occultus` remains available on port 8279. Probe chat, slots and metrics on a temporary port before deployment.
4. Autosave and the server share a Kintsugi-specific directory. Exercise save and restore and compare the original configuration digest.
5. Dropping the two Kintsugi overrides restores the previous launch and state directory. Keep the old profile as the rollback configuration.

The isolated cache-retention patch adds checkpoint memory and remains a prototype. A GPU0-local drafter was slower on the matched host workload. Split persistent cache placement would need engine work and measurements, so this activation uses the already tested GPU1 plus host KV tier. Preserve independent use of GPU0.

## Activation

Run from the repository root as the workstation owner. The two destination files are new overrides. They update the existing enabled system unit and pair the sidecar with the same systemd-managed state directory.

```bash
sudo install -m 0644 scripts/systemd/vitriol-server.service.d/kintsugi-gpu1.conf \
  /etc/systemd/system/vitriol-server.service.d/kintsugi-gpu1.conf &&
sudo install -m 0644 scripts/systemd/vitriol-autosave.service.d/kintsugi-gpu1.conf \
  /etc/systemd/system/vitriol-autosave.service.d/kintsugi-gpu1.conf &&
sudo systemctl daemon-reload &&
sudo systemctl restart vitriol-server.service
```

The user configuration continues to be the fallback for the generic VITRIOL launcher. This service override takes its flags from `scripts/serve-kintsugi.sh`, so Officina's generic settings editor can still show the previous profile. Health, model identity and slots are checked through the same loopback API. The existing `OOMScoreAdjust=-500` and restart policy are inherited. Kintsugi logs and its fingerprint are available in `journalctl -u vitriol-server.service`.

After startup, verify `/health`, `/v1/models`, `/slots`, and `/metrics` on port 8279. A chat probe should return content. Match `systemctl show -p MainPID --value vitriol-server.service` against `nvidia-smi --query-compute-apps=pid,gpu_uuid,used_gpu_memory --format=csv` and match that GPU UUID to PCI 03:00.0. Check autosave's journal for the dedicated `/var/lib/vitriol-kintsugi` path.

## Rollback

Stop the autosave and engine units, remove the two `kintsugi-gpu1.conf` drop-ins above, reload systemd and start the engine again. The original launcher, configuration and previous slot mirror remain available. Preserve `/var/lib/vitriol-kintsugi` for a later switch.

The launcher and these overrides are verified independently of administrator authentication. Local evidence is under `/tmp/vitriol-kintsugi-daily`.

## 2026-11-26T11:08:10+01:00 pre-activation result

The isolated launcher reported the `Lapis Occultus` alias, answered `Paris`, and exposed slots and metrics. The test saved slot 0, erased it, and restored the saved token count from the dedicated temporary directory. Nvidia's process listing matched the launched PID to physical GPU1 only. The previous user configuration digest verified. The unit pair passed `systemd-analyze verify` with the existing base units and drop-ins in a private test directory. Full fixtures and output are under `/tmp/vitriol-kintsugi-daily`.

- 2026-11-26T11:08:50+01:00 — GPU1 runtime and checkpoint smoke passed. The installation and live-unit check await sudo authentication.

## 2026-11-26T12:40:00+01:00 config selection

The [selectable profile](2026-11-26-123509-selectable-kintsugi-gpu1.md)
now uses the existing service entry point. Use that route to switch
profiles on this workstation. The direct-launch overrides above document
the earlier approach and would bypass config selection.
