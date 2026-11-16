// Pure env-file helpers for the Caduceus bootstrap (OmniRoute vendor, MIT). Separated so tests do not
// import the CLI entrypoint (which runs main()).

export function parseEnvFile(content) {
  const values = new Map();
  for (const rawLine of String(content).split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    const eq = line.indexOf("=");
    if (eq <= 0) continue;
    const key = line.slice(0, eq).trim();
    let value = line.slice(eq + 1).trim();
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1);
    }
    values.set(key, value);
  }
  return values;
}

export function formatEnvValue(value) {
  const text = String(value);
  return /^[A-Za-z0-9_./:@%+,=-]+$/.test(text) ? text : JSON.stringify(text);
}

export function upsertEnvFile(content, updates) {
  const lines = String(content).split(/\r?\n/);
  while (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  const seen = new Set();
  const out = lines.map((line) => {
    const match = line.match(/^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=/);
    if (!match) return line;
    const key = match[1];
    if (!(key in updates)) return line;
    seen.add(key);
    return `${key}=${formatEnvValue(updates[key])}`;
  });
  for (const [key, value] of Object.entries(updates)) {
    if (!seen.has(key)) out.push(`${key}=${formatEnvValue(value)}`);
  }
  while (out.length > 0 && out[out.length - 1] === "") out.pop();
  return `${out.join("\n")}\n`;
}

export function systemdUnit({ bin, envFile, port = 20128 }) {
  return `[Unit]
Description=Caduceus local gateway for Officina (OmniRoute vendor)
After=network-online.target
Wants=network-online.target
# Dependability (2026-11-16): a local gateway that gives up after a crash
# loop is worse than one that keeps trying — the whole model path is behind
# it. Rate limiting is disabled (0) so systemd never parks the unit in the
# failed state; every restart is still logged and counted (NRestarts).
StartLimitIntervalSec=0

[Service]
Type=simple
ExecStart=${bin} serve --headless --port ${port}
Environment=PORT=${port}
Environment=OMNIROUTE_HEADLESS=1
# Loopback only: the vendor warns that the default binds 0.0.0.0 and serves
# /v1/* anonymously, which puts the operator's provider accounts on the LAN
# (2026-11-16: the journal printed exactly that warning on every boot).
Environment=OMNIROUTE_SERVER_HOST=127.0.0.1
# Cold start measured at 62s on this host (esbuild + DB + Playwright); the
# vendor's default ready timeout is shorter and kills a slow start.
Environment=OMNIROUTE_READY_TIMEOUT_MS=120000
EnvironmentFile=-${envFile}
# Restart=always also covers a clean exit (OmniRoute self-exits on config
# reload/channel close); on-failure left the gateway dead until someone
# noticed the model path was gone.
Restart=always
RestartSec=2
# ...but back off instead of hammering every 2s forever when the gateway is
# permanently broken (systemd 254+; ignored on older). StartLimitIntervalSec=0
# means systemd would otherwise never stop retrying.
RestartSteps=5
RestartMaxDelaySec=60
# Playwright children must be reaped before the next start.
TimeoutStopSec=20
KillMode=control-group
# The gateway peaks ~2.6G with browser channels. Throttle (MemoryHigh) so it
# reclaims instead of walking into the kernel OOM killer, and keep it off the
# preferred-victim list (+200 used to make it the first thing the OOM killer
# took while the box was already under memory pressure).
MemoryHigh=3G
OOMScoreAdjust=-100

[Install]
WantedBy=default.target
`;
}

/** Watchdog timer unit — period probe that restarts a hung gateway. */
export function systemdWatchdogTimer({ everySeconds = 60 } = {}) {
  return `[Unit]
Description=Caduceus gateway watchdog (health probe + restart)

[Timer]
OnBootSec=90
OnUnitActiveSec=${everySeconds}
AccuracySec=10
Unit=vitriol-caduceus-watchdog.service

[Install]
WantedBy=timers.target
`;
}

/** Watchdog oneshot service — one health probe per timer tick. */
export function systemdWatchdogService({ execStart }) {
  return `[Unit]
Description=Caduceus gateway watchdog run (one health probe)
After=vitriol-caduceus.service

[Service]
Type=oneshot
ExecStart=${execStart}
# The probe itself must never wedge the timer.
TimeoutStartSec=90
Nice=10
`;
}

// ── Watchdog policy (2026-11-16) ──────────────────────────────────────────
// Pure so the decision is testable without systemd or a live gateway.
//
// Why a watchdog at all: systemd notices a process that EXITS. It cannot
// notice one that is alive but wedged (event loop blocked by a Playwright
// channel, an upstream that never answers). For a local gateway that is the
// whole model path, "alive but not answering" is the common failure.

/** Consecutive failed probes tolerated before the unit is restarted. */
export const WATCHDOG_FAILURE_THRESHOLD = 3;

/**
 * Decide what the watchdog should do after one health probe.
 *
 * @param {object} input
 * @param {boolean} input.healthy        probe answered within the timeout
 * @param {boolean} input.unitActive     `systemctl --user is-active` succeeded
 * @param {number}  input.failures       consecutive failures BEFORE this probe
 * @param {number} [input.threshold]     failures before a restart
 * @returns {{ nextFailures: number, action: "none"|"start"|"stop"|"restart", reason: string }}
 */
export function watchdogDecision({
  healthy,
  unitActive,
  failures,
  threshold = WATCHDOG_FAILURE_THRESHOLD,
}) {
  const prior = Number.isFinite(failures) && failures > 0 ? Math.floor(failures) : 0;
  if (healthy) {
    return { nextFailures: 0, action: "none", reason: "healthy" };
  }
  if (!unitActive) {
    // Dead, not wedged: starting is cheaper and safer than restarting a
    // unit systemd already considers stopped.
    return { nextFailures: 0, action: "start", reason: "unit inactive but port dead" };
  }
  const nextFailures = prior + 1;
  if (nextFailures < threshold) {
    return {
      nextFailures,
      action: "none",
      reason: `probe ${nextFailures}/${threshold} failed — waiting`,
    };
  }
  // Sustained failure on a running unit = wedged event loop.
  return {
    nextFailures: 0,
    action: "restart",
    reason: `${nextFailures} consecutive failed probes on an active unit`,
  };
}
