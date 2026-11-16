import { describe, expect, it } from "vitest";
import {
  parseEnvFile,
  systemdUnit,
  systemdWatchdogService,
  systemdWatchdogTimer,
  upsertEnvFile,
  watchdogDecision,
  WATCHDOG_FAILURE_THRESHOLD,
} from "./caduceus-lib.mjs";

describe("parseEnvFile", () => {
  it("parses simple values and ignores comments", () => {
    const env = parseEnvFile("# note\nA=1\n\nB=two\n");
    expect(env.get("A")).toBe("1");
    expect(env.get("B")).toBe("two");
  });

  it("strips surrounding quotes", () => {
    const env = parseEnvFile('A="hello world"\nB=\'x y\'\n');
    expect(env.get("A")).toBe("hello world");
    expect(env.get("B")).toBe("x y");
  });
});

describe("upsertEnvFile", () => {
  it("replaces keys and preserves unrelated lines", () => {
    const next = upsertEnvFile("# keep\nA=old\nB=stay\n", { A: "new" });
    expect(next).toBe("# keep\nA=new\nB=stay\n");
  });

  it("appends missing keys", () => {
    const next = upsertEnvFile("A=1\n", { B: "2" });
    expect(next).toBe("A=1\nB=2\n");
  });

  it("quotes values with spaces", () => {
    const next = upsertEnvFile("", { A: "hello world" });
    expect(next).toBe('A="hello world"\n');
  });
});


describe("systemdUnit", () => {
  it("builds a foreground gateway unit with optional egress env", () => {
    const unit = systemdUnit({
      bin: "%h/.vitriol/vendor/omniroute/node_modules/.bin/omniroute",
      envFile: "%h/.vitriol/officina/egress.env",
    });
    expect(unit).toContain("ExecStart=%h/.vitriol/vendor/omniroute/node_modules/.bin/omniroute serve --headless --port 20128");
    expect(unit).toContain("EnvironmentFile=-%h/.vitriol/officina/egress.env");
    expect(unit).toContain("Restart=always");
    expect(unit).toContain("WantedBy=default.target");
  });
});

describe("systemdUnit dependability directives (2026-11-16)", () => {
  const unit = systemdUnit({ bin: "/x/omniroute", envFile: "/e.env" });

  it("never gives up after a crash loop", () => {
    // default StartLimitBurst=5/10s parks the unit in `failed` for good
    expect(unit).toContain("StartLimitIntervalSec=0");
    expect(unit).not.toContain("StartLimitBurst=5");
  });

  it("restarts on a clean exit too", () => {
    expect(unit).toContain("Restart=always");
    expect(unit).toContain("RestartSec=2");
  });

  it("is not the preferred OOM victim and throttles instead of dying", () => {
    expect(unit).toContain("OOMScoreAdjust=-100");
    expect(unit).toContain("MemoryHigh=3G");
  });

  it("allows the browser children time to die", () => {
    expect(unit).toContain("TimeoutStopSec=20");
    expect(unit).toContain("KillMode=control-group");
  });
});

describe("systemdWatchdogTimer", () => {
  it("probes after boot and then on a period", () => {
    const timer = systemdWatchdogTimer({ everySeconds: 45 });
    expect(timer).toContain("OnBootSec=90");
    expect(timer).toContain("OnUnitActiveSec=45");
    expect(timer).toContain("Unit=vitriol-caduceus-watchdog.service");
    expect(timer).toContain("WantedBy=timers.target");
  });
});

describe("systemdWatchdogService", () => {
  it("is a bounded oneshot that cannot wedge the timer", () => {
    const svc = systemdWatchdogService({ execStart: "/usr/bin/node /x watchdog --quiet" });
    expect(svc).toContain("Type=oneshot");
    expect(svc).toContain("ExecStart=/usr/bin/node /x watchdog --quiet");
    expect(svc).toContain("TimeoutStartSec=90");
  });
});

describe("watchdogDecision", () => {
  it("resets the counter on a healthy probe", () => {
    expect(watchdogDecision({ healthy: true, unitActive: true, failures: 2 })).toEqual({
      nextFailures: 0,
      action: "none",
      reason: "healthy",
    });
  });

  it("tolerates isolated failures", () => {
    const first = watchdogDecision({ healthy: false, unitActive: true, failures: 0 });
    expect(first).toMatchObject({ nextFailures: 1, action: "none" });
    const second = watchdogDecision({ healthy: false, unitActive: true, failures: 1 });
    expect(second).toMatchObject({ nextFailures: 2, action: "none" });
  });

  it("restarts a wedged unit at the threshold", () => {
    const at = WATCHDOG_FAILURE_THRESHOLD - 1;
    const d = watchdogDecision({ healthy: false, unitActive: true, failures: at });
    expect(d.action).toBe("restart");
    expect(d.nextFailures).toBe(0);
    expect(d.reason).toContain(String(WATCHDOG_FAILURE_THRESHOLD));
  });

  it("starts an inactive unit instead of restarting it", () => {
    const d = watchdogDecision({ healthy: false, unitActive: false, failures: 0 });
    expect(d.action).toBe("start");
    expect(d.nextFailures).toBe(0);
  });

  it("honours a custom threshold", () => {
    const d = watchdogDecision({
      healthy: false,
      unitActive: true,
      failures: 1,
      threshold: 2,
    });
    expect(d.action).toBe("restart");
  });

  it("treats a garbage failure count as zero", () => {
    const d = watchdogDecision({ healthy: false, unitActive: true, failures: NaN });
    expect(d.nextFailures).toBe(1);
  });
});
