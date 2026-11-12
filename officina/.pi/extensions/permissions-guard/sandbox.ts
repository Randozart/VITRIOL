// bwrap sandbox wrapper (codex-qol stage 10, invariant I6). Only the
// `sandboxed` permissions mode ever wraps a command; ask/auto/yolo stay
// byte-identical. The wrapper is built as a quoted argv string — quoting is
// the load-bearing part, so it is unit-tested against quotes, $ and newlines.

import { existsSync } from "node:fs";

/** I1/I6 golden gate: only sandboxed wraps. */
export function shouldWrap(mode: string): boolean {
  return mode === "sandboxed";
}

/** Locate bwrap: OFFICINA_SANDBOX_BWRAP override, then common paths. */
export function findBwrap(): string | null {
  const override = process.env.OFFICINA_SANDBOX_BWRAP;
  if (override !== undefined && override !== "") {
    return existsSync(override) ? override : null;
  }
  for (const p of ["/usr/bin/bwrap", "/bin/bwrap"]) {
    if (existsSync(p)) return p;
  }
  return null;
}

/** POSIX single-quote: every byte survives, including `'`, `$`, newlines. */
export function shellQuote(s: string): string {
  return `'${s.replace(/'/g, `'\\''`)}'`;
}

export interface SandboxOpts {
  /** OFFICINA_SANDBOX_NET=1 re-allows the network (default off). */
  net: boolean;
}

/** Wrap `command` so it runs inside a bwrap box: read-only filesystem, rw
 *  cwd + /tmp, private /dev and /proc, no network unless opted in. */
export function wrapBwrap(command: string, cwd: string, bwrap: string, opts: SandboxOpts): string {
  const argv: string[] = [
    bwrap,
    "--ro-bind",
    "/",
    "/",
    "--bind",
    cwd,
    cwd,
    "--bind",
    "/tmp",
    "/tmp",
    "--dev",
    "/dev",
    "--proc",
    "/proc",
  ];
  if (!opts.net) argv.push("--unshare-net");
  argv.push("--", "bash", "-c", command);
  return argv.map(shellQuote).join(" ");
}
