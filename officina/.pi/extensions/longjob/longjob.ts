// longjob rules (2026-11-12). Pure + injectable: matcher, dispatch
// rewrite, registry announce-once logic. Invariants live in
// .opencode/plans/longjob-2026-11-12.md — headline: fail-open, never
// kill, heuristic precision, #fg always wins.

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

/** Command prefixes that legitimately run minutes, not milliseconds. */
const LONG_RUNNER_RE =
  /^\s*(?:sudo\s+|env\s+\S+\s+|CARGO_TARGET_DIR=\S+\s+)*?(?:cargo\b|make\b|brievc\b|npm\s+(?:install|ci|run|test|build)|pnpm\s|yarn\s|cmake\b|go\s+(?:build|test|install)|gradle\b|mvn\b|pytest\b|rustc\b|clang(?:\+\+)?\b|g\+\+\b)/;

/** True when the command should be dispatched instead of blocking. */
export function isLongRunner(command: string): boolean {
  const c = String(command ?? "");
  if (hasFgOptOut(c)) return false;
  return LONG_RUNNER_RE.test(c);
}

/** Trailing `#fg` forces foreground (I4: always wins). */
export function hasFgOptOut(command: string): boolean {
  return /(^|\s)#fg\s*$/i.test(String(command ?? "").trimEnd());
}

export interface JobPaths {
  dir: string;
  log: string;
  pid: string;
  exit: string;
  cmd: string;
}

export function jobDir(root: string, id: string): JobPaths {
  const dir = join(root, id);
  return {
    dir,
    log: join(dir, "out.log"),
    pid: join(dir, "pid"),
    exit: join(dir, "exit"),
    cmd: join(dir, "cmd"),
  };
}

/** The dispatch wrapper: forks the payload detached, everything lands in
 *  the job dir. The OUTER command returns in ~50 ms with a pointer line. */
export function rewriteToDispatch(command: string, paths: JobPaths, id: string): string {
  const inner = `cd "$PWD"; ${command}`;
  return [
    `mkdir -p '${paths.dir}'`,
    `printf '%s' ${shellQuote(command)} > '${paths.cmd}'`,
    `setsid bash -c ${shellQuote(inner)} > '${paths.log}' 2>&1 &`,
    `echo $! > '${paths.pid}'`,
    `echo "longjob ${id} dispatched · log: ${paths.log} (poll bg_status ${id}, or bg_wait ${id} <max_s>; it will be announced on completion)"`,
  ].join("; ");
}

/** POSIX single-quote (same contract as permissions-guard/sandbox). */
export function shellQuote(s: string): string {
  return `'${String(s).replace(/'/g, `'\\''`)}'`;
}

export interface JobSnapshot {
  id: string;
  running: boolean;
  exit: number | null;
  tail: string;
  announced: boolean;
}

export interface SnapshotDeps {
  readFile?: (p: string) => string;
  tailLines?: (p: string, n: number) => string;
  readdir?: (p: string) => string[];
}

/** Read one job's snapshot. Missing exit file = still running (I2: the
 *  watcher never kills). */
export function snapshotJob(
  root: string,
  id: string,
  deps: SnapshotDeps = {},
): JobSnapshot | null {
  const paths = jobDir(root, id);
  const read = deps.readFile ?? ((p: string) => readFileSync(p, "utf8"));
  let exit: number | null = null;
  let announced = false;
  try {
    const raw = read(paths.exit).trim();
    // exit file holds "exit=0" or "exit=1"; announced holds "1"
    exit = Number(raw.replace(/^exit=/, ""));
    if (!Number.isFinite(exit)) exit = null;
  } catch {
    exit = null;
  }
  let tail = "";
  try {
    const tailFn =
      deps.tailLines ??
      ((p: string, n: number) => readFileSync(p, "utf8").split("\n").slice(-n).join("\n"));
    tail = tailFn(paths.log, 40);
  } catch {
    tail = "(no output yet)";
  }
  try {
    announced = read(join(paths.dir, "announced")).trim() === "1";
  } catch {
    announced = false;
  }
  return { id, running: exit === null, exit, tail, announced };
}

/** Finished but not yet announced → announce exactly once (I5). */
export function jobsToAnnounce(root: string, deps: SnapshotDeps = {}): string[] {
  const list = deps.readdir ?? ((p: string) => readdirSync(p));
  let ids: string[];
  try {
    ids = list(root);
  } catch {
    return [];
  }
  const out: string[] = [];
  for (const id of ids.sort()) {
    const snap = snapshotJob(root, id, deps);
    if (snap && !snap.running && !snap.announced) out.push(id);
  }
  return out;
}
