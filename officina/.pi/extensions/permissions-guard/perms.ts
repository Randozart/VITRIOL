// permissions-guard — pure policy pieces (Round 4 gap D1, DAILY-DRIVER-GAP).
//
// Policy lives in the unified config (single source); this side reads a
// JSON SNAPSHOT (permissions.json) emitted by `trismegistus perms-sync`, so
// the node runtime needs no YAML parser and drift is checkable by hash.
// Matcher semantics: ordered rules, FIRST MATCH WINS; glob: ** crosses
// directories, * does not, ? is one char; patterns starting with ** or bare
// names match against the ABSOLUTE path, relative patterns against cwd-
// relative; a path escaping cwd (../) is matched on the absolute form only.

export type PermAction = "allow" | "deny" | "ask";

export interface PermRule {
  tool: string; // exact tool name (lowercase) — "edit" | "write" | "read"
  pattern: string; // glob
  action: PermAction;
}

export interface PermSnapshot {
  default_action: PermAction;
  rules: PermRule[];
  source_hash: string;
}

export interface Verdict {
  action: PermAction;
  ruleIndex: number; // -1 = default
  pattern?: string;
}

type GlobToken = { kind: "dstar" | "star" | "quest" | "lit"; value: string };

const GLOB_PIECES = /\*\*\/|\*\*|\*|\?|[^*?]+/g;
const TOKEN_PATTERNS: Record<string, GlobToken> = {
  "**/": { kind: "dstar", value: "(?:.*/)?" },
  "**": { kind: "dstar", value: ".*" },
  "*": { kind: "star", value: "[^/]*" },
  "?": { kind: "quest", value: "[^/]" },
};

/** Tokenize the glob subset via one alternation split (no branching maze). */
export function tokenizeGlob(glob: string): GlobToken[] {
  return (glob.match(GLOB_PIECES) ?? []).map(
    (t) => TOKEN_PATTERNS[t] ?? { kind: "lit", value: t.replace(/[.+^${}()|[\]\\]/g, "\\$&") },
  );
}

/** Compile our glob subset to an anchored regex. */
export function globToRegExp(glob: string): RegExp {
  return new RegExp(`^${tokenizeGlob(glob).map((t) => t.value).join("")}$`);
}

function normalize(cwd: string, path: string): { rel: string; abs: string } {
  const root = cwd.replace(/\/+$/, "");
  const abs = path.startsWith("/") ? path : `${root}/${path}`;
  const rel = abs.startsWith(root + "/") ? abs.slice(root.length + 1) : abs; // outside cwd: match on abs only
  return { rel, abs };
}

/** True when rule pattern matches the path (both forms considered). */
export function matches(pattern: string, cwd: string, path: string): boolean {
  const { rel, abs } = normalize(cwd, path);
  const rx = globToRegExp(pattern);
  if (rx.test(rel)) return true;
  if (rel !== abs && rx.test(abs)) return true;
  return false;
}

/** First-match-wins decision. Pure, total (never throws). */
export function decide(snapshot: PermSnapshot, tool: string, path: string, cwd: string): Verdict {
  const t = tool.toLowerCase();
  for (let i = 0; i < snapshot.rules.length; i++) {
    const r = snapshot.rules[i];
    if (r.tool !== t) continue;
    if (matches(r.pattern, cwd, path)) return { action: r.action, ruleIndex: i, pattern: r.pattern };
  }
  return { action: snapshot.default_action, ruleIndex: -1 };
}

/** Parse the snapshot JSON; null on any problem (caller: allow-all + WARN). */
export function parseSnapshot(text: string): PermSnapshot | null {
  try {
    const raw = JSON.parse(text) as Partial<PermSnapshot>;
    if (!Array.isArray(raw.rules) || typeof raw.default_action !== "string") return null;
    const ok = new Set(["allow", "deny", "ask"]);
    if (!ok.has(raw.default_action)) return null;
    const rules: PermRule[] = [];
    for (const r of raw.rules) {
      if (typeof r?.tool !== "string" || typeof r?.pattern !== "string" || !ok.has(r.action)) return null;
      rules.push({ tool: r.tool.toLowerCase(), pattern: r.pattern, action: r.action as PermAction });
    }
    return { default_action: raw.default_action as PermAction, rules, source_hash: String(raw.source_hash ?? "") };
  } catch {
    return null;
  }
}

/** Extract the target path from edit/write/read tool input (pi shapes). */
export function pathOf(input: Record<string, unknown> | undefined): string | null {
  if (!input) return null;
  for (const key of ["path", "file_path", "file"]) {
    const v = input[key];
    if (typeof v === "string" && v) return v;
  }
  return null;
}
