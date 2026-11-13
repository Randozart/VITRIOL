# Crash forensics + safe quit + shape-true probes + E2

- Date: 2026-11-13
- Status: stages 1–3 landed 2026-11-13 (crash black-box 399e505, safe quit, shape-true probe grid — synth loads in llama-bench, probe-load defaults to shape-mirrored synth). Stage 4 (E2 adaptive_k) IN PROGRESS at operator request — GPU seconded for training mid-run; ak0/ak1 at 13.1K measured (ak1 +2.9%, inconclusive), 42K arms pending GPU availability.
- Origin: operator reports "Officina crashed" with zero artifacts (no
  coredump, no panic text, no OOM) and restarts mid-turn that churn the
  session store. Plus: synth GGUF now loads in llama-bench — the probe
  grid can finally measure real shapes. Plus E2 (adaptive_k), the last
  unrun codex protocol.

## Stages

1. **Crash black-box (Rust TUI)**
   - `std::panic::set_hook` at startup: writes panic message, location,
     and the last ~100 diag-ring lines to
     `~/.vitriol/officina/last-crash.log` — best-effort `O_APPEND` on a
     detached thread with a 2 s hard timeout (a hook failure never masks
     or delays the original panic path; after writing, the default hook
     runs so behavior is unchanged).
   - The diag overlay's stderr ring becomes shared state
     (`Arc<Mutex<VecDeque<String>>>`, last ~100 lines) so the hook can
     copy it.
   - Clean-exit stamp: `~/.vitriol/officina/last-clean-exit` written on
     deliberate quit paths. On startup: if `last-crash.log` is newer than
     the stamp → amber notice `previous run crashed — /diag last` and a
     `/diag last` section showing the file.
2. **Safe quit (Rust TUI)**
   - `^esc` and `/quit` while `is_streaming || has_running_tool()`: first
     press arms (amber footer `working — press again to quit anyway (esc
     waits)`), second press within 5 s quits; any other key or the timeout
     disarms. Idle quits never prompt.
3. **Shape-true probe grid**
   - `probe load` default model becomes the synth-tiny GGUF, generated on
     demand (`probe synth --scale tiny`) when missing; `recommend` cites
     the newest shape-true grid labeled `measured on shape-mirrored
     synth`. Toy-model rows stay available via `--model`.
4. **E2 — adaptive_k A/B** (`scripts/depth_cert.py`)
   - Verify the knob in the engine (fingerprint carries `specak=`), add
     `--adaptive-k 0|1` passthrough.
   - Arms at 13.1K and 42K filled: spec mtp:n2, adaptive_k 0 vs 1,
     everything else identical to the E1 depth arms.
   - Pass/fail: ≥5% decode delta at 42K without acceptance collapse.
   - Result appended to the codex E-table (E2 resolved/kept).

## Invariants

- **I1 black-box is best-effort** — a failing/timeout write never masks
  the original panic; after the hook, the default panic hook still runs.
- **I2 quit guard never triggers when idle**; engine and unit paths are
  untouched.
- **I3 labels** — shape-mirrored synth numbers are labeled as such; never
  mixed with real-weight numbers in a conclusion.
- **I4 E2 arms differ only in adaptive_k** (fingerprint `specak=` proof);
  manual input still disarms fix-loop (unrelated).
- **I5 parity** — crash/quit changes never alter idle behavior; probe
  default change is additive (`--model` still overrides).

## Gates

- Stages 1–2: cargo test + fmt + rebuild/install (panic hook test spawns
  a panicking thread; quit-guard state-machine test).
- Stage 3: probe-load smoke on the synth output; docs.
- Stage 4: fingerprints per arm; results into docs/engine-settings.md.
