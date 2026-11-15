# Officina model picker groups + OpenCode Go catalog + Tab/ghost fix

- Date: 2026-11-15
- Status: implemented, rebuilt, installed
- Origin: operator requests —
  1. group the `/model` picker: Officina specials at the top (Lapis,
     Nimbus), then one header per provider;
  2. bring the operator's OpenCode Go models into Officina directly (the
     models, not OpenCode's settings);
  3. after a GRIND loop ends, Tab must still cycle agent modes.

## Model picker grouping

`officina/tui/src/tui/state.rs`

- `filtered_model_groups()` builds display groups; `filtered_models()` is
  the flattened model order the selection indexes.
- OFFICINA group first (rank 0): Lapis, Nimbus, then any other Officina
  virtual models.
- OpenCode Go next (rank 1), then every provider/owner alphabetically.
- Caduceus ids are `owner/model`, so the owner is the group. Example
  headers: `OpenCode Go`, `Z.AI`, `GLM`, `AgentRouter`, `Cloudflare
  Playground`, `VITRIOL local`.
- `OpenCode Go` merges the native provider and gateway copies, deduping by
  model id in favour of the native per-model API/metadata entry.
- `officina/tui/src/tui/layout.rs` renders header rows with a count and
  page-scrolls by display row, not model index.

## OpenCode Go models

`officina/tui/src/rpc/bridge.rs`

- pi has the OpenCode Go provider built in but gates it on
  `OPENCODE_API_KEY`. The bridge reads the operator's existing key from
  OpenCode's auth store at spawn time (`$XDG_DATA_HOME/opencode/auth.json`
  or `~/.local/share/opencode/auth.json`) and passes it to the pi child.
- The key is never copied into Officina config, reports, logs or argv.

`officina/.pi/extensions/opencode-go-catalog/index.ts`

- Re-registers the built-in `opencode-go` provider with two models its
  generated catalog lags: `hy4` and `longcat-2.5-free`.
- Reuses the built-in provider object (auth + stream handlers untouched);
  appends only missing ids, so a future pi catalog bump cannot duplicate.
- Kill switch: `OFFICINA_NO_OPENCODE_GO_CATALOG=1`.
- Result verified against pi RPC: 32 `opencode-go` models, including both
  extras.

## Tab after GRIND

`officina/tui/src/tui/mod.rs`

- Tab was swallowed whenever `prompt_ghost.is_some()`. After a GRIND loop
  settled, a voice ghost could leave the operator with a no-op Tab. The
  guard now only swallows Tab for the command-autocomplete popup; a ghost
  no longer blocks agent cycling (and is still not accepted by Tab).
- Regression test: `tab_cycles_agent_even_when_a_ghost_suggestion_is_visible`.

## Gates

- `cargo test`: 276 passed (271 binary + 5 protocol).
- `cargo fmt --check` clean.
- `cargo clippy -- -D warnings` clean.
- New vitest: `opencode-go-catalog/index.test.ts` 3 passed.
- Release rebuilt + installed to `~/.local/bin/officina`.
