// Project-scoped permission allowlist (2026-11-17).
//
// Why not the global file: `~/.config/trismegistus/permissions.json` is a
// GENERATED mirror (`source_hash`, written by `trismegistus perms-sync`), so a
// rule appended there is destroyed by the next sync. Why not in the repo: this
// monorepo publishes snapshots of `officina/`, and a file that grants the agent
// permissions must never ship inside such a snapshot by accident.
//
// So the store lives under the operator's home, keyed per project:
//   ~/.vitriol/officina/perms/<slug>-<hash8>.json   (mode 0600)
//
// Invariants (see .opencode/plans/permission-modal-command-and-project-allow-2026-11-17.md):
//   I1 fail closed    — unreadable/corrupt file means NO rules
//   I3 exact by default — `kind: "command"` compares the normalised command
//                         string, never a prefix or substring
//   I4 project scope  — rules are filed under the project path and never leak
//   I6 no repo writes — the store path is always under $HOME
import {
  closeSync,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
  fsyncSync,
} from "node:fs";
import { homedir } from "node:os";
import { basename, join } from "node:path";

export type ProjectRuleKind = "command" | "program";

export interface ProjectRule {
  tool: string;
  kind: ProjectRuleKind;
  pattern: string;
  at: string;
}

export interface ProjectStore {
  version: 1;
  project: string;
  rules: ProjectRule[];
}

/** FNV-1a over the project path: stable, dependency-free, collision-cheap here. */
function pathHash(path: string): string {
  let hash = 0xcbf29ce484222325n;
  for (const byte of Buffer.from(path, "utf8")) {
    hash ^= BigInt(byte);
    hash = (hash * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return hash.toString(16).padStart(16, "0").slice(0, 8);
}

function slug(path: string): string {
  const name = basename(path)
    .replace(/[^A-Za-z0-9._-]+/g, "-")
    .slice(0, 40);
  return name || "project";
}

/** Where this project's rules live. Always under $HOME (I6). */
export function projectStorePath(cwd: string, home = homedir()): string {
  return join(
    home,
    ".vitriol",
    "officina",
    "perms",
    `${slug(cwd)}-${pathHash(cwd)}.json`,
  );
}

/** Normalise a command the same way the dialog and the policy see it. */
export function normalizeCommand(command: string): string {
  return String(command ?? "")
    .replace(/\s+/g, " ")
    .trim();
}

/**
 * The leading program of a shell command, or "" when it cannot be determined
 * safely. Shell keywords and env-prefixes are skipped so `FOO=1 cargo build`
 * yields `cargo`, but a compound (`a && b`) still yields `a` (the rule then only
 * covers the first program, which is the conservative reading).
 */
export function leadingProgram(command: string): string {
  const parts = normalizeCommand(command).split(" ").filter(Boolean);
  for (const part of parts) {
    if (/^[A-Za-z_][A-Za-z0-9_]*=/.test(part)) continue; // env prefix
    if (
      part === "sudo" ||
      part === "command" ||
      part === "exec" ||
      part === "time"
    )
      continue;
    return basename(part);
  }
  return "";
}

/** Read the store. Any failure is "no rules" (I1). */
export function loadProjectStore(cwd: string, home = homedir()): ProjectStore {
  const empty: ProjectStore = { version: 1, project: cwd, rules: [] };
  const path = projectStorePath(cwd, home);
  try {
    const doc = JSON.parse(readFileSync(path, "utf8")) as Partial<ProjectStore>;
    if (!doc || !Array.isArray(doc.rules)) return empty;
    const rules = doc.rules.filter(
      (r): r is ProjectRule =>
        !!r &&
        typeof r.tool === "string" &&
        (r.kind === "command" || r.kind === "program") &&
        typeof r.pattern === "string" &&
        r.pattern.length > 0,
    );
    return { version: 1, project: cwd, rules };
  } catch {
    return empty;
  }
}

/** Write atomically with 0600: a torn store must never grant anything. */
export function saveProjectStore(
  store: ProjectStore,
  home = homedir(),
): boolean {
  const path = projectStorePath(store.project, home);
  const dir = join(home, ".vitriol", "officina", "perms");
  try {
    mkdirSync(dir, { recursive: true, mode: 0o700 });
    const tmp = `${path}.${process.pid}.tmp`;
    writeFileSync(tmp, `${JSON.stringify(store, null, 2)}\n`, { mode: 0o600 });
    const fd = openSync(tmp, "r+");
    try {
      fsyncSync(fd);
    } finally {
      closeSync(fd);
    }
    renameSync(tmp, path);
    return true;
  } catch {
    try {
      rmSync(`${path}.${process.pid}.tmp`, { force: true });
    } catch {
      /* best effort */
    }
    return false;
  }
}

/** Append a rule (idempotent: same tool+kind+pattern replaces the old stamp). */
export function addProjectRule(
  cwd: string,
  rule: Omit<ProjectRule, "at"> & { at?: string },
  home = homedir(),
): boolean {
  const store = loadProjectStore(cwd, home);
  const at = rule.at ?? new Date().toISOString();
  const next: ProjectRule = {
    tool: rule.tool,
    kind: rule.kind,
    pattern: rule.pattern,
    at,
  };
  store.rules = store.rules.filter(
    (r) =>
      !(
        r.tool === next.tool &&
        r.kind === next.kind &&
        r.pattern === next.pattern
      ),
  );
  store.rules.push(next);
  return saveProjectStore(store, home);
}

/** Drop every rule for this project. */
export function clearProjectRules(cwd: string, home = homedir()): boolean {
  try {
    rmSync(projectStorePath(cwd, home), { force: true });
    return true;
  } catch {
    return false;
  }
}

/**
 * Does a stored rule cover this call? `command` matching is EXACT (I3); a
 * `program` rule only covers the leading program, which is what the operator
 * opted into when they chose it in the dialog.
 */
export function matchesProjectRule(
  rules: readonly ProjectRule[],
  tool: string,
  command: string,
): ProjectRule | null {
  const t = String(tool ?? "").toLowerCase();
  const exact = normalizeCommand(command);
  if (!exact) return null;
  const program = leadingProgram(exact);
  for (const rule of rules) {
    if (rule.tool !== t) continue;
    if (rule.kind === "command" && rule.pattern === exact) return rule;
    if (rule.kind === "program" && program && rule.pattern === program)
      return rule;
  }
  return null;
}

/** True when the store file exists (used by the UI to say "3 rules"). */
export function hasProjectStore(cwd: string, home = homedir()): boolean {
  return existsSync(projectStorePath(cwd, home));
}
