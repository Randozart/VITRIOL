// Sandbox wrapper tests (codex-qol stage 10, invariants I1/I6).
import { describe, expect, it } from "vitest";
import { findBwrap, shellQuote, shouldWrap, wrapBwrap } from "./sandbox.ts";

describe("shouldWrap", () => {
  it("wraps only sandboxed — the I1 golden gate", () => {
    expect(shouldWrap("sandboxed")).toBe(true);
    expect(shouldWrap("ask")).toBe(false);
    expect(shouldWrap("auto")).toBe(false);
    expect(shouldWrap("yolo")).toBe(false);
    expect(shouldWrap("")).toBe(false);
  });
});

describe("shellQuote", () => {
  it("survives quotes, dollars and newlines", () => {
    expect(shellQuote("ls -la")).toBe("'ls -la'");
    expect(shellQuote("echo 'hi'")).toBe(`'echo '\\''hi'\\'''`);
    expect(shellQuote("echo $HOME")).toBe(`'echo $HOME'`);
    expect(shellQuote("a\nb")).toBe(`'a\nb'`);
    const round = shellQuote("printf '%s' \"$(whoami)\"");
    // re-parsing via bash is the real oracle; structural check here
    expect(round.startsWith("'")).toBe(true);
    expect(round.endsWith("'")).toBe(true);
  });
});

describe("wrapBwrap", () => {
  it("builds a ro-bind box with rw cwd+/tmp and no network by default", () => {
    const wrapped = wrapBwrap("cargo test", "/proj", "/usr/bin/bwrap", { net: false });
    expect(wrapped).toContain(`'/usr/bin/bwrap'`);
    expect(wrapped).toContain(`'--ro-bind' '/' '/'`);
    expect(wrapped).toContain(`'--bind' '/proj' '/proj'`);
    expect(wrapped).toContain(`'--bind' '/tmp' '/tmp'`);
    expect(wrapped).toContain(`'--unshare-net'`);
    expect(wrapped.endsWith(`'--' 'bash' '-c' 'cargo test'`)).toBe(true);
  });

  it("drops --unshare-net when the network is opted in", () => {
    const wrapped = wrapBwrap("cargo fetch", "/proj", "/usr/bin/bwrap", { net: true });
    expect(wrapped).not.toContain("--unshare-net");
  });

  it("the payload survives as one bash -c argument (quotes inside)", () => {
    const cmd = `echo 'a b' && echo "$X"`;
    const wrapped = wrapBwrap(cmd, "/proj", "/usr/bin/bwrap", { net: false });
    // the last quoted chunk must be the command, quoting intact
    const tail = wrapped.slice(wrapped.lastIndexOf("'bash'")).trim();
    expect(tail.endsWith(shellQuote(cmd))).toBe(true);
    expect(shellQuote(cmd)).toContain(`'\\''a b'\\''`);
    expect(shellQuote(cmd)).toContain("$X");
  });
});

describe("findBwrap", () => {
  it("honours the override env, missing → null", () => {
    const prev = process.env.OFFICINA_SANDBOX_BWRAP;
    process.env.OFFICINA_SANDBOX_BWRAP = "/definitely/not/here";
    expect(findBwrap()).toBeNull();
    process.env.OFFICINA_SANDBOX_BWRAP = "/bin/sh";
    expect(findBwrap()).toBe("/bin/sh");
    if (prev === undefined) delete process.env.OFFICINA_SANDBOX_BWRAP;
    else process.env.OFFICINA_SANDBOX_BWRAP = prev;
  });
});
