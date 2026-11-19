// Band registry tests (2026-11-18).
//
// The rule that matters: a SUGGESTION row never renders while a STATE row is
// present. The owner saw "route manual — overflow paused" followed by
// "▸ next: …" because a suggestion took one of the three strip rows.

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import {
  BAND_MAX_ROWS,
  BAND_WIDGET_KEY,
  bandLines,
  bandSnapshot,
  clearBandRow,
  flushBand,
  resetBand,
  setBandRow,
  setBandUi,
} from "./band.ts";

interface FakeUi {
  calls: { key: string; lines: string[]; placement?: string }[];
  setWidget?: (
    key: string,
    lines: string[],
    options?: { placement?: string },
  ) => void;
}

function fakeUi(): FakeUi {
  const ui: FakeUi = {
    calls: [],
    setWidget(key, lines, options) {
      ui.calls.push({ key, lines, placement: options?.placement });
    },
  };
  return ui;
}

afterEach(() => resetBand());

describe("band row rule", () => {
  it("keeps suggestions out of the strip while state is reporting", () => {
    const ui = fakeUi();
    setBandUi(ui);
    setBandRow("overflow-route", "state", "route manual — overflow paused");
    setBandRow("composer-prediction", "suggestion", "▸ next: run the gates");
    expect(bandLines()).toEqual(["route manual — overflow paused"]);
  });

  it("still shows a suggestion when nothing else reports (negative control)", () => {
    // The rule must not silently delete the feature — with no state rows the
    // prediction is the only thing there is to show.
    const ui = fakeUi();
    setBandUi(ui);
    setBandRow("composer-prediction", "suggestion", "▸ next: run the gates");
    expect(bandLines()).toEqual(["▸ next: run the gates"]);
  });

  it("orders state before progress and caps the strip", () => {
    setBandRow("deep-research-mode", "state", "research x");
    setBandRow("vitriol-decode", "progress", "42 tk/s");
    setBandRow("overflow-route", "state", "route cloud · deepseek");
    setBandRow("plan-mode", "state", "plan 2/5");
    expect(bandLines()).toEqual([
      "route cloud · deepseek",
      "plan 2/5",
      "research x",
    ]);
    expect(bandLines().length).toBe(BAND_MAX_ROWS);
  });

  it("drops empty rows instead of leaving a blank line", () => {
    setBandRow("overflow-route", "state", "route local");
    setBandRow("plan-mode", "state", []);
    setBandRow("vitriol-decode", "progress", "  ");
    expect(bandLines()).toEqual(["route local"]);
  });

  it("treats a cleared row as gone", () => {
    setBandRow("overflow-route", "state", "route local");
    clearBandRow("overflow-route");
    expect(bandSnapshot()).toEqual([]);
    expect(bandLines()).toEqual([]);
  });
});

describe("band flush", () => {
  it("writes ONE below-editor widget, not one per producer", () => {
    const ui = fakeUi();
    setBandUi(ui);
    setBandRow("overflow-route", "state", "route local");
    setBandRow("vitriol-decode", "progress", "42 tk/s");
    expect(ui.calls.length).toBeGreaterThan(0);
    const last = ui.calls[ui.calls.length - 1];
    expect(last.key).toBe(BAND_WIDGET_KEY);
    expect(last.placement).toBe("belowEditor");
    expect(last.lines).toEqual(["route local", "42 tk/s"]);
    // Never the producers' own keys.
    expect(ui.calls.every((c) => c.key === BAND_WIDGET_KEY)).toBe(true);
  });

  it("does not churn the widget when nothing changed", () => {
    // A 10 Hz producer (decode) must not invalidate the widget container, which
    // disturbs scrolling; identical payloads are skipped.
    const ui = fakeUi();
    setBandUi(ui);
    setBandRow("vitriol-decode", "progress", "42 tk/s");
    const after = ui.calls.length;
    flushBand();
    flushBand();
    expect(ui.calls.length).toBe(after);
  });

  it("is the only writer of the band keys", () => {
    // Migration gate (2026-11-18): the strip used to be assembled from one widget
    // key per producer, ordered by a `match` in the Rust layout. Producers now
    // declare rows here; a direct setWidget of a band key would bypass the tier
    // rule and reintroduce "next" under "route".
    const dir = dirname(fileURLToPath(import.meta.url));
    const offenders: string[] = [];
    const keys = [
      "overflow-route",
      "vitriol-decode",
      "plan-mode",
      "phase-model",
      "deep-research-mode",
      "composer-prediction",
    ];
    const walk = (p: string): void => {
      for (const entry of readdirSync(p, { withFileTypes: true })) {
        const full = join(p, entry.name);
        if (entry.isDirectory()) {
          if (entry.name === "node_modules" || entry.name.startsWith("."))
            continue;
          walk(full);
          continue;
        }
        if (!entry.name.endsWith(".ts") || entry.name.endsWith(".test.ts"))
          continue;
        // Squash whitespace before matching (2026-11-19): the gate existed and
        // named vitriol-decode, but the offending call put the key on its own
        // line, so the literal never appeared. A guard that only sees one
        // formatting style guards only one formatting style.
        const text = readFileSync(full, "utf8").replace(/\s+/g, "");
        for (const key of keys) {
          if (text.includes(`setWidget("${key}"`)) offenders.push(`${entry.name}:${key}`);
        }
      }
    };
    walk(resolve(dir, ".."));
    expect(offenders).toEqual([]);
  });

  it("the producer gate sees multi-line calls (the form that slipped past it)", () => {
    // The original regression, verbatim in shape: key on its own line. The old
    // gate matched the un-squashed text, so this passed the guard and shipped.
    const historical = [
      "    if (!last.up) {",
      '      ui.setWidget(',
      '        "vitriol-decode",',
      "        [`◈ VITRIOL  engine down (tris up to start)`],",
      '        { placement: "belowEditor" },',
      "      );",
    ].join("\n");
    expect(historical.includes('setWidget("vitriol-decode"')).toBe(false);
    expect(historical.replace(/\s+/g, "")).toContain('setWidget("vitriol-decode"');
  });

  it("survives a ui that throws", () => {
    setBandUi({
      setWidget: () => {
        throw new Error("widget exploded");
      },
    });
    expect(() =>
      setBandRow("overflow-route", "state", "route local"),
    ).not.toThrow();
  });
});
