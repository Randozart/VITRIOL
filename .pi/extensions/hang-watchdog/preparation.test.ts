import { describe, expect, it } from "vitest";
import { admitPreparation, type PreparationBudget, type PreparationFacts } from "./preparation.ts";
const budget: PreparationBudget = { device: "gpu", modelIdentity: "checkpoint@revision", tokenizerIdentity: "tokenizer@revision", schemaIdentity: "watchdog-liveness-v1", resident: true, boundedWorkMs: 5, allowedWorkMs: 10, workingBytes: 100, reserveBytes: 100, allowDecodeOverlap: false };
const facts: PreparationFacts = { now: 10_000, sampledAt: 10_000, foregroundPrefill: false, queuedRequests: 0, foregroundDecode: false, cpuLoad: 0.2, hostFreeBytes: 1000, gpu: { sampledAt: 10_000, utilization: 0.5, freeBytes: 1000 } };

describe("offline preparation admission qualification", () => {
  it("admits idle capacity and requires explicit decode-overlap qualification", () => {
    expect(admitPreparation(budget, facts).allowed).toBe(true);
    expect(admitPreparation(budget, { ...facts, foregroundDecode: true }).allowed).toBe(false);
    expect(admitPreparation({ ...budget, allowDecodeOverlap: true }, { ...facts, foregroundDecode: true }).allowed).toBe(true);
    expect(admitPreparation({ ...budget, device: "cpu", resident: false }, facts).allowed).toBe(true);
  });
  it.each([
    { foregroundPrefill: true }, { foregroundPrefill: null }, { queuedRequests: 1 }, { queuedRequests: null },
    { foregroundDecode: null }, { sampledAt: 7999 }, { sampledAt: 10_001 },
    { gpu: undefined }, { gpu: { ...facts.gpu!, utilization: 0.51 } },
    { gpu: { ...facts.gpu!, sampledAt: 0 } }, { gpu: { ...facts.gpu!, freeBytes: 199 } },
    { gpu: { ...facts.gpu!, freeBytes: NaN } },
  ])("denies foreground/unknown/stale/insufficient evidence %j", (patch) => {
    expect(admitPreparation(budget, { ...facts, ...patch }).allowed).toBe(false);
  });
  it.each([{ resident: false }, { boundedWorkMs: 11 }, { boundedWorkMs: 0 }, { workingBytes: Infinity }, { reserveBytes: -1 }, { modelIdentity: "" }, { schemaIdentity: " " }])("denies unqualified budget %j", (patch) => {
    expect(admitPreparation({ ...budget, ...patch }, facts).allowed).toBe(false);
  });
  it.each([{ cpuLoad: null }, { cpuLoad: 0.51 }, { hostFreeBytes: NaN }, { hostFreeBytes: Infinity }, { hostFreeBytes: 199 }])("denies unavailable CPU capacity %j", (patch) => {
    expect(admitPreparation({ ...budget, device: "cpu" }, { ...facts, ...patch }).allowed).toBe(false);
  });
});
