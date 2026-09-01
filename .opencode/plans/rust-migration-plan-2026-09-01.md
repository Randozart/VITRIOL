# Officina Rust Migration Plan

## Status: Phase 2 complete — honest results recorded
## Date: 2026-09-01

## Measured Reality (2026-09-01 bench, Node 22, release LTO build)

| Path | JS | Native | Speedup |
|---|---|---|---|
| visibleWidth (single line, per-call) | 397ms/200k | 324ms/200k | **1.2x** |
| merge_split_rows (50 rows, per-render) | 467ms/2k | 450ms/2k | **1.0x** |

**Verdict: V8 is NOT the bottleneck for this TUI's string work.** The NAPI
boundary (string conversion JS→Rust→JS) costs as much as the JS regex work it
replaces. The premise "JS/TS are terribly inefficient" does not hold for this
workload — modern V8 optimizes exactly this kind of code well.

**The real waste was render COUNT, not per-render cost:** the sidebar fully
re-rendered (12+ sections → setSidebar → rebuild 10+ Text components →
invalidate split) every 700ms engine poll even when idle. Fixed with a
content-hash guard in session-panel (`lastSidebarKey`) — idle renders drop
from ~86/min to ~0. That single JS change beats the entire native addon.

## What the native addon is FOR (kept, not scrapped)

`officina/native/` stays as FFI infrastructure with parity tests:
1. **Foundation for genuinely heavy Rust work** — SPQL/model-surgery, GGUF
   census calls, anything operating on MB-scale binary data where per-call
   conversion cost is amortized (the 1.0x merge bench is the FFI floor for
   KB-scale strings; MB-scale payloads flip the ratio hard).
2. **Engine poller (Phase 3, next)** — background Rust thread with HTTP +
   ThreadsafeFunction push eliminates the 700ms JS timer/GC interaction.
   Expected win: latency consistency, not throughput.
3. **Single source of truth for hot-path semantics** — parity-tested against
   the JS fallbacks (6 vitest cases, byte-identical); JS fallbacks are the
   correctness contract, native is opportunistic.

## Principle
Move what Rust can improve to Rust. Keep what Pi needs in TS. No rewrite for
rewrite's sake — every Rust boundary must cross a performance or correctness
boundary that TS cannot.

## Architecture: Rust Native Addon (NAPI-RS)

Officina runs inside Node.js (pi-coding-agent). We cannot replace the runtime.
We CAN replace hot paths with native addons via NAPI-RS — zero-overhead FFI,
no child process, no serialization.

```
officina/
  native/                    ← NEW: Rust NAPI addon
    Cargo.toml
    src/
      lib.rs                 ← NAPI entry, exports functions to JS
      ansi.rs                ← ANSI strip + width + cut (replaces officinaStripZeroWidth etc.)
      engine_poller.rs       ← Background thread: HTTP poll /metrics + /slots, push to JS
      braille.rs             ← Braille gauge rendering (pure math, no I/O)
      debounce.rs            ← Smart sidebar update deduplication
  package.json               ← Add napi-rs build dependency
  .pi/extensions/            ← TS extensions call native.* functions
```

## Phase 1: Immediate Fixes (TS) — DO FIRST

### Fix 1: Duplicate mode badge
Session-panel P15 "mode" duplicates agent-mode P18. Remove P15.

### Fix 2: Narrow terminal fallback
`__officinaSidebarVisible` isn't set until first OfficinaSplit render.
Push initial visibility from session_start.

## Phase 2: Rust NAPI Addon — `officina-native`

### 2a. ANSI Width Engine (highest impact)
**Current:** `officinaStripZeroWidth`, `officinaVisibleWidth`, `officinaCut` in
build-patch.mjs — runs on EVERY line of EVERY render (60+ times/sec during generation).

**Rust replacement:** `native/ansi.rs` — SIMD-accelerated ANSI strip + Unicode
width. Exports: `stripAnsi(s) -> string`, `visibleWidth(s) -> u32`,
`cut(line, width) -> string`, `wrapAnsi(line, width) -> Vec<string>`.

**Performance gain:** ~10-50x for width calculations on typical terminal lines.
The JS version allocates regex matches per escape sequence; Rust walks bytes.

### 2b. Braille Gauge Rendering
**Current:** `renderGauge()` in `braille.ts` — braille character math, called
for every gauge (ctx, engine, ingestion = 3-6 gauges per sidebar render).

**Rust replacement:** `native/braille.rs` — `renderGauge(ramp, ratio, width) -> String`.

### 2c. Engine Telemetry Poller
**Current:** `_shared/engine.ts` — 700ms JS setInterval, two HTTP requests per
tick, delta computation, busy-slot calculation. Runs even when idle.

**Rust replacement:** `native/engine_poller.rs` — background thread, same HTTP
polling, but pushes deltas to JS via NAPI callback. Eliminates JS timer overhead
and GC pauses during polling.

### 2d. Sidebar Update Debouncer
**Current:** Every engine tick (700ms) triggers full sidebar re-render (all 12+
sections recompute). Extension fallbacks call setWidget on every tick.

**Rust replacement:** `native/debounce.rs` — `shouldRender() -> bool` that only
allows re-render when content actually changed (hash-based dedup). Saves all the
JS re-rendering when nothing moved.

## Phase 3: Rust Sidecar — `vitriol-daemon` (existing)

The existing `vitriol-daemon` (Unix socket, early stage) can absorb the engine
polling + sidebar debouncing as a persistent background process. This is for
cases where the NAPI addon's background thread isn't sufficient (e.g., the daemon
persists across session restarts).

## What STAYS in TypeScript

| Component | Why |
|---|---|
| Extension API calls (`pi.on`, `pi.registerTool`, etc.) | Pi's plugin contract is TS closures |
| Widget/sidebar rendering (`ctx.ui.setWidget`) | Returns `string[]` to pi-tui Container |
| Event handlers | TypeScript closure state |
| Tool execution handlers | TypeScript functions passed to Pi |
| Command/shortcut registrations | Pi's registration API |
| Session management | Pi's session lifecycle |
| Theme management | Pi's theme system |
| Module hooks (`hooks.mjs`) | Node.js loader API |

## Implementation Order

1. Fix duplicate mode badge + narrow terminal fallback (5 min, TS only)
2. Create `officina/native/` crate with NAPI-RS (30 min)
3. Port ANSI width engine (highest impact, ~200 lines Rust)
4. Port braille gauge rendering (~50 lines Rust)
5. Add NAPI-RS build integration to package.json
6. Update build-patch.mjs to use native functions (guarded by try/catch fallback)
7. Port engine poller to Rust background thread
8. Add sidebar debounce

## Verification

- `cargo test -p officina-native` for Rust unit tests
- `npm run typecheck` for TS type safety
- `npm test` for extension integration tests
- Bench: compare render times before/after native addon
