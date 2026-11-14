// AGENTS.md awareness (2026-11-14) — injects a compact digest of the project's
// AGENTS.md files and reloads it when they change.
//
// Why: pi does not read AGENTS.md; Officina's own doctrine (invariants, gates,
// conventions) lived only in docs the model never saw. The digest keeps the
// always-on budget honest (headings + bullets, capped) instead of pasting a
// 30 KB file into every session.
//
// Reactive: the file set (paths + mtimes) is checked on every turn boundary;
// a change re-arms the digest for the next request and tells the operator.
//
// Kill switch: OFFICINA_NO_AGENTS_MD=1.
// Tuning: OFFICINA_AGENTS_FILES (colon-separated), OFFICINA_AGENTS_DIGEST_CHARS.

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import {
  agentsSignature,
  buildDigest,
  digestSummary,
  loadAgentsFiles,
  type AgentsFile,
} from "./discovery.ts";

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_NO_AGENTS_MD === "1") return;

  let files: AgentsFile[] = [];
  let signature = "";
  let digest = "";
  let pending = false;
  let lastNotice = "";
  let cwd = process.cwd();

  const reload = (notify: (text: string, kind?: string) => void, reason: string): void => {
    const maxChars = Number(process.env.OFFICINA_AGENTS_DIGEST_CHARS) || 3000;
    files = loadAgentsFiles(cwd);
    const built = buildDigest(files, { maxChars, cwd });
    digest = built.text;
    signature = agentsSignature(files);
    pending = digest.length > 0;
    const summary = digestSummary(files, built.rules, digest.length);
    if (summary !== lastNotice) {
      lastNotice = summary;
      notify(summary, "info");
    }
    emitHarnessEvent(
      harnessEvent("lc-agents", reason, {
        detail: `${files.length} file(s), ${built.rules} rule(s), ${digest.length} chars`,
      }),
    );
  };

  pi.on("session_start", (_event, ctx) => {
    const sessionCwd = (ctx as { cwd?: string }).cwd;
    if (sessionCwd) cwd = sessionCwd;
    reload((text, kind) => ctx.ui?.notify?.(text, kind as never), "loaded");
  });

  // Cheap reactivity: stat-based signature on every turn boundary.
  pi.on("turn_start", (_event, ctx) => {
    const current = agentsSignature(loadAgentsFiles(cwd));
    if (current === signature) return;
    reload((text, kind) => ctx.ui?.notify?.(text, kind as never), "changed");
  });

  pi.on("context", (event) => {
    if (!pending || !digest) return undefined;
    pending = false;
    return {
      messages: [
        ...event.messages,
        {
          role: "custom" as const,
          customType: "agents-md",
          content: `\n\n${digest}`,
          display: false,
          details: {},
          timestamp: Date.now(),
        },
      ],
    } as never;
  });

  pi.registerCommand("agents-md", {
    description: "Show how Officina sees AGENTS.md (files, rule count, digest)",
    handler: async (_args, ctx) => {
      const maxChars = Number(process.env.OFFICINA_AGENTS_DIGEST_CHARS) || 3000;
      files = loadAgentsFiles(cwd);
      const built = buildDigest(files, { maxChars, cwd });
      const lines = [
        digestSummary(files, built.rules, built.text.length),
        ...files.map((f) => `- ${f.path}`),
        "",
        built.text || "(no AGENTS.md found)",
      ];
      ctx.ui?.notify?.(lines.join("\n"), "info");
    },
  });
}
