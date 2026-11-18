// Structured-output plumbing (2026-11-18).
//
// The measurement (density-bench) decided the policy these tests pin: constrain
// the nested lanes (where an unconstrained model invented `rule_id` and spent
// tokens on pretty-printing) but never override a caller's own format, and never
// touch a lane that did not ask.

import { describe, expect, it } from "vitest";
import {
  SCHEMAS,
  attachStructuredOutput,
  marker,
  requestedSchema,
  structuredOutputEnabled,
} from "./structured-output.ts";

const on = { OFFICINA_STRUCTURED_OUTPUT: "1" } as NodeJS.ProcessEnv;

function payload(system: string, extra: Record<string, unknown> = {}) {
  return {
    messages: [
      { role: "system", content: system },
      { role: "user", content: "audit these diffs" },
    ],
    max_tokens: 128,
    ...extra,
  } as Record<string, unknown>;
}

describe("schema request", () => {
  it("finds a declared marker in the system prompt", () => {
    expect(
      requestedSchema(
        `You audit. Answer ONLY with JSON. ${marker("findings")}`,
      ),
    ).toBe("findings");
  });

  it("ignores an unknown schema name instead of guessing", () => {
    // Fail open: an unknown name must not send a wrong grammar.
    expect(requestedSchema("[[officina-json nonexistent]]")).toBeNull();
  });

  it("finds nothing when the prompt does not ask", () => {
    expect(requestedSchema("Just answer in prose.")).toBeNull();
  });
});

describe("attachStructuredOutput", () => {
  it("attaches the schema the prompt asked for", () => {
    const out = attachStructuredOutput(
      payload(`Answer ONLY with JSON. ${marker("findings")}`),
      on,
    );
    const rf = out.response_format as {
      type: string;
      json_schema: { name: string; schema: unknown };
    };
    expect(rf.type).toBe("json_schema");
    expect(rf.json_schema.name).toBe("findings");
    expect(rf.json_schema.schema).toEqual(SCHEMAS.findings);
  });

  it("is inert unless explicitly enabled (opt-in until re-measured)", () => {
    const text = `Answer ONLY with JSON. ${marker("findings")}`;
    expect(structuredOutputEnabled({} as NodeJS.ProcessEnv)).toBe(false);
    expect(
      attachStructuredOutput(payload(text), {} as NodeJS.ProcessEnv)
        .response_format,
    ).toBeUndefined();
    expect(structuredOutputEnabled(on)).toBe(true);
  });

  it("never overrides a caller's own response_format", () => {
    const explicit = { type: "text" };
    const out = attachStructuredOutput(
      payload(`x ${marker("decision")}`, { response_format: explicit }),
      on,
    );
    expect(out.response_format).toEqual(explicit);
  });

  it("leaves a prose lane byte-identical (negative control)", () => {
    const before = payload("Explain the change in one paragraph.");
    const after = attachStructuredOutput(before, on);
    expect(after).toBe(before);
  });

  it("does not mutate the payload it was given", () => {
    const before = payload(`x ${marker("report")}`);
    const snapshot = JSON.stringify(before);
    attachStructuredOutput(before, on);
    expect(JSON.stringify(before)).toBe(snapshot);
  });

  it("keeps every schema strict-compatible (additionalProperties false)", () => {
    // llama.cpp builds a GBNF grammar from this; a permissive schema yields a
    // permissive grammar, which is the failure we measured.
    for (const [name, schema] of Object.entries(SCHEMAS)) {
      const s = schema as {
        additionalProperties?: boolean;
        required?: string[];
      };
      expect(s.additionalProperties, name).toBe(false);
      expect(Array.isArray(s.required), name).toBe(true);
    }
  });
});
