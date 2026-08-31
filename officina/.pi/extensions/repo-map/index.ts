import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import { RepoMapClient, capText, repoMapConfig } from "./client.ts";

// Repo map (PLAN.md Enhancement 0 / REPORT-02 step 5, technique from Aider):
// tree-sitter symbol graph + PageRank so the model knows WHERE code lives
// before spending tokens reading files. ~1K structural overview replaces 5-10K
// of blind reads. Upstream repo-map is IMPORTED by shim.py, never patched.
//
// Kill switch: TRIS_NO_REPO_MAP=1 (registers nothing) — mirrored at
// ~/.config/trismegistus/config.yaml injection.repo_map.enabled.
// Budget: outputs capped at maxOutputChars (~500 tok) client-side.

export default function (pi: ExtensionAPI) {
  const cfg = repoMapConfig();
  if (!cfg.enabled) return;
  const client = new RepoMapClient(cfg);
  const cwd = () => process.cwd();

  /** Run a shim command with the entry-side cap applied. */
  async function ask(cmd: string, args: Record<string, unknown>) {
    const text = await client.request(cmd, args);
    return { content: [{ type: "text" as const, text: capText(text, cfg.maxOutputChars) }], details: {} };
  }

  /** Ensure the target repo is indexed before a read call. */
  async function indexed(repo: string | undefined) {
    const target = repo || cwd();
    await client.ensureIndexed(target);
    return target;
  }

  pi.registerTool({
    name: "repomap_index",
    label: "RepoMap Index",
    description:
      "(Re)target the repo map on a project path and build the symbol graph (tree-sitter + PageRank). " +
      "Returns file/definition/edge counts. Cheap on re-run: unchanged files come from the cache.",
    parameters: Type.Object({
      path: Type.Optional(Type.String({ description: "Project root (default: current directory)" })),
    }),
    async execute(_id, { path }) {
      try {
        return await ask("index", { path: path || cwd() });
      } catch (e) {
        return { content: [{ type: "text", text: `repo-map: ${(e as Error).message}` }], details: {}, isError: true };
      }
    },
  });

  pi.registerTool({
    name: "repomap_where_is",
    label: "RepoMap WhereIs",
    description: "Find where a symbol is defined, ranked by contextual PageRank. Use BEFORE reading files.",
    parameters: Type.Object({
      query: Type.String({ description: "Symbol name or substring" }),
      repo: Type.Optional(Type.String({ description: "Target repo root (default: indexed cwd)" })),
    }),
    async execute(_id, { query, repo }) {
      try {
        const target = await indexed(repo);
        return await ask("where_is", { query, repo: target });
      } catch (e) {
        return { content: [{ type: "text", text: `repo-map: ${(e as Error).message}` }], details: {}, isError: true };
      }
    },
  });

  pi.registerTool({
    name: "repomap_outline",
    label: "RepoMap Outline",
    description: "Table of contents of a file: signatures + line ranges. ~95% fewer tokens than reading it.",
    parameters: Type.Object({
      file: Type.String({ description: "Path relative to the repo root" }),
      repo: Type.Optional(Type.String({ description: "Target repo root (default: indexed cwd)" })),
    }),
    async execute(_id, { file, repo }) {
      try {
        const target = await indexed(repo);
        return await ask("outline", { file, repo: target });
      } catch (e) {
        return { content: [{ type: "text", text: `repo-map: ${(e as Error).message}` }], details: {}, isError: true };
      }
    },
  });

  pi.registerTool({
    name: "repomap_symbol",
    label: "RepoMap Symbol",
    description: "Full body of ONE symbol — the minimum read needed to start editing.",
    parameters: Type.Object({
      file: Type.String({ description: "Path relative to the repo root" }),
      name: Type.String({ description: "Symbol name" }),
      repo: Type.Optional(Type.String({ description: "Target repo root (default: indexed cwd)" })),
    }),
    async execute(_id, { file, name, repo }) {
      try {
        const target = await indexed(repo);
        return await ask("get_symbol", { file, name, repo: target });
      } catch (e) {
        return { content: [{ type: "text", text: `repo-map: ${(e as Error).message}` }], details: {}, isError: true };
      }
    },
  });

  pi.registerTool({
    name: "repomap_refs",
    label: "RepoMap Refs",
    description: "Impact check: who references a symbol (what a signature change might break).",
    parameters: Type.Object({
      name: Type.String({ description: "Symbol name" }),
      repo: Type.Optional(Type.String({ description: "Target repo root (default: indexed cwd)" })),
    }),
    async execute(_id, { name, repo }) {
      try {
        const target = await indexed(repo);
        return await ask("who_references", { name, repo: target });
      } catch (e) {
        return { content: [{ type: "text", text: `repo-map: ${(e as Error).message}` }], details: {}, isError: true };
      }
    },
  });

  pi.on("session_shutdown", async () => {
    client.shutdown();
  });
}
