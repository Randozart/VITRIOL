---
schema: "codex-work-plan/v1"
created_at: "2026-11-28T12:55:58+01:00"
timezone: "Europe/Amsterdam"
status: "in-progress"
baseline_commit: "7394826"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-26-223028-inkling-context-ceiling-z440.md", ".opencode/plans/inkling2-serve-profile-2026-11-23.md"]
---

# Inkling 2 Mini 200K upstream launcher

## Purpose

Give the user a working ~200K Inkling 2 Mini window again through the
existing `vitriol-server.service`, instead of the stale 200K profile that
now over-allocates CUDA0 under the `n_layer_all + 1` placement denominator.

## Approach and alternatives

Wire the service to the measured build `b12774` runtime and the argv that
already passed the 2026-11-26 capacity bracket: `-c 212992`, `-ts 1,3`,
`-ub 16`, `-fa on`, Q4_0 K/V KV, F16 weights. Alias stays `Lapis Occultus`
so Officina's local lane resolves it.

- Alternative A: current in-tree engine at 200K with Q4_0 KV. Unmeasured;
  the dense relative-bias path and `3394a8d` placement make it likely to
  OOM or crawl, and there is no GPU access in this sandbox to test it.
- Alternative B: host Q4_0 KV at 401,408. Larger window, but the 11-26
  plan measured a warm follow-up at 291 MiB free that was stopped.

## Scope and documentation

- `scripts/serve-inkling.sh`: launcher for the b12774 runtime.
- `scripts/vitriol`: accept `server.launcher = inkling`, add dispatch,
  dry-run and fingerprint plumbing next to the Kintsugi launchers.
- `scripts/lull_slot_persist.py`: let the autosave sidecar follow
  `~/.vitriol/checkpoints-inkling`.
- `profiles/inkling2-mini-200k-upstream/`: bundled profile stub.
- Runtime relocated from `/tmp/vitriol-inkling-upstream/runtime/llama-b12774`
  to `~/.local/share/vitriol/runtimes/llama-b12774` because `/tmp` is ephemeral.

## Verification

- `bash -n` on both shell files; `python3 -m py_compile` on the sidecar.
- `scripts/serve-inkling.sh --dry-run` prints the full argv.
- `vitriol config load inkling2-mini-200k-upstream` then
  `vitriol config show` and `vitriol serve --dry-run`.
- On the host: stop/start the unit, require `/health` ok, `/v1/models`
  shows `Lapis Occultus`, one idle slot, then a short completion.
- Freshness: the 212,992 bracket is historical. A successful load and a
  short reply are new evidence; a filled 200K prompt is not claimed.

## Invariants

1. Kintsugi profiles keep their current argv and stay runnable; the new
   launcher is additive.
2. The engine stays a single slot on port 8279 with alias `Lapis Occultus`;
   no provider, credential or routing-default change.
3. Separate slot directory `~/.vitriol/checkpoints-inkling`; the autosave
   sidecar must not write over `/scratch/kvtier*`.
4. No production restart is performed from this sandbox; host restart is
   explicit user action.

## Change history

- 2026-11-28T12:55:58+01:00 — Plan created before implementation.
