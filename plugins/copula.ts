// Copula Hermetis — OpenCode plugin connecting OpenCode into VITRIOL's Hermetis memory.
// Copula = the VITRIOL<->OpenCode bond; Hermetis = the memory system.
// Rolling window over a database (2026-08-06):
//   ingest:  per-message + chat.message (full user turns) + tool results
//   lossless: experimental.session.compacting dumps pre-compaction context
//   auto-inject: on new user message, retrieve /hermetis/context and inject as
//                [Hermetis context] via session.prompt({noReply}) — COPULA_AUTO_CONTEXT
// Retrieve: `memory_search` custom tool -> Hermetis /hermetis/search.
import type { Plugin } from "@opencode-ai/plugin"
import { tool } from "@opencode-ai/plugin"

const HERMETIS_URL = process.env.COPULA_HERMETIS_URL ?? "http://127.0.0.1:8090"
const MAX_CONTENT = 20000
const AUTO_CONTEXT = process.env.COPULA_AUTO_CONTEXT !== "0"
const CONTEXT_BUDGET = Number(process.env.COPULA_CONTEXT_BUDGET ?? 3000)
const CONTEXT_TOP_K = Number(process.env.COPULA_CONTEXT_TOP_K ?? 5)

function hashString(s: string): string {
  let h = 5381
  for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0
  return String(h)
}

export const CopulaHermetis: Plugin = async ({ project, client, directory, worktree }) => {
  // Master toggle: COPULA_ENABLED=0 disables the plugin entirely (no-op hooks,
  // zero network calls, zero injection) — for when VITRIOL/Hermetis isn't running.
  if (process.env.COPULA_ENABLED === "0") {
    return {}
  }
  const projectRoot = worktree ?? directory
  const projectId = projectRoot ?? project?.id ?? "default"

  // Dedupe ingested parts so streaming + transcript pulls don't double-store.
  const stored = new Set<string>()
  // Dedupe auto-injected context blocks (rolling window, B).
  const injected = new Set<string>()
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

  async function store(role: string, content: string, sessionId: string, maxChars = MAX_CONTENT): Promise<void> {
    if (!content?.trim()) return
    await post("/hermetis/store", {
      project_id: projectId,
      session_id: sessionId,
      role,
      content: content.slice(0, maxChars),
    })
  }

  async function storeTextPart(part: any, sessionId: string): Promise<void> {
    const key = `${sessionId}:${part?.messageID}:${part?.id}`
    if (stored.has(key)) return
    stored.add(key)
    if (part?.type === "text" && typeof part.text === "string") {
      // Skip the plugin's own labels (injected context + compaction capture) so they
      // are never re-ingested as conversation.
      if (part.text.startsWith("[Hermetis context]") || part.text.startsWith("[compaction capture]")) return
      // synthetic text parts carry tool/tool-output content; plain text is assistant prose.
      const role = part.synthetic ? "tool" : "assistant"
      await store(role, part.text, sessionId)
    }
  }

  // Rolling window (B): on a new user turn, retrieve relevant memory and inject it as a
  // labeled noReply context part so the window is reassembled from what matters.
  async function injectContext(sessionId: string, query: string): Promise<void> {
    try {
      const res = await fetch(`${HERMETIS_URL}/hermetis/context`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          project_id: projectId,
          recent_text: query.slice(0, 2000),
          budget_tokens: CONTEXT_BUDGET,
          top_k: CONTEXT_TOP_K,
        }),
      })
      if (!res.ok) return
      const data = await res.json()
      const block = data?.context
      if (!block?.trim()) return
      const h = hashString(block)
      if (injected.has(h)) return
      injected.add(h)
      const labeled = `[Hermetis context]\n${block}`
      await client.session.prompt({
        body: { noReply: true, parts: [{ type: "text", text: labeled }] },
        path: { id: sessionId },
      })
    } catch {}
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

    // Rolling window (A + B): full user-turn capture + per-turn auto-injection.
    "chat.message": async (input, output) => {
      try {
        const message = (output as any)?.message
        const role = message?.role === "assistant" ? "assistant" : "user"
        const text = typeof message?.content === "string" ? message.content : ""
        const sessionId = (input as any)?.sessionID ?? "default"
        if (text?.trim() && role === "user") {
          await store("user", text, sessionId)
          if (AUTO_CONTEXT) await injectContext(sessionId, text)
        }
      } catch {}
    },

    // Rolling window (A): lossless compaction capture — dump the pre-compaction
    // context strings to Hermetis before the window is replaced.
    "experimental.session.compacting": async (input, output) => {
      try {
        const sessionId = (input as any)?.sessionID ?? "default"
        const context = (output as any)?.context
        if (Array.isArray(context) && context.length) {
          await store("tool", `[compaction capture]\n${context.join("\n")}`, sessionId, 500000)
        }
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
