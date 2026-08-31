import { describe, expect, it } from "vitest";
import { ChurnTracker, churnConfig } from "./churn.ts";

const cfg = churnConfig({ OFFICINA_NO_CHURN: "", OFFICINA_CHURN_LOOP: "3", OFFICINA_CHURN_FILE: "5" } as NodeJS.ProcessEnv);
const obs = (file: string, oldHash: string, newHash: string) => ({ file, oldHash, newHash });

describe("churnConfig", () => {
  it("defaults on and kill-switches", () => {
    expect(churnConfig({} as NodeJS.ProcessEnv).enabled).toBe(true);
    expect(churnConfig({ OFFICINA_NO_CHURN: "1" } as NodeJS.ProcessEnv).enabled).toBe(false);
  });
});

describe("ChurnTracker", () => {
  it("stays silent on healthy varied edits", () => {
    const t = new ChurnTracker(cfg);
    expect(t.record(obs("a.ts", "o1", "n1"))).toBeNull();
    expect(t.record(obs("a.ts", "n1", "n2"))).toBeNull();
    expect(t.record(obs("b.ts", "o9", "n9"))).toBeNull();
  });
  it("fires the loop directive on the threshold repeat", () => {
    const t = new ChurnTracker(cfg);
    t.record(obs("a.ts", "o1", "n1"));
    t.record(obs("a.ts", "n1", "o1"));
    t.record(obs("a.ts", "o1", "n1"));
    expect(t.record(obs("a.ts", "o1", "n1"))).toMatchObject({ message: expect.stringContaining("NOT sticking") });
  });
  it("fires the volume warning once per file", () => {
    const t = new ChurnTracker(cfg);
    let vol: string | null = null;
    for (let i = 0; i < 5; i++) {
      const d = t.record(obs("b.ts", `o${i}`, `n${i}`));
      if (d?.message.includes("5 edits")) vol = d.message;
    }
    expect(vol).toBeTruthy();
    // further edits stay silent about volume
    expect(t.record(obs("b.ts", "x", "y"))).toBeNull();
  });
});
