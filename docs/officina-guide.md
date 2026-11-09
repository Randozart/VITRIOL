# Officina — The Workshop Guide

> *"The forge, the crucibles, the stills."* Officina is the interactive workshop
> where the model is treated as a queryable database and every edit is a
> composable operation. Type `GUIDE > topic` in the REPL to jump to a section.

## Usage

- **The prompt**: `┌──(☿ ALKA)-[model: …]-[context: …]-[drift: …]` then
  `└───> `. Telemetry blocks are toggled in `~/.vitriol/officina.toml`.
- **Autofill**: press **Tab** to cycle keyword/target completions (shown on the
  ⧉ bar under the prompt). BackTab still cycles tabs.
- **Scroll**: **PgUp**/**PgDn** scroll the output; Up/Down recall command
  history; Enter runs a command.
- **Grammar**: `[COMMIT … >] KEYWORD > target args`. Uppercase keywords first,
  lowercase targets/args, `>` is the work conduit.

## Subscriptions via OmniRoute

Officina remains engine-first. To use subscription providers such as OpenCode
Go, run:

    vitriol omniroute setup --opencode-key-stdin < keyfile
    vitriol omniroute status

Then open `vitriol officina` and select `/model` → `omniroute/<model>`. The
choice persists. A fresh session still defaults to the local VITRIOL engine.
`vitriol omniroute up|down|sync` manages the gateway. See `docs/OFFICINA.md`
for the passthrough contract and rollback steps.

Zero-key free providers:

    vitriol omniroute setup --free
    vitriol omniroute doctor
    vitriol omniroute service install
    vitriol omniroute use ddgw/gpt-5.4-mini       # persist cloud default
    vitriol omniroute accounts free --template    # fill-in free API-key list
    vitriol omniroute accounts free --apply
    vitriol omniroute accounts discover --apply   # reuse OpenCode auth

Bulk accounts:

    vitriol omniroute accounts init       # ~/.vitriol/officina/accounts.json
    vitriol omniroute accounts sync       # API keys via credential_env refs
    vitriol omniroute accounts status
    vitriol omniroute accounts oauth list

API-key providers import in bulk. OAuth is assisted, not unattended: browser
or device consent is still required except `cursor`/`zed` system imports.
Web-cookie and local providers stay manual.

## Cloud cache, Praetor, egress

Cloud cache mode is automatic for non-local providers:

    vitriol omniroute cache enable --providers opencode-go,glm
    vitriol cache-report

Praetor is advisory and opt-in:

    OFFICINA_PRAETOR=1 vitriol officina

Fixed egress only (no quota-based IP rotation):

    vitriol omniroute egress set --proxy socks5://127.0.0.1:1080
    vitriol omniroute egress status
    vitriol omniroute egress test

See `docs/OFFICINA.md` for cache-safe rules, Praetor budgets, and egress policy.

## Probe vs Commit

Every mutating command runs as a **probe** by default — it shows the impact and
changes nothing. Prefix with a commit to apply:

- `COMMIT overwrite >` — modify the active model/mask in place.
- `COMMIT as "name" >` — write a new target, base untouched.
- Bare `COMMIT >` on a base-destructive op is **blocked** (safety contract).
  Artifact ops (COMPILE, grimoires) keep bare `COMMIT >`.

## Diagnose

- `DESCRIBE > model` — aggregate census: arch, layers, experts, tensors, size.
- `DESCRIBE > layer.12` / `layer.12.mlp` — per-tensor catalog (quant, size).
- `CENSUS > layer.12` — W0 value census: dead-lane %, entropy, magnitude
  (decodes f32/f16/q8_0/q4_0; other types reported unsupported).
- `MAP` — real system memory: VRAM, host RAM, Hermetis, context, decode t/s.
- `TEST > "prompt"` — run a prompt through the active model and syntax-check.

## Rectify (masks)

`RECTIFY` tallies which MoE experts actually fire for your workload and stores
the result in a **named mask** — an auditable, versioned list of transactions.

- `RECTIFY > "prompt" into my_mask` — run a generation, record the fired
  experts into `my_mask` (live data needs the fork's expert-activity hook).
- `DESCRIBE > model my_mask` — census: active %, dross, estimated savings.
- `LOG > model my_mask` — the transaction history.
- `REVERT > model my_mask 2` — probe shows the impact; `COMMIT overwrite >`
  surgically drops transaction 2.
- `DISCARD > model my_mask` — delete the mask.
- The right-hand **Spagyric Journal** `[MASKS]` panel lists every mask live.

## Ascensus

- `ASCENSUS > RECTIFY > "specialize in Vulkan shaders" 50 into vulkan_shaders`
  — the cloud model writes 50 calibration prompts, then they run locally as a
  batch to populate the mask in minutes. Needs a Gemini key set in the
  SUBSYSTEMS tab (`~/.vitriol/secrets`).

## Grimoires

- `RECORD > "sys-opt"` — begin capturing committed ops.
- `STOP` — write `~/.vitriol/grimoires/sys-opt.grimoire` (plain SPQL text,
  git-trackable).
- `PLAY > "sys-opt"` — probe lists the recipe; `COMMIT > PLAY` runs it.

## Compile

- `COMPILE > "name"` — package a `.spagyr` bundle (grimoire ref + model
  fingerprint + profile). Probe by default; `COMMIT >` writes it.
- A real AOT backend (Spagyric) is planned.

## Weight surgery

- `DISSOLVE > layer.12.mlp wanda 0.35` — weight pruning.
- `COAGULATE > layer.12 norm into mlp` — fold a normalizer into the weights.
- Both need the offline-rewrite backend (P3) — until then they report it is
  not built yet. When live, `COMMIT as "name" > DISSOLVE > model <mask>` writes
  a pruned variant.

## Undo

- `UNDO` reverts the last committed journal entry. Mask edits are versioned:
  `REVERT` on a mask is always exact.
