// Permission policy: modes, protected roots, and bash command risk.
//
// Propagated from docs/DAILY-DRIVER-GAP.md Decision 1, extended 2026-11-10:
//   ask  — in-cwd writes allowed; outside-cwd edits and risky shell ask.
//   auto — allow risky shell; protected roots still ask.
//   yolo — auto-approve everything EXCEPT protected roots and catastrophic
//          commands (operator requirement: "full YOLO except sensitive roots").
//
// Protected roots are checked BEFORE mode/rules so YOLO cannot bypass them.

import { realpathSync } from "node:fs";
import { resolve } from "node:path";
import { matches, type PermAction, type PermSnapshot, decide } from "./perms.ts";

export type PermMode = "ask" | "auto" | "yolo";

export function parseMode(raw: unknown): PermMode {
  const v = String(raw ?? "").trim().toLowerCase();
  return v === "auto" || v === "yolo" ? v : "ask";
}

/** Absolute-form globs. `~` is expanded by the caller into `home`. */
export function protectedGlobs(home: string): string[] {
  return [
    "/etc/**",
    "/boot/**",
    "/usr/**",
    "/sys/**",
    "/proc/**",
    "/dev/**",
    "/var/**",
    `${home}/.ssh/**`,
    `${home}/.gnupg/**`,
    `${home}/.aws/**`,
    `${home}/.config/trismegistus/**`,
    `${home}/.vitriol/secrets`,
    `${home}/.vitriol/secrets/**`,
    `${home}/.vitriol/officina/env`,
    "**/.env",
    "**/.env.*",
    "**/*.pem",
    "**/*.key",
    "**/id_rsa*",
    "**/id_ed25519*",
    "**/.netrc",
    "**/credentials.json",
    "**/.npmrc",
    "**/.pypirc",
  ];
}

function canonical(cwd: string, path: string): string {
  const abs = path.startsWith("/") ? path : resolve(cwd, path);
  try {
    return realpathSync(abs);
  } catch {
    return abs;
  }
}

export function protectedPath(path: string, cwd: string, home: string): boolean {
  const abs = canonical(cwd, path);
  return protectedGlobs(home).some((g) => matches(g, cwd, abs));
}

export function insideCwd(path: string, cwd: string): boolean {
  const abs = canonical(cwd, path);
  const root = cwd.replace(/\/+$/, "");
  return abs === root || abs.startsWith(`${root}/`);
}

export interface Verdict {
  action: PermAction;
  reason: string;
}

/** Path-scoped edit/write/read decision. */
export function pathVerdict(
  tool: string,
  path: string,
  cwd: string,
  home: string,
  mode: PermMode,
  snap: PermSnapshot | null,
): Verdict {
  const t = tool.toLowerCase();
  const v = snap ? decide(snap, t, path, cwd) : null;
  // Explicit deny wins even for a protected path (operator intent).
  if (v && v.action === "deny") {
    return { action: "deny", reason: `${v.pattern} denies ${t} on this path` };
  }
  // Protected roots are checked before allow/ask rules so neither a rule nor
  // YOLO can silently approve them.
  if (protectedPath(path, cwd, home)) {
    return { action: "ask", reason: `protected path (${t} ${path})` };
  }
  if (v && v.action === "allow") {
    return { action: "allow", reason: "path rule" };
  }
  if (v && v.action === "ask") {
    if (mode === "yolo") return { action: "allow", reason: "yolo" };
    return { action: "ask", reason: `rule ${v.pattern} requires approval` };
  }
  // No rule/default: ask only for outside-cwd writes in ask mode.
  if (mode === "ask" && t !== "read" && !insideCwd(path, cwd)) {
    return { action: "ask", reason: "write outside the project" };
  }
  return { action: "allow", reason: "policy default" };
}

const DENY_COMMANDS: RegExp[] = [
  /\brm\s+-[a-zA-Z]*r[a-zA-Z]*\s+\/(?:\s|$)/,
  /\brm\s+-[a-zA-Z]*f[a-zA-Z]*\s+\/(?:\s|$)/,
  /\bmkfs(?:\.|\s)/,
  /\bdd\b[^\n]*\bof=\/dev\//,
  /:\(\)\s*\{[^}]*\|[^}]*&[^}]*\};?:/,
  /\bchmod\s+-R\s+777\s+\/(?:\s|$)/,
  /\b(?:shutdown|reboot|poweroff|halt)\b/,
];

const ASK_COMMANDS: RegExp[] = [
  /\bsudo\b/,
  /\bdoas\b/,
  /\bgit\s+push\b/,
  /\bgit\s+reset\s+--hard\b/,
  /\bgit\s+clean\s+-[a-zA-Z]*f/,
  /\bcurl\b[^\n|]*\|\s*(?:ba|z)?sh\b/,
  /\bwget\b[^\n|]*\|\s*(?:ba|z)?sh\b/,
  /\brm\s+-[a-zA-Z]*r/,
  /\bchmod\s+-R\b/,
  /\bchown\s+-R\b/,
  /\b(?:systemctl|service)\b/,
  /\b(?:pacman|apt|apt-get|dnf|yum|zypper)\b/,
  /\b(?:npm|cargo|pip)\s+publish\b/,
];

/** Shell-command decision. Catastrophic commands deny; protected targets ask. */
export function commandVerdict(
  command: string,
  cwd: string,
  home: string,
  mode: PermMode,
): Verdict {
  const cmd = String(command ?? "");
  if (DENY_COMMANDS.some((rx) => rx.test(cmd))) {
    return { action: "deny", reason: "catastrophic shell command" };
  }
  const roots = [
    "/etc/",
    "/boot/",
    "/usr/",
    "/sys/",
    "/proc/",
    "/dev/",
    `${home}/.ssh`,
    `${home}/.gnupg`,
    `${home}/.aws`,
    `${home}/.config/trismegistus`,
    `${home}/.vitriol/secrets`,
  ];
  // `2>/dev/null` is ubiquitous and harmless. Strip the benign device/fd
  // targets before the protected-root scan; otherwise every command carrying
  // a redirect asks for confirmation — and because pi prepares a whole tool
  // batch before executing any of it, one such ask stalls every parallel call
  // in the turn (observed 2026-11-10: two dangling bash calls, silent TUI).
  const scan = cmd.replace(
    /\/dev\/(?:null|zero|random|urandom|stdin|stdout|stderr|fd\/\d+)/g,
    "",
  );
  const protectedTarget = roots.some((r) => scan.includes(r));
  if (protectedTarget) {
    return { action: "ask", reason: "command targets a protected root" };
  }
  if (mode === "ask" && ASK_COMMANDS.some((rx) => rx.test(cmd))) {
    return { action: "ask", reason: "risky shell command" };
  }
  // `insideCwd` is not meaningful for arbitrary shell text; ask mode still
  // allows ordinary commands (ls, git status, tests) without noise.
  void cwd;
  return { action: "allow", reason: "policy default" };
}
