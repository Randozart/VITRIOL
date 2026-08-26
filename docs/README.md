# VITRIOL Documentation Map

> *"Visita Interiora Terrae Rectificando Invenies Occultum Lapidem"*

This directory holds the **living** documentation. Everything else is history —
see [`archive/`](archive/) for the full excavation record.

## Start here

| document | what it is |
|---|---|
| [../README.md](../README.md) | project thesis, quick start |
| [ARCHITECTURE.md](ARCHITECTURE.md) | how the system works today (single source of truth) |
| [OPERATIONS.md](OPERATIONS.md) | the self-healing runtime: watchdogs, checkpoints, bounces |
| [VERDICTS.md](VERDICTS.md) | tombstones — every abandoned line, with measurements and reasons |
| [GLOSSARY.md](GLOSSARY.md) | LULL, tq3_0, slot tenancy, fingerprints — the project's vocabulary |
| [BENCHMARKS.md](BENCHMARKS.md) | certified numbers (filled-context depth, not shallow benches) |

## Reference

| document | what it is |
|---|---|
| [CONFIG_REFERENCE.md](CONFIG_REFERENCE.md) | every config key |
| [RECOMMENDED_SETTINGS.md](RECOMMENDED_SETTINGS.md) | per-model launch recommendations |
| [CONFIG_DEFAULTS_GUIDE.md](CONFIG_DEFAULTS_GUIDE.md) | default values + rationale |
| [vitriol.env.example](vitriol.env.example) | environment variables |
| [RESOURCE_LOCATIONS.md](RESOURCE_LOCATIONS.md) | where external resources live |

## Subsystems

| document | subsystem |
|---|---|
| [hermetis.md](hermetis.md) | Hermetis — memory system |
| [REBIS.md](REBIS.md) / [REBIS_FLAGS.md](REBIS_FLAGS.md) | Rebis — dual-model cognitive architecture |
| [officina-guide.md](officina-guide.md) | Officina — interactive workshop |
| [copula.md](copula.md) | Copula — opencode integration |
| [spagyric-autotuner.md](spagyric-autotuner.md), [spagyric-profile-schema.md](spagyric-profile-schema.md) | Spagyric — autotuner |

## Integration

| document | what it is |
|---|---|
| [OPENCODE_SETUP.md](OPENCODE_SETUP.md) | using VITRIOL with opencode |

## Directories

- `provenance/` — GPL compliance: inspiration-vs-copy records per module
- `reference/` — third-party reference material
- `plans/` — historical planning documents
- `optimizations/` — optimization notes
- `pymander/` — curated reference-mind nodes
- `archive/` — **54+ session reports, sprint logs, and superseded designs
  from May–August 2026.** Kept because negative results are part of the
  record; nothing in here describes current behavior.
