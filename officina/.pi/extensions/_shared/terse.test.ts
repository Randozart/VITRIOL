import { describe, expect, it } from "vitest";
import {
  TERSE_INSTRUCTION,
  alreadyTerse,
  applyTerseInstruction,
  hasTerseMarker,
  terseEnabled,
} from "./terse.ts";

const payload = (system: string, user = "plan it") => ({
  messages: [
    { role: "system", content: system },
    { role: "user", content: user },
  ],
});

describe("terse lanes (E2)", () => {
  it("appends the measured instruction and removes the marker", () => {
    const on = { OFFICINA_TERSE: "1" } as NodeJS.ProcessEnv;
    const out = applyTerseInstruction(
      payload("You plan engine changes. [[officina-terse]]"),
      on,
    );
    const messages = out.messages as Array<{ role: string; content: string }>;
    expect(messages[0].content).toContain(TERSE_INSTRUCTION);
    expect(messages[0].content).not.toContain("[[officina-terse]]");
    // The instruction is appended once, not duplicated.
    const occurrences = messages[0].content.split(TERSE_INSTRUCTION).length - 1;
    expect(occurrences).toBe(1);
  });

  it("is off unless the knob says otherwise (negative control)", () => {
    // The feature is opt-in: default behaviour must be byte-identical.
    const off = {} as NodeJS.ProcessEnv;
    expect(terseEnabled(off)).toBe(false);
    const untouched = payload("You plan engine changes. [[officina-terse]]");
    expect(applyTerseInstruction(untouched, off)).toBe(untouched);
  });

  it("leaves payloads without the marker alone even when enabled", () => {
    const on = { OFFICINA_TERSE: "1" } as NodeJS.ProcessEnv;
    const plain = payload("You plan engine changes.");
    expect(hasTerseMarker(plain)).toBe(false);
    expect(applyTerseInstruction(plain, on)).toBe(plain);
  });

  it("keeps a caller's own brevity wording instead of stacking a second sentence", () => {
    // A lane that already asked for terseness gets its marker stripped and nothing
    // else: two near-identical instructions only spend tokens.
    const on = { OFFICINA_TERSE: "1" } as NodeJS.ProcessEnv;
    const out = applyTerseInstruction(
      payload("Answer concisely. [[officina-terse]]"),
      on,
    );
    const messages = out.messages as Array<{ role: string; content: string }>;
    expect(messages[0].content).toBe("Answer concisely.");
    expect(messages[0].content).not.toContain(TERSE_INSTRUCTION);
  });

  it("adds a system message rather than rewriting the operator's user text", () => {
    const on = { OFFICINA_TERSE: "1" } as NodeJS.ProcessEnv;
    const out = applyTerseInstruction(
      { messages: [{ role: "user", content: "plan [[officina-terse]] it" }] },
      on,
    );
    const messages = out.messages as Array<{ role: string; content: string }>;
    expect(messages[0]).toEqual({ role: "system", content: TERSE_INSTRUCTION });
    expect(messages[1].content).toBe("plan  it");
  });

  it("detects brevity requests in the caller's own words", () => {
    expect(alreadyTerse("Answer concisely.")).toBe(true);
    expect(alreadyTerse("Be terse.")).toBe(true);
    expect(alreadyTerse("You are planning an engine change.")).toBe(false);
  });
});
