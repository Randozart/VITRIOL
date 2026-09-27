// PROVENANCE: original — VITRIOL officina (2026-09-27, parity plan phase 1.4,
// .opencode/plans/officina-parity-2026-09-27.md). No third-party code.
//
// init — one-shot project bootstrap. `/init` scans the working tree with
// plain filesystem calls (build-manifest detection, directory shape, language
// census) and seeds .officina/MEMORY.md so the memory extension injects real
// orientation from the next turn on. Deliberately NOT an agent turn: the
// scan is deterministic and costs zero tokens (deviation from the plan's
// "reuse repo-map" note — the tree-sitter shim needs a 60s-ready subprocess,
// which is the wrong dependency for a cold-start command; recorded in the
// plan doc). No-op with confirm when MEMORY.md already exists.

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { buildMemorySeed, scanProject } from "./scan.ts";

export default function (pi: ExtensionAPI) {
  pi.registerCommand("init", {
    description: "Scan the project and seed .officina/MEMORY.md (orientation for future turns)",
    handler: async (_args: string, ctx: { ui: any; cwd?: string }) => {
      const cwd = ctx.cwd ?? process.cwd();
      const memoryPath = `${cwd}/.officina/MEMORY.md`;

      const { existsSync, mkdirSync, writeFileSync } = await import("node:fs");
      if (existsSync(memoryPath)) {
        const ok = await ctx.ui
          .confirm?.(`${memoryPath} already exists. Overwrite with a fresh scan?`)
          .catch(() => false);
        if (!ok) {
          await ctx.ui.notify?.("init: kept existing MEMORY.md", "info");
          return;
        }
      }

      const scan = scanProject(cwd);
      const seed = buildMemorySeed(scan, new Date().toISOString().slice(0, 16).replace("T", " "));
      mkdirSync(`${cwd}/.officina`, { recursive: true });
      writeFileSync(memoryPath, seed);

      await ctx.ui.notify?.(
        `init: seeded .officina/MEMORY.md (${scan.languages.length} languages, ${scan.buildFiles.length} build files) — edit it to correct anything wrong`,
        "info",
      );
    },
  });
}
