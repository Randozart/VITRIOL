# Caduceus dependability (2026-11-16 16:30)

Owner ask: "can we make Caduceus more dependable?" Caduceus is the local gateway
(`vitriol caduceus`, provider id `caduceus`, OmniRoute vendor at
`~/.vitriol/vendor/omniroute`) and the primary model path for
`officina/lapis`. A failure here is a dead session, so the bar is supervision,
not features.

## Evidence gathered before changing anything

`vitriol caduceus doctor` said 0 failures, and the live unit still had five
real defects (`systemctl --user show vitriol-caduceus.service`):

| property | observed | why it matters |
|---|---|---|
| `Restart` | `on-failure` | a clean self-exit leaves the gateway down |
| `StartLimitIntervalUSec` / `Burst` | `10s` / `5` | after 5 crashes in 10 s systemd parks the unit in `failed` — permanently |
| `OOMScoreAdjust` | `+200` | preferred OOM victim on a box that has already been OOM-killed repeatedly; the engine needed `-500` for the same reason |
| `MemoryHigh/Max` | `infinity` | memory peak measured at **2.6 G** with 696 M swap — it swells into the killer |
| bind address | `0.0.0.0` | journal, every boot: *"listening on 0.0.0.0 with NO API-key requirement — the inference plane (/v1/\*) is reachable by ANY device that can route to this host, and requests are billed to your configured providers"* |
| readiness | none | cold start measured **62 s** (16:21:59 → 16:23:01), and the vendor's own log prints *"set OMNIROUTE_READY_TIMEOUT_MS=120000 … for slower cold starts"* |
| `NRestarts` | `1` | it had already restarted once unnoticed |

Also observed: `doctor`'s binary check races a busy gateway — one run reported
`gateway binary: missing` while the binary was present and `--version` answered
in 0.3 s. A false "missing" is worse than a second attempt.

And the structural gap: **systemd only notices a process that exits.** A wedged
gateway (blocked event loop, an upstream that never answers, a Playwright
channel stuck) stays `active` while every model call hangs. Nothing watched it.

## What shipped

### Unit (`officina/scripts/caduceus-lib.mjs`)
- `Restart=always`, `RestartSec=2`
- `StartLimitIntervalSec=0` — never parks the unit in `failed`; restarts stay
  visible in the journal and in `NRestarts`
- `OOMScoreAdjust=-100` (below the engine's `-500`, above default tenants),
  `MemoryHigh=3G` so it reclaims instead of inviting the killer
- `TimeoutStopSec=20` for the Playwright children
- `OMNIROUTE_SERVER_HOST=127.0.0.1` — the inference plane is loopback-only
- `OMNIROUTE_READY_TIMEOUT_MS=120000` — matches the measured 62 s cold start
- New generators: `systemdWatchdogTimer()`, `systemdWatchdogService()`

### Supervision (`officina/scripts/caduceus.mjs`)
`vitriol caduceus watchdog [--install|--remove|--json]`:
- probes **both planes**: `/api/health` (dashboard) and `/v1/models` (auth +
  router — the path a chat request takes, no provider spend). A dashboard-only
  "ok" is how a wedged router hides.
- `watchdogDecision()` (pure, in `caduceus-lib.mjs`): healthy ⇒ counter reset;
  inactive unit ⇒ `start`; ≥ 3 consecutive failures on an active unit ⇒
  `restart`; otherwise wait with the counter persisted in
  `~/.vitriol/officina/caduceus-watchdog.json`.
- installed as `vitriol-caduceus-watchdog.{service,timer}` (60 s cadence,
  `OnBootSec=90`, oneshot bounded at `TimeoutStartSec=90` so the probe can
  never wedge the timer). `service install` wires it; `--no-watchdog` opts out.
- `service install` also bounces an already-active unit so new directives take
  effect now instead of at the next reboot.

### Doctor
New checks: `restart policy`, `oom priority`, `restart count`, `watchdog`
(timer + last action), `inference plane`, `responsiveness` (latency buckets),
and a missing env file is now a **failure** (the unit's `EnvironmentFile` is
optional, so a missing file booted a keyless gateway). The binary check retries
once and distinguishes "missing" from "present but `--version` failed".

## Invariants

- **I1 — one healthy gateway**: with both probes green the watchdog performs no
  action (`action: "none"`), so a healthy system is untouched. Pinned by
  `watchdogDecision` tests (`resets the counter on a healthy probe`).
- **I2 — no self-healing loop**: a restart resets the failure counter, so the
  watchdog cannot restart more than once per `threshold` ticks. Pinned by
  `restarts a wedged unit at the threshold` (`nextFailures: 0`).
- **I3 — local reachability unchanged**: the gateway still answers
  `127.0.0.1:20128` with the existing inference key; only the *non-loopback*
  exposure is gone. Verified live with a `/v1/chat/completions` 200.
- **I4 — doctor is honest**: every check reports the state it observed
  (statuses from `systemctl`/HTTP), and a warning names the exact command that
  fixes it (`vitriol caduceus service install`, `… watchdog --install`).

## Verification (live, 2026-11-16 16:27–16:31)

```
$ systemctl --user show vitriol-caduceus.service -p Restart -p StartLimitIntervalUSec -p OOMScoreAdjust -p MemoryHigh -p TimeoutStopUSec
Restart=always
StartLimitIntervalUSec=0
OOMScoreAdjust=-100
MemoryHigh=3221225472
TimeoutStopUSec=20s

$ systemctl --user is-active vitriol-caduceus-watchdog.timer
active                       # next run in 35s, ticks every 60s

$ ss -ltnp | grep 20128
LISTEN 127.0.0.1:20128       # was 0.0.0.0

$ vitriol caduceus watchdog --json
{ "healthy": true, "latencyMs": 104, "failures": 0, "apiLayer": true,
  "apiStatus": 200, "reason": "healthy", "restarts": 0 }

$ curl -X POST /v1/chat/completions  (model auto/glm)
http=200 t=8.7s  content="pong"

$ vitriol caduceus doctor
OK gateway binary · gateway · inference plane (15ms) · responsiveness (66ms)
OK restart policy · oom priority · restart count · watchdog
0 failure(s), 1 warning(s)   # the warning is the pre-existing "egress: off"
```

Gates: `npx vitest --run extensions scripts` → 1346 passed, 18 skipped;
`npx tsc --noEmit` clean. (`core-rpc.test.ts` fails only when the ambient
`OFFICINA_ROUTER_CORE=rust` is exported — pre-existing, unrelated.)

## Follow-ups worth considering (not done)

1. `Restart=always` + `StartLimitIntervalSec=0` means a *permanently* broken
   gateway restarts every 2 s forever. The journal makes it visible and the
   watchdog counts restarts, but a `RestartSteps=` backoff would be gentler.
2. The watchdog restarts only after 3 failures (≈3 min). If the owner wants a
   faster recovery, lower `CADUCEUS_WATCHDOG_SECONDS` to 30.
3. Client-side hang tolerance (pi's own request timeouts) was not audited; a
   hung gateway is now recovered in ≤3 min, but the in-flight turn still waits.
