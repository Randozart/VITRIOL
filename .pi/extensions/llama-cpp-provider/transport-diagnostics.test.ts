import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { createRequire } from "node:module";
import { channel } from "node:diagnostics_channel";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const require = createRequire(new URL("../../../node_modules/@earendil-works/pi-coding-agent/package.json", import.meta.url));
const { createJiti } = require("jiti") as typeof import("jiti");
let directory = "";
beforeAll(() => {
  directory = mkdtempSync(join(tmpdir(), "officina-transport-diag-"));
  vi.stubEnv("TRIS_STATE_DIR", directory);
});
afterAll(() => {
  vi.unstubAllEnvs();
  rmSync(directory, { recursive: true, force: true });
});

describe("local transport diagnostics under isolated loaders", () => {
  it("subscribes once across jiti instances and records only sanitized transport metadata", async () => {
    const a = createJiti(import.meta.url, { moduleCache: false });
    const b = createJiti(import.meta.url, { moduleCache: false });
    const path = new URL("./transport-diagnostics.ts", import.meta.url).pathname;
    const first = await a.import(path) as typeof import("./transport-diagnostics.ts");
    const second = await b.import(path) as typeof import("./transport-diagnostics.ts");
    first.observeLocalTransport("http://user:secret@local.invalid:8279/private-key/v1");
    second.observeLocalTransport("http://local.invalid:8279/v1");
    channel("undici:request:error").publish({
      request: { origin: "http://local.invalid:8279", path: "/private-key/v1/chat/completions?api_key=secret", headers: "Authorization: secret", body: "private prompt" },
      error: { code: "UND_ERR_BODY_TIMEOUT" },
    });
    const text = readFileSync(join(directory, "events.jsonl"), "utf8");
    const rows = text.trim().split("\n").map((line) => JSON.parse(line));
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({ src: "lc-local-http", ev: "transport_error" });
    expect(rows[0].detail).toContain("UND_ERR_BODY_TIMEOUT");
    expect(rows[0].detail).toContain("http://local.invalid:8279");
    expect(text).not.toMatch(/secret|private-key|private prompt|Authorization/);
    expect(() => channel("undici:request:error").publish(null)).not.toThrow();
  });
});
