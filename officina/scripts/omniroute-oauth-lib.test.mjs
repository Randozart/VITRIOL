import { describe, expect, it } from "vitest";
import {
  findOAuthProvider,
  OAUTH_PROVIDERS,
  parseOAuthCallback,
  resolveBackendKey,
} from "./omniroute-oauth-lib.mjs";

describe("OAuth provider map", () => {
  it("contains native and free flows", () => {
    expect(findOAuthProvider("copilot")?.backend).toBe("github");
    expect(findOAuthProvider("claude-code")?.backend).toBe("claude");
    expect(findOAuthProvider("openference")?.native).toBe(false);
    expect(findOAuthProvider("freebuff")?.flow).toBe("device");
    expect(findOAuthProvider("qoder")?.requiresEnv).toContain("QODER_OAUTH_CLIENT_ID");
    expect(OAUTH_PROVIDERS.some((p) => p.id === "amazon-q" && p.backend === "kiro")).toBe(true);
  });

  it("resolves backend aliases", () => {
    expect(resolveBackendKey("claude-code")).toBe("claude");
    expect(resolveBackendKey("copilot")).toBe("github");
    expect(resolveBackendKey("openference")).toBe("openference");
    expect(resolveBackendKey("unknown")).toBe("unknown");
  });
});

describe("parseOAuthCallback", () => {
  it("parses a full callback URL", () => {
    expect(parseOAuthCallback("http://127.0.0.1:56123/callback?code=abc&state=xyz", "xyz")).toEqual({
      code: "abc",
      state: "xyz",
    });
  });

  it("parses code#state paste", () => {
    expect(parseOAuthCallback("abc#xyz", "xyz")).toEqual({ code: "abc", state: "xyz" });
  });

  it("accepts a raw code when no state is present", () => {
    expect(parseOAuthCallback("abc")).toEqual({ code: "abc", state: "" });
  });

  it("rejects state mismatch", () => {
    expect(() => parseOAuthCallback("http://x/cb?code=abc&state=wrong", "expected")).toThrow(
      /state mismatch/i,
    );
  });

  it("rejects OAuth errors and empty codes", () => {
    expect(() => parseOAuthCallback("http://x/cb?error=access_denied")).toThrow(/authorization failed/i);
    expect(() => parseOAuthCallback("http://x/cb?state=only")).toThrow(/no authorization code/i);
    expect(() => parseOAuthCallback("")).toThrow(/empty/i);
  });
});
