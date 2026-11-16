# Fire F5 + Caduceus routing dependability — consolidated plan (2026-11-16)

Two work packages, both approved by the owner in the same session:

1. **F5** — the fire's bottom two rows are identical (cosmetic, 1-file fix).
2. **Routing** — "I really want Caduceus to just pick me a model and run with
   it so I don't end up having to resume sessions so much."
   Three layers: **L1** TUI, **L2** router extension, **L3** a routing proxy we
   own.

Detailed research lives in `.opencode/plans/fire-dot-patterns-2026-11-16.md` (F5)
and `.opencode/plans/caduceus-model-routing-dependability-2026-11-16.md` (L1-L3
trace). This document is the build order and the acceptance criteria.

---

## Part 1 — F5: rule must not duplicate the row above

**Cause (measured).** `render`'s bottom row uses `frac = 0.5/rows`, hash seed
`rows-1`; `line_cells` uses `base = 0.5/rows`, seed `rows-1`. Identical inputs ⇒
identical glyph. And the ramp saturates (indices 5…7 at `frac ≈ 0.1`), so
decorrelating alone is not enough once `level ≥ 0.6`.

**Fix (B).** `line_cells` takes the ramp index the body's bottom row would have
and steps one down:

```rust
let idx = body_bottom_index(x, rows, phase_ms, level).saturating_sub(1);
if idx == 0 { continue; }        // first embers keep the underscore
*slot = Some(DENSITY[idx]);
```

**Acceptance.** Every claimed column differs from the row above at
`level ∈ {0.06, 0.3, 0.6, 1.0}`; ember intermittency, the `_` rule, the middle
band and the `⣿` ceiling unchanged; the seven existing line/plane tests stay
green.

---

## Part 2 — Routing

### L1 — TUI: a rate limit is a switchable failure (`mod.rs`, `heal.rs`)

`handle_provider_failure` currently handles `EngineDown | NoPhysicalModel` and
`ModelNotFound`; everything else (including `RateLimit`) only shows a notice
(`mod.rs:3361`). Change:

- New `heal::quota_hint(detail) -> Option<Duration>`: parse `retry_after`
  (seconds) or `reset_at` (ISO-8601) out of the provider message. Clamp
  1 s … 1 h.
- `Failure::RateLimit`:
  - not on lapis ⇒ `SetModel{officina/lapis}` + `pending_replay` (the
    `ModelNotFound` path, `mod.rs:3327`), notice names the lane and the reset;
  - already on lapis ⇒ `pending_replay.at = now + hint.unwrap_or(90 s)`, bounded
    by `policy.max_attempts` / window; when the budget is spent ⇒ `degrade()`
    with a notice that says how long to wait.
- Reuse `failure_seen` dedupe so a burst cannot stack replays.

**Acceptance.** Classification tests for the two hint formats; a 429 while not
on lapis schedules a replay and requests the lapis model; a 429 on lapis
schedules at the hint (not the default); exhaustion degrades instead of
looping.

### L2 — router extension: cooling must engage on the virtual lane

`adaptive-router/index.ts:497` starts with `if (tier !== "cloud" || !activeCloud)
return;`, so on `officina/lapis` no cooling is ever recorded. Change:

- Record provider failures for **whatever served** the request (from the event's
  provider/model) even when `tier !== "cloud"`; keep the lane-picking logic
  gated as today.
- Accept the quota hint from the response **body** (`retry_after`, `reset_at`)
  when the `Retry-After` header is absent — the gateway emits it in JSON.

**Acceptance.** A 429 body hint parks the lane for the stated duration; a
lapis-tier session still records the failure; `pickCloud` skips a cooled lane
(existing behaviour, now with data); the router test matrix stays green.

### L3 — a routing proxy we own (`officina/scripts/caduceus-router.cjs`)

The gateway is a vendor package (`~/.vitriol/vendor/omniroute`) and its
`config` CLI only manages *client* tool configs — there is no supported knob for
"reroute this request to another model until one answers". So L3 is a small
proxy we own, modelled on `decision-shim.cjs`:

