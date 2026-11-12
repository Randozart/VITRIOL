// github-tools extension (exec-infra stage 6). Registers the read-only gh
// tools when — and only when — origin is a GitHub remote (I6: view-only).
// The pi tool surface uses TypeBox like every other extension here; gh.ts
// stays dependency-free for unit tests.

import { spawnSync } from "node:child_process";
import { Type } from "@sinclair/typebox";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { GH_TOOLS, makeRunner, repoSlug } from "./gh.ts";

function originSlug(cwd: string): string | null {
  try {
    const res = spawnSync("git", ["remote", "get-url", "origin"], {
      cwd,
      encoding: "utf8",
      timeout: 5_000,
    });
    if (res.status !== 0) return null;
    return repoSlug(String(res.stdout ?? ""));
  } catch {
    return null;
  }
}

export default function (pi: ExtensionAPI) {
  // Registered at extension load; ctx.cwd arrives per execute call.
  const runner = makeRunner();
  for (const tool of GH_TOOLS) {
    pi.registerTool({
      name: tool.name,
      label: tool.label,
      description: `${tool.description} Repo: resolved from git remote origin at call time.`,
      parameters: Type.Object(
        Object.fromEntries(
          Object.entries((tool.parameters as { properties: Record<string, unknown> }).properties).map(
            ([k, v]) => [k, v as never],
          ),
        ),
      ),
      async execute(_id, input, ctx) {
        const cwd = String((ctx as { cwd?: string }).cwd ?? process.cwd());
        const slug = originSlug(cwd);
        if (!slug) {
          return {
            content: [{ type: "text" as const, text: "github-tools: origin is not a GitHub remote" }],
            details: {},
          };
        }
        const result = tool.execute(input as Record<string, unknown>, runner, cwd);
        void slug;
        return {
          content: [{ type: "text" as const, text: result.text }],
          details: {},
        };
      },
    });
  }
}
