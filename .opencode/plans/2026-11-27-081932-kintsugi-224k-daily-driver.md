---
schema: "codex-work-plan/v1"
created_at: "2026-11-27T08:19:32+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "93add2a1fe0255aea61b3dbbe9b0f1d2d5a29e47"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-26-210332-kintsugi-rust-control-plane-session.md"]
---

# kintsugi-224k-daily-driver

## Purpose

Activate the qualified 229,376-token Kintsugi reference build on GPU1 for the workstation and make Lapis the Officina default.

## Approach and alternatives

Add a bundled reference profile using the existing forking systemd entry point, preserve the existing 65K profile, and give the reference its own autosave directory. Reuse the installed reference runtime and exact qualified inference flags. Confirm Officina default selection, local routing and slot capacity at the normal endpoints.

- Keep the distribution's runtime under the existing systemd entry point.
  Building a second engine or duplicating system units would add another
  runtime path and break the existing profile selector. Preserve independent
  state so switching profiles never feeds one engine the other's snapshots.

## Scope and documentation

The bundled `kintsugi-reference-gpu1` profile uses the existing launcher,
system scope and the operator's local reference runtime. It adds a private
reference checkpoint directory and is documented in the config reference.
The user preference changes only the default provider and model to
`officina/lapis`. The previous VITRIOL profile and Officina preference have
local backups in the evidence directory.

## Verification

- Diff fingerprints against the old production profile and the tested 224K reference run. Exercise profile round-trip, legacy parity, full launcher/sidecar startup, GPU1 placement, chat, tool or Lapis routing, sidecar save and restart reuse. Run changed-scope verification with explicit provenance and record whether interactive sudo is required for live activation.

## Invariants

1. The 65K DFlash2 and ordinary launchers retain their prior behavior. Pin the former with a full fingerprint diff and isolate checkpoint formats per backend.
2. The daily profile launches tested MTP and 112,640 GPU KV cells with a 229,376-token slot on physical GPU1. The existing service keeps the loopback port and Lapis alias.
3. Autosave follows the live PID and uses the directory named in the selected engine's argv. A systemd-compatible isolated restart must reuse its saved state. The production probe separately checks the periodic sidecar.
4. The Officina preference resolves to Lapis. Verify the physical lane with a local request and a routing trace before reporting it active.

## Results and evidence

### Launcher, routing and persistence

The 65K fingerprint matches the baseline byte for byte after normalizing
the scratch script path. The 224K profile's argv and performance-bearing
environment match the qualified Rust experiment, with the production alias,
port and state path substituted. The prior 65K profile is still in the
local profile catalog. Evidence includes `fingerprint-review.log` and
`production-fingerprint.diff`.

An isolated forking unit exposed a 229,376-token Lapis slot. Officina's
Pi RPC stack picked the saved virtual model and logged a local completed
request. The sidecar saved 4,767 tokens into its dedicated path. A new
engine PID restored all 4,767 in 246 ms and reused them on the next reply.
See `isolated-lapis.log`, `isolated-persistence.log` and
`sidecar-dir-regression.log`.

Systemd denied the noninteractive start. After operator authorization in
Konsole, both production units became active. The service owns the
reference executable on physical GPU1, with one idle 229,376-token slot
and the Lapis alias on port 8279. The production smoke selected a fresh
local slot and completed. The new Officina preference saved as
`officina/lapis`. See `production-runtime.log`, `production-lapis.log`
and `production-officina-default.log`.

The first production autosave after the smoke persisted 1,512 tokens
in the dedicated reference directory on its normal five-minute cadence.
The existing engine service is enabled at boot and starts its autosave
dependency. See `production-autosave.log`.

The scratch evidence directory is `/tmp/vitriol-kintsugi-daily-224k/`.
Shell parsing, ShellCheck, Ruff, and a scoped Praetor invocation passed.
Bandit reports two pre-existing B310 findings at the sidecar's
constant-loopback URL operations. An identical base-versus-candidate
check found zero added signatures. See `bandit-existing.diff`.
The changed-scope verification uses base `93add2a` and retains the two
existing security findings.

## Change history

- 2026-11-27T08:19:32+01:00 — Plan created before implementation.
- 2026-11-27T08:32+01:00 — Production activated by the operator. Verified live GPU placement, local Lapis and timed autosave after testing restart reuse in the isolated unit.