- Listens on `CADUCEUS_ROUTER_PORT` (default **20129**), forwards to the
  gateway (20128).
- Candidate list: the request's model first, then models from
  `/v1/models` filtered to lanes that are not parked; parked lanes are read from
  a persisted state file (`caduceus-router-state.json`) with `until` timestamps
  taken from `retry_after` / `reset_at` in the error body.
- Per-attempt deadline (connect + first byte, default 45 s). **Retry only before
  the first byte reaches the client** — never after, so a partial stream is
  never duplicated.
- Park a lane on 429/401/403/404/5xx **or** deadline expiry; give up only when
  every candidate is parked, and say how many were tried and when the earliest
  returns.
- Append one line per reroute to the route ledger
  (`~/.vitriol/officina/route-ledger.jsonl`, existing format) so a silent
  reroute is auditable, and expose state for `/doctor`.

**Acceptance.** Unit tests for candidate ordering, parking from both hint
formats, "parked lanes are never chosen", "no retry after the first byte", and
the give-up message. Live check: with the proxy in the path, killing the lane a
request is pinned to must still produce a 200.

### Wiring and rollout

- `vitriol caduceus router [--install|--status|--remove]` installs the proxy as
  a user unit next to the gateway; the Officina provider entry points at 20129
  when enabled, 20128 otherwise. Off by default (`CADUCEUS_ROUTER=1` to enable).
- `/doctor` gains a line: proxy active, candidates, parked lanes, last reroute.

---

## Invariants (all layers)

- **I30** every ladder has a candidate budget and a total deadline.
- **I31** a parked lane (`until > now`) is never selected — router and proxy.
- **I32** a reroute is never silent: ledger line + at most one UI line.
- **I33** healthy behaviour is byte-identical: no extra probe, no extra hop, no
  latency when nothing fails (proxy passes through untouched).
- **I34** local is the floor: when every cloud candidate is parked the turn
  continues on `officina/lapis`; only then may it fail.
- **I35** no duplicate output: the proxy never retries after forwarding bytes.

## Build order

1. F5 (fire, 1 file + tests) — install.
2. L1 (TUI heal) — install.
3. L2 (router extension).
4. L3 (proxy + tests + unit wiring + `/doctor` line).
5. Gates: `cargo test` / `cargo fmt --check` / `vitest` / `tsc` / release build /
   install / PTY smoke; one commit per stage; as-landed notes appended here.

## Risk

- Fire F5: display only.
- L1: routing/state only, bounded by the existing heal budget; worst case a
  notice, never a loop.
- L2: cools lanes it previously ignored — a too-aggressive cool could starve a
  provider; the 1 m…24 h clamp and `/doctor` make it visible.
- L3: an extra loopback hop, off by default. If it proves unnecessary it is
  removed by flipping one env var; the failure modes it covers (dead provider,
  quota, first-byte hang) have no cheaper owner in this stack.

---

# As landed (2026-11-16)

| stage | commit | what shipped |
|---|---|---|
| F5 | `0562de9` | rule is one ramp step below the flame's bottom row; installed `3a736cc4` |
| L1 | `6d91a81` | TUI RateLimit → switch to lapis / scheduled replay; hint parsing; replay budget; installed `2531f660` |
| L2 | `049a01d` | quota hint read from the error text, bare `retry_after` accepted, lane parked for the stated duration |
| L3 | (this commit) | `caduceus-router` proxy + lib + e2e, `vitriol caduceus router` CLI |

## Verification

- **F5**: dump at levels 0.3 / 0.6 / 1.0 shows the rule strictly lighter than the
  row above (`⡀⡄…` vs `⡀⡇…`; `⡆⡇…` vs `⡇⣇…`; `⡇⣧…` vs `⣇⣷…`).
  316 cargo tests.
- **L1**: 429 hint parsing in both shapes (bare seconds / ISO), clamping,
  unparseable input, replay budget + refill — 3 new tests.
- **L2**: the operator's real error text now parks the lane for 177 s instead of
  60 s; `parseCooldownFromError` gained the bare-number shape it never matched
  (it had tests but **no production caller**).
