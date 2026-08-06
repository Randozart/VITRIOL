// Copula Hermetis — OpenCode plugin connecting OpenCode into VITRIOL's Hermetis memory.
// Copula = the VITRIOL<->OpenCode bond; Hermetis = the memory system.
// Ingest: per-message (session.idle transcript) + tool results (tool.execute.after).
// Retrieve: `memory_search` custom tool -> Hermetis /hermetis/search.
import type { Plugin } from "@opencode-ai/plugin"
import { tool } from "@opencode-ai/plugin"

const HERMETIS_URL = process.env.COPULA_HERMETIS_URL ?? "http://127.0.0.1:8090"
const MAX_CONTENT = 20000

export const CopulaHermetis: Plugin = async ({ project, client, directory, worktree }) => {
  const projectRoot = worktree ?? directory
  const projectId = projectRoot ?? project?.id ?? "default"

  // Dedupe ingested parts so streaming + transcript pulls don't double-store.
  const stored = new Set<string>()
  // Debounce repo-map node refresh per changed file (P3.4).
  const fileTimers = new Map<string, ReturnType<typeof setTimeout>>()

  async function post(path: string, body: unknown): Promise<boolean> {
    try {
      const res = await fetch(`${HERMETIS_URL}${path}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      })
      return res.ok
    } catch {
      return false
    }
  }

  async function store(role: string, content: string, sessionId: string): Promise<void> {
    if (!content?.trim()) return
    await post("/hermetis/store", {
      project_id: projectId,
      session_id: sessionId,
      role,
      content: content.slice(0, MAX_CONTENT),
    })
  }

  async function storeTextPart(part: any, sessionId: string): Promise<void> {
    const key = `${sessionId}:${part?.messageID}:${part?.id}`
    if (stored.has(key)) return
    stored.add(key)
    if (part?.type === "text" && typeof part.text === "string") {
      // synthetic text parts carry tool/tool-output content; plain text is assistant prose.
      const role = part.synthetic ? "tool" : "assistant"
      await store(role, part.text, sessionId)
    }
  }

  return {
    event: async ({ event }) => {
      try {
        if (event.type === "message.part.updated") {
          const part = event.properties.part as any
          // Store final text parts (no streaming delta) as they complete.
          if (!event.properties.delta && part?.type === "text") {
            await storeTextPart(part, part.sessionID ?? "default")
          }
        } else if (event.type === "session.idle") {
          // Turn complete: pull the full transcript, store any new user/assistant text.
          const sessionId = event.properties.sessionID
          const res = await client.session.messages({ path: { id: sessionId } })
          const msgs = (res as any).data ?? res ?? []
          for (const m of msgs) {
            for (const p of m?.parts ?? []) {
              await storeTextPart(p, m?.info?.id ?? sessionId)
            }
          }
        } else if (event.type === "file.edited" || event.type === "file.watcher.updated") {
          // File changed: refresh its Hermetis node so the map stays current (P3.4).
          const f = (event.properties as any).file
          if (f && projectRoot) {
            const key = `${projectRoot}:${f}`
            if (fileTimers.has(key)) clearTimeout(fileTimers.get(key)!)
            fileTimers.set(
              key,
              setTimeout(() => {
                fileTimers.delete(key)
                const rel = f.startsWith(projectRoot) ? f.slice(projectRoot.length + 1) : f
                void post("/hermetis/repo_map", {
                  project_id: projectId,
                  root: projectRoot,
                  file: rel,
                  budget_tokens: 200,
                })
              }, 2000),
            )
          }
        }
      } catch {}
    },

    "tool.execute.after": async (input, output) => {
      try {
        const args = JSON.stringify((input as any)?.args ?? {})
        const out = typeof output === "string" ? output : JSON.stringify(output ?? "")
        const content = `${(input as any)?.tool}\nARGS: ${args.slice(0, 2000)}\nRESULT: ${out.slice(0, 8000)}`
        await store("tool", content, "default")
      } catch {}
    },

    tool: {
      memory_search: tool({
        description:
          "Search Hermetis memory (past sessions, decisions, tool results, repo context) for relevant context. Use when a task needs information from earlier in this or previous sessions.",
        args: {
          query: tool.schema.string().describe("The search query"),
          top_k: tool.schema.number().optional().describe("Number of results (default 5)"),
        },
        async execute(args, _context) {
          try {
            const res = await fetch(`${HERMETIS_URL}/hermetis/search`, {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({ project_id: projectId, query: args.query, top_k: args.top_k ?? 5 }),
            })
            if (!res.ok) return `Hermetis search failed (HTTP ${res.status}).`
            const data = await res.json()
            if (!data?.results?.length) return "No memory found for that query."
            return data.results
              .map((r: any) => `[${r.type} score=${r.score}] ${r.content}`)
              .join("\n\n")
          } catch {
            return "Hermetis search failed (service unreachable)."
          }
        },
      }),
    },
  }
}
