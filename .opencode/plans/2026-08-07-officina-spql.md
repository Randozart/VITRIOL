# Officina — the model-surgery workshop (Alka / SPQL)

Date: 2026-08-07.

## 1. Vision

A new **OFFICINA tab** in `vitriol-tui`: a transactional REPL where the model is
a queryable database. Concept re-derived from LARQL's "the model is the
database" (public idea, 2026): weights treated as rows in a relational catalog,
editing as composable operations. Left pane = a snappy `>`-pipe command line
with the Kali-style two-line **ALKA-☿** prompt + configurable telemetry header;
right pane = the **Spagyric Journal** (mem arenas, transformation log, context).

**Naming note (2026-08-07):** the old `alka-lang` project is dropped entirely;
the name **Alka** is repurposed for this DSL. No code or design is borrowed from
it. Its historical docs remain untouched as records.

Probe-by-default; `COMMIT >` applies. Operations whose runtime does not exist
(routing, real AOT) are absent from the command set — never stubs (AGENTS §3).

## 2. Grammar (fresh Rust, line-based, non-Turing)

```
COMMAND  := [ "COMMIT" ">" ] KEYWORD ">" TARGET [ARGS]
KEYWORD  := DESCRIBE | DISSOLVE | COAGULATE | TEST | MAP | COMPILE
          | RECORD | STOP | PLAY | UNDO | CLEAR | HELP
```

- Uppercase keywords first; lowercase targets/args; `>` is the work conduit.
- Every mutating op runs as a **probe** by default: impact table, then
  `(Run with 'COMMIT >' to apply)`. `COMMIT >` prefix applies it.
- Read-only ops (DESCRIBE/TEST/MAP) ignore a COMMIT prefix.

## 3. Telemetry config — `~/.vitriol/officina.toml`

```toml
[repl.telemetry]
show_model = true
show_context = true
show_drift = true
show_vram = false
show_experts = false

[repl.style]
theme = "kali-teal"
bold_logo = true
sidebar_width = 35
```

Prompt (bold ALKA, teal box lines, telemetry from live snapshot):

```
┌──(☿ ALKA)-[model: vitriol-32b: dirty]-[context: 4192/8192]-[drift: 0.0024]
└───> _
```

## 4. Journal sidebar (right)

- **MEM ARENAS**: VRAM used/total (GPU snapshot), pinned-RAM est (/proc), cache
  counts (Hermetis episodes/nodes).
- **TRANSFORMATION LOG**: committed ops, undoable.
- **SYSTEMS COGNITION**: context, model, decode t/s.

All data already polled by the snapshot.

## 5. Phases (each commit green)

- **P0** — OFFICINA tab skeleton: two-pane layout, prompt render, `officina.toml`
  load, journal sidebar from snapshot.
- **P1** — SPQL grammar + executor: probe/commit semantics, output tree renderer,
  HELP/UNDO/CLEAR, grimoire (`RECORD`/`STOP`/`PLAY`, `~/.vitriol/grimoires/`),
  COMPILE = package bundle (`.spagyr` = grimoire ref + model fingerprint +
  build-profile JSON).
- **P2** — weight catalog (LARQL DB): extend `gguf.rs` to a tensor catalog
  (name → type/shape/offset/size). DESCRIBE = metadata census; MAP = real system
  memory; TEST = run prompt through the live gen server + syntax check.
- **P3** — weight surgery (offline GGUF rewrite): DISSOLVE (prune mask;
  magnitude instant, wanda calibration phased), COAGULATE (fold norm; f16 exact,
  quant phased), TEST on the rewritten file, logit-parity vs baseline at a print
  boundary. W0 value census (dead-lane %, entropy) gated on a quantized-block
  decoder prototype verified against llama.cpp decode.
- **P4 — deferred**: PIN/ROUTE/STREAM (needs Chimera/WPIR), real AOT COMPILE
  (needs Spagyric). Absent from HELP.

## 6. Gates

Each commit: `cargo test` + `clippy -D warnings` + `fmt --check` + Praetor on
changed files. Final `cargo build --release` + real-TTY verify.

## 7. Provenance

`docs/provenance/officina-spql.md`: concept re-derived from LARQL (public idea),
no third-party code; user-named Alka repurposed; nothing borrowed from the
dropped alka-lang.

## 8. Results

- **P0+P1 landed** (single commit): OFFICINA tab (9th), two-pane layout (REPL +
  Spagyric Journal sidebar), two-line ALKA-☿ prompt with configurable telemetry
  (`~/.vitriol/officina.toml`), `officina/` module (config, grammar, grimoire,
  mod). Commands live: HELP, CLEAR, UNDO, RECORD/STOP/PLAY, COMPILE (.spagyr
  bundle), DESCRIBE (gguf metadata census via vitriol-calibrate), MAP (system
  memory), TEST (live gen server + syntax check). DISSOLVE/COAGULATE report
  "P3 not built yet" — no fake results. Old alka-lang dropped entirely; name
  repurposed.
- P2 (tensor catalog), P3 (offline rewrite), P4 (routing/AOT) — next.