- **L3**: 16 policy tests + 4 e2e tests against a stub gateway
  (reroute → attempt=2, healthy lane → attempt=1, parked lane skipped, 177 s
  parking).
- Gates: 1368 vitest, 316 cargo, `tsc` clean, `cargo fmt --check` clean.

## What is NOT enabled

The proxy is installed on demand and **stopped by default**
(`vitriol caduceus router --install [--enable]`). With no
`CADUCEUS_ROUTER_FALLBACKS` it is a pass-through, so I33 (healthy behaviour is
byte-identical) holds whether or not the unit exists. Choosing the fallback
list is the operator's call: a different model mid-session changes behaviour, so
it is not inferred from the catalog.

## Known limit (vendor)

pi's `after_provider_response` event carries `{status, headers}` only — no body
and no provider/model — so the router cannot cool a lane from that event alone.
L2 therefore reads the hint from `message_end`'s `errorMessage`, which is where
Caduceus' JSON ends up. A future pi that includes the body would let the cooling
happen a beat earlier.

---

# Fallback configuration in the Caduceus panel (2026-11-16, `fd75671`)

Owner: *"make sure the fallback is easily configured in the Caduceus menu. For me
this is OpenCode Go's DeepSeek v4.1 Flash. If all else fails, I keep using that
model."*

## Shipped

- **Prefs contract** (`caduceus-prefs.ts` + Rust `CaduceusPrefs`): `fallback: []`
  — serde-defaulted, skipped when empty, so an empty policy writes nothing
  (I1 parity: absent ⇒ unchanged behaviour).
- **Panel**: Models tab, `f` marks/unmarks the selected model as the reroute
  fallback; the row shows `⤵`; the normal prefs save persists it. Test covers
  the round-trip *and* that clearing it removes the field from the file.
- **Proxy** follows the panel: env `CADUCEUS_ROUTER_FALLBACKS` wins (scripted
  setups), otherwise the prefs file, mtime-cached so a change in the panel takes
  effect without a restart. `/health` and `vitriol caduceus router` report the
  effective list.
- **Fail-loud fix**: the quiet `catch` in the prefs read hid a real bug
  (`statSync is not defined` after an import refactor) that made the proxy
  silently ignore the panel. It now logs the reason once per read.

## Configured for this host

`~/.vitriol/officina/caduceus-prefs.json`:
`"fallback": ["opencode-go/deepseek-v4.1-flash"]` (id verified present in the
gateway catalog; the older `opencode-go/deepseek-v4-flash` is on the hidden
list, the v4.1 id is the one to use).

Unit `vitriol-caduceus-router.service` is **enabled** on port 20129.

## Live proof

```
$ curl -D- http://127.0.0.1:20129/v1/chat/completions -d '{"model":"agnes/agnes-3.0-flash",…}'
HTTP/1.1 200   t=2.39s
x-caduceus-router: model=opencode-go/deepseek-v4.1-flash;attempt=2
{"model":"deepseek-v4.1-flash", …}

state:   agnes/agnes-3.0-flash parked for its stated reset (HTTP 429)
ledger:  {"model":"agnes/agnes-3.0-flash","status":429,…} then
         {"model":"opencode-go/deepseek-v4.1-flash","status":200,"served":true,"attempt":2}
```

## Open item (not guessed)

For a *session* to benefit, its `caduceus` provider must address the proxy
(`http://127.0.0.1:20129/v1`) instead of the gateway (`:20128`). The extensions
honour `OFFICINA_CADUCEUS_BASE_URL`, but I could not find where pi's `caduceus`
provider is registered: no `registerProvider("caduceus")` in
`officina/.pi/extensions`, no `caduceus` entry in `~/.pi/agent/models-store.json`
or `settings.json`, and pi's own bundle does not define one. Rather than guess at
a provider URL and break the live path, this stays open with the knob named.

Until it is wired, the user-visible symptom is still covered by **L1**: a 429
switches the turn to `officina/lapis` and replays it.
