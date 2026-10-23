# VITRIOL architecture

VITRIOL is not a llama.cpp fork with scripts around it - it is a stack.
Since the 2026-10-23 monorepo import the engine lives in-tree; the layers
and their owners are:

```
+--------------------------------------------------------------+
| harness (clients)                                             |
|   officina/ (JS + Rust TUI), alka/, plugins/, libhermes/      |
+--------------------------------------------------------------+
| control plane                                                 |
|   scripts/vitriol (launcher: config, fingerprint, units)      |
|   vitriol-governor/ (Rust: VRAM policy ladder, NVML poll)     |
|   systemd units (user + system), watchdog, sidecars           |
+--------------------------------------------------------------+
| engine policy (Rust, coarse-grained C ABI)                    |
|   vitriol-cache/   (radix prefix index + ARC; vc_* ABI)       |
|   vitriol-stream/  (tiered streaming; vs_* ABI)               |
|   (policy lives OUTSIDE the engine on purpose)                |
+--------------------------------------------------------------+
| engine (C++/CUDA, in-tree fork)                               |
|   engine/  (llama.cpp fork @ 30b128504, upstream FROZEN)      |
|   binary: engine/build/bin/vitriol-server                     |
|   (docs/ENGINE_PROVENANCE.md)                                 |
+--------------------------------------------------------------+
| calibration/offline                                           |
|   libvitriol/ (Rust), libvitriol/*.py, profiles/              |
+--------------------------------------------------------------+
```

## Ownership rules

1. **Tensor/KV execution stays in `engine/`.** Anything that must run per
   token or per kernel belongs in the fork (C++/CUDA).
2. **Policy lives outside the engine.** Cache lookup/eviction decisions,
   VRAM escalation policy, routing - Rust crates behind coarse-grained C
   ABIs (`vc_*`, `vs_*`). Never per-token FFI; the boundary is per
   request/lookup/epoch.
3. **The launcher is the choke point.** Units invoke `scripts/vitriol`,
   never the engine binary directly. Flag provenance, fingerprinting, and
   process-name transitions are handled there.
4. **Upstream is frozen.** See `docs/ENGINE_PROVENANCE.md` for the import
   record and the manual-port policy.

## Process-name contract

The engine process is `vitriol-server` (comm name). During the transition
window (until declared closed in this file's changelog) every detector
accepts both `vitriol-server` and `llama-server`:

- `vitriol-governor/src/engine.rs` (`pids_by_comm`)
- `scripts/lull_slot_persist.py` (`server_pid`, `PROTECTED`)
- `scripts/vitriol-watchdog.sh`, `scripts/vitriol-oom-hardening.sh`
- `scripts/vitriol` (pidfile: `vitriol-server.pid`, legacy fallback
  `llama-server.pid`)

A `llama.cpp -> engine` symlink at the repo root keeps old absolute paths
(and legacy path-based scripts) alive during the same window.

## Changelog

- 2026-10-23: monorepo import; engine renamed to `vitriol-server`;
  upstream sync frozen; transition window opened.
