# Fiat Lux — the model triad, one-tap free tiers, composer wrap marker

- Date: 2026-11-09
- Status: approved, implementing
- Scope: `officina/scripts`, `officina/runtime`, `scripts/vitriol`, `~/.vitriol/officina/`, docs

## The triad

Three named model states, one hot-swap command (`/coupling`), conversation preserved by `pi.setModel`:

| Name | Meaning | Backing |
|---|---|---|
| **Lapis Occultus** | the stone; local engine, whatever model it serves | `llamacpp` provider (exists, default) |
| **Fiat Lux** | the free merry-go-round: all zero-cost providers, fallback chain | new coupling `fiat-lux` |
| *named cloud* | one targeted subscription | existing `couplings.json` entries (`ascensus`, `zen-free`, …) |

Name approved by owner 2026-11-09: **Fiat Lux** ("let there be light" — the free
tiers are borrowed light, many sources, no obligation).

## Fiat Lux coupling

- Generator: pure function in `officina/scripts/omniroute-lib.mjs`
  (catalog rows → `CouplingDef` with `id: "fiat-lux"`).
- Membership: model id or name contains `-free` / `free`, or provider tagged
  zero-cost in the gateway catalog. Fallback chain order: current winner first
  (stickiest), newest last; round-robin advance on provider failure (no IP
  rotation, no quota evasion — one identity per provider).
- Writer: merges into `~/.vitriol/officina/couplings.json` (atomic, mode 600,
  never clobbers other couplings).
- Regeneration: `vitriol fiat` (and after any new subscription) rewrites the
  `fiat-lux` entry from the live catalog (`~/.vitriol/officina/omniroute-models.json`
  or fresh fetch).
- Tests: `omniroute-lib.test.mjs` (catalog → CouplingDef, idempotent merge,
  free-only filtering).

## `vitriol fiat` — one screen, minimal owner effort

```
◈ Fiat Lux — free tiers                [current: ox-alpha-free]
→ opencode-go  ox-alpha-free      ✓ active      (reuses local auth)
  gemini         gemini-free       ○ one-click   (browser consent)
  claude-code    claude-sonnet     ○ one-click   (browser OAuth)
  codex          gpt-5-mini        ○ one-click   (callback)
  copilot        gpt-4.1-copilot   ○ paste-code  (device flow)
  freebuff       assorted-free     ○ paste-code  (device flow)
  aihorde        (pool, zero-key)  ✓ active
── key-required plans (N) ──
  mistral        mistral-8b       🔑 paste key   (env ref only)
```

Per-row effort ladder (reuses already-built flows — nothing new gateway-side):
1. Zero-key / reused auth → `accounts free --apply` (DuckDuckGo, Cloudflare
   playground, AI Horde + `auth.json` reuse of opencode-go/zai/zai-coding-plan).
2. Browser OAuth → `accounts oauth login <id>`; CLI drives
   `/api/oauth/<backend>/authorize|exchange|poll|device-code`; owner's only
   action = click consent in the browser.
3. API-key plans → masked stdin prompt, stored as **env ref** in
   `accounts.json` (never literal, never logged).
4. After any success: catalog refresh → `fiat-lux` regenerated → fingerprint
   diff → `omniroute doctor` row.

Explicitly out of scope: rotating proxies/VPN for quota evasion, multi-account
farming of one provider. Fixed egress only, one identity per provider.

## Composer wrap-edge marker

Root cause (verified in vendor code): the composer is pi's multi-line
`Editor` (`CustomEditor` in `runtime/patched/interactive-mode.officina.js:513`).
`layoutText()` (pi-tui `editor.js`) **does** word-wrap via `wordWrapLine`; the
box already reserves rows (`maxVisibleLines = max(5, 30% terminal rows)`).
But when a layout row overflows the visible width, the overflow is silently
clipped and only the *top* border shows a scroll indicator — so typing at the
right edge looks like a hard cut.

Fix (anchored build-patch, vendor file
`node_modules/@earendil-works/pi-coding-agent/dist/modes/interactive/components/custom-editor.js`):
append a trailing `»` marker on any visible row whose text is wider than
`contentWidth`. Patch anchored in `officina/runtime/build-patch.mjs` with a
`[officina P<N>]` canary the build greps (Vendor Patch Rule — never hand-edit
the generated file). PTY smoke: type a 200-char string, assert marker visible.

Notes: submit = Enter; new line = Shift+Enter / Ctrl+J (or `\` before Enter —
existing upstream workaround). No keybinding changes in this plan.

## Build order

1. `fiat-lux` generator + writer + tests (`omniroute-lib.mjs`, tests).
2. `vitriol fiat` TUI (`officina/scripts/fiat.mjs`, picker pattern from
   `opencode.mjs`) + `scripts/vitriol` dispatch + selfcheck entry.
3. Composer `»` marker: anchored build-patch + canary + `node runtime/build-patch.mjs`
   + PTY smoke.
4. Docs: `docs/OFFICINA.md` Couplings section gains the triad + Fiat Lux name +
   per-member privacy law; `docs/officina-guide.md`; this plan updated with
   verification results.
5. Full `npm test` + typecheck + selfcheck → commit → push.

## Verification (fill in on completion)

- [ ] `vitriol fiat` lists current free tiers with correct state badges
- [ ] `fiat-lux` coupling present in `~/.vitriol/officina/couplings.json`, mode 600
- [ ] `/coupling fiat-lux` hot-swaps mid-session, conversation preserved
- [ ] Regeneration after a new free tier adds the model to Fiat Lux
- [ ] Composer: long prompt shows `»` at the wrap edge, no clipping
- [ ] `node runtime/build-patch.mjs` → `canaries ok`
- [ ] `npm test`, typecheck, selfcheck all green; committed + pushed
