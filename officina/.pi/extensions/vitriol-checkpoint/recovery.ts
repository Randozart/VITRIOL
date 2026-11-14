import { mkdirSync, mkdtempSync, readFileSync, renameSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import type { SlotOpResult } from "./client.ts";

/** Only checkpoint-specific rejections suspend automatic restore; outages remain retryable. */
export function shouldQuarantineRestore(result: SlotOpResult): boolean {
  if (result.ok || ![400, 404, 422].includes(result.status)) return false;
  return /invalid slot save file|invalid tokens in slot save file|restored prompt does not fit|no such file or directory|(?:checkpoint|slot (?:save |state )?file).*(?:not found|missing|invalid|corrupt)/i.test(result.error ?? "");
}

/** Preserve a rejected marker under a unique directory without extending a long basename. */
export function quarantineRecoveryMarker(path: string, expected: string): string | undefined {
  try {
    if (readFileSync(path, "utf8") !== expected) return undefined;
    const root = join(dirname(path), "failed-restores");
    mkdirSync(root, { recursive: true });
    const backup = join(mkdtempSync(join(root, "restore-")), basename(path));
    renameSync(path, backup);
    return backup;
  } catch {
    // A failed rename leaves the active marker available for recovery.
    return undefined;
  }
}
