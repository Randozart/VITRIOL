import { appendFileSync, closeSync, fstatSync, mkdirSync, openSync, readSync } from "node:fs";
import { dirname } from "node:path";
import { keyOf, type ModelKey } from "./catalog.ts";

const WINDOW = 64 * 1024;
const LIMIT = 16;

function readTail(path: string): string[] {
  let fd: number;
  try { fd = openSync(path, "r"); } catch { return []; }
  try {
    const size = fstatSync(fd).size;
    const offset = Math.max(0, size - WINDOW);
    const data = Buffer.alloc(size - offset);
    const bytes = readSync(fd, data, 0, data.length, offset);
    const lines = data.toString("utf8", 0, bytes).split("\n");
    return offset === 0 ? lines : lines.slice(1);
  } catch {
    return [];
  } finally {
    closeSync(fd);
  }
}

function parseModel(line: string): ModelKey | undefined {
  try {
    const parsed: unknown = JSON.parse(line);
    if (!Array.isArray(parsed) || parsed.length !== 2) return;
    const [provider, id] = parsed;
    if (typeof provider === "string" && typeof id === "string" && provider && id) return { provider, id };
  } catch {
    // Concurrent or interrupted appends can leave a partial record.
  }
}

export function recentModels(path: string): ModelKey[] {
  const unique = new Map<string, ModelKey>();
  for (const line of readTail(path).reverse()) {
    const model = parseModel(line);
    if (!model || unique.has(keyOf(model))) continue;
    unique.set(keyOf(model), model);
    if (unique.size === LIMIT) break;
  }
  return [...unique.values()];
}

export function rememberModel(path: string, model: ModelKey): void {
  const key = keyOf(model);
  const previous = recentModels(path)[0];
  if (previous && keyOf(previous) === key) return;
  try {
    mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
    appendFileSync(path, key + "\n", { mode: 0o600 });
  } catch {
    // An unavailable history file must not prevent switching models.
  }
}
