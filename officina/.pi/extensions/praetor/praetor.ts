// Pure Praetor helpers: config, JSON diagnostics parsing, bounded rendering.

export interface PraetorConfig {
  enabled: boolean;
  budgetChars: number;
  timeoutMs: number;
  instructions: boolean;
}

export interface PraetorDiagnostic {
  file: string;
  line: number;
  source: string;
  message: string;
}

export function praetorConfig(env: NodeJS.ProcessEnv = process.env): PraetorConfig {
  const enabled =
    env.OFFICINA_NO_PRAETOR !== "1" &&
    (env.OFFICINA_PRAETOR === "1" || env.TRIS_DIAG_PRAETOR === "1");
  const budget = Number(env.OFFICINA_PRAETOR_BUDGET ?? 300);
  const timeout = Number(env.OFFICINA_PRAETOR_TIMEOUT_MS ?? 60_000);
  return {
    enabled,
    budgetChars: Number.isFinite(budget) ? Math.max(100, Math.min(4_000, Math.round(budget))) : 300,
    timeoutMs: Number.isFinite(timeout) ? Math.max(5_000, Math.min(300_000, Math.round(timeout))) : 60_000,
    instructions: env.OFFICINA_PRAETOR_INSTRUCTIONS !== "0",
  };
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

export function parsePraetorJson(text: string): PraetorDiagnostic[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(String(text ?? "").trim());
  } catch {
    return [];
  }
  const failures = record(parsed).failures;
  if (!Array.isArray(failures)) return [];
  const out: PraetorDiagnostic[] = [];
  for (const raw of failures) {
    const f = record(raw);
    const message = typeof f.message === "string" ? f.message : "";
    if (!message) continue;
    out.push({
      file: typeof f.file === "string" ? f.file : "?",
      line: typeof f.line === "number" ? f.line : 0,
      source: typeof f.source === "string" ? f.source : "praetor",
      message,
    });
  }
  return out;
}

export function renderPraetorDiagnostics(
  diagnostics: PraetorDiagnostic[],
  budgetChars = 300,
): string {
  if (diagnostics.length === 0) return "";
  const head = `[praetor: ${diagnostics.length} diagnostic(s) after your edit — advisory]`;
  const body: string[] = [];
  for (const d of diagnostics) {
    const line = `- ${d.file}:${d.line} | ${d.source} | ${d.message}`;
    if (head.length + body.join("\n").length + line.length + 2 > budgetChars) break;
    body.push(line);
  }
  if (body.length === 0) return head.slice(0, budgetChars);
  const text = `${head}\n${body.join("\n")}`;
  return text.length > budgetChars ? `${text.slice(0, Math.max(0, budgetChars - 1))}…` : text;
}

export function isPraetorSourceFile(file: string): boolean {
  return /\.(ts|tsx|js|jsx|py|rs|c|h|cc|cpp|hpp|go|rb)$/.test(file);
}
