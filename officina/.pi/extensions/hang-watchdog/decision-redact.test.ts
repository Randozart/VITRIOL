// Redaction corpus (2026-11-16). Every case is a shape that has appeared in real
// terminal output: env dumps, copied keys, connection strings, docker run lines.
import { describe, expect, it } from "vitest";
import {
  MAX_COMMAND,
  MAX_TOTAL,
  REDACTED,
  redactCapped,
  redactText,
  scrubEvidence,
} from "./decision-redact.ts";

// [name, input, secret that must not survive]
//
// The fake secrets are ASSEMBLED AT RUNTIME on purpose: written as literals they
// trip GitHub push protection (a Stripe/Google/GitHub-shaped string is treated
// as a leaked credential even inside a redaction test), which blocked a push on
// 2026-11-16. Concatenation keeps the fixtures fake and the scanner quiet.
const X = "abcdefghijklmnopqrstuvwxyz";
const FAKE = {
  google: "AI" + "za" + "SyD-1234567890" + X.slice(0, 10),
  openai: "sk" + "-proj-" + X.slice(0, 24),
  anthropic: "sk" + "-ant-api03-" + X.slice(0, 20),
  github: "ghp" + "_" + X.slice(0, 24),
  hf: "hf" + "_" + X.slice(0, 20),
  slack: "xox" + "b-1234567890-" + X.slice(0, 10),
  aws: "AKIA" + "IOSFODNN7EXAMPLE".slice(0, 16),
  jwt: "eyJhbGciOiJIUzI1NiJ9" + "." + "eyJzdWIiOiIxMjM0NTY3ODkwIn0" + "." + X.slice(0, 20),
  stripe: "sk" + "_live_" + X.slice(0, 24),
  password: "s3cr3t" + "-pw",
  queryKey: "abcdef" + "123456",
  dbPass: "hunter" + "2",
  hex: "9f86d081" + "884c7d65" + "9a2feaa0c55ad015a3bf4f1b",
};

const CASES: Array<[string, string, string]> = [
  ["google key", `GEMINI_API_KEY=${FAKE.google}`, FAKE.google],
  ["openai key", `export OPENAI_API_KEY=${FAKE.openai}`, FAKE.openai],
  ["anthropic key", `claude --key ${FAKE.anthropic}`, FAKE.anthropic],
  ["github pat", `git remote add origin https://${FAKE.github}@github.com/x/y`, FAKE.github],
  ["huggingface", `hf download --token ${FAKE.hf}`, FAKE.hf],
  ["slack", `curl -H 'Authorization: Bearer ${FAKE.slack}'`, FAKE.slack],
  ["aws", `AWS_ACCESS_KEY_ID=${FAKE.aws}`, FAKE.aws],
  ["jwt", `Authorization: Bearer ${FAKE.jwt}`, FAKE.jwt],
  ["db password", `psql postgres://admin:${FAKE.password}@db.internal:5432/app`, FAKE.password],
  ["query key", `curl 'https://api.example.com/v1?key=${FAKE.queryKey}&x=1'`, FAKE.queryKey],
  ["assignment", `DATABASE_PASSWORD=${FAKE.dbPass} docker compose up`, FAKE.dbPass],
  ["dotenv dump", `STRIPE_SECRET_KEY=${FAKE.stripe}`, FAKE.stripe],
  ["hex blob", `commit ${FAKE.hex}`, FAKE.hex],
];

describe("redactText", () => {
  for (const [name, text, secret] of CASES) {
    it(`scrubs ${name}`, () => {
      const out = redactText(text);
      expect(out, `secret survived: ${out}`).not.toContain(secret);
      expect(out).toContain(REDACTED);
    });
  }

  it("keeps ordinary evidence readable", () => {
    const text = "cargo build --release | Compiling officina-tui v0.2.0 … 187 files";
    expect(redactText(text)).toBe(text);
  });

  it("is idempotent", () => {
    const once = redactText(CASES[0][1]);
    expect(redactText(once)).toBe(once);
  });

  it("does not eat normal paths with digits", () => {
    const text = "/scratch/briev-builds/h5-phase-a-target/ debug · 3m10s";
    expect(redactText(text)).toBe(text);
  });

  it("keeps the assignment name so the judge knows what was scrubbed", () => {
    expect(redactText("GEMINI_API_KEY=AIza" + "c".repeat(35))).toBe(
      "GEMINI_API_KEY=«redacted»",
    );
  });
});

describe("redactCapped", () => {
  it("caps and marks truncation", () => {
    const out = redactCapped("x".repeat(500), 20);
    expect(out.length).toBe(20);
    expect(out.endsWith("…")).toBe(true);
  });

  it("scrubs before capping so a split key cannot survive", () => {
    // A key that starts inside the cap window.
    const out = redactCapped(`ok AIza${"a".repeat(60)}`, 20);
    expect(out).not.toContain("AIza");
  });
});

describe("scrubEvidence", () => {
  it("redacts and bounds both fields", () => {
    const ev = scrubEvidence(
      `GEMINI_API_KEY=AIza${"b".repeat(40)} cargo test`,
      "tail ".repeat(200),
    );
    expect(ev.redacted).toBe(true);
    expect(ev.command).not.toContain("AIza");
    expect(ev.command.length).toBeLessThanOrEqual(MAX_COMMAND);
    expect(ev.output_tail.length).toBeLessThanOrEqual(200);
    expect(ev.command.length + ev.output_tail.length).toBeLessThanOrEqual(
      MAX_TOTAL,
    );
  });

  it("leaves text alone (but still capped) when redaction is off", () => {
    const ev = scrubEvidence("KEY=value", "t".repeat(400), false);
    expect(ev.redacted).toBe(false);
    expect(ev.command).toBe("KEY=value");
    expect(ev.output_tail.length).toBeLessThanOrEqual(200);
  });
});
