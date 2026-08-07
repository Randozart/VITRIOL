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
import * as fs from "node:fs"
import * as os from "node:os"
import * as path from "node:path"

const HERMETIS_URL = process.env.COPULA_HERMETIS_URL ?? "http://127.0.0.1:7980"
const MAX_CONTENT = 20000
const AUTO_CONTEXT = process.env.COPULA_AUTO_CONTEXT !== "0"
const CONTEXT_BUDGET = Number(process.env.COPULA_CONTEXT_BUDGET ?? 1500)
const CONTEXT_TOP_K = Number(process.env.COPULA_CONTEXT_TOP_K ?? 5)
const CONTEXT_MIN_SCORE = Number(process.env.COPULA_CONTEXT_MIN_SCORE ?? 0.3)

// Read Ascensus secrets managed by vitriol-tui at ~/.vitriol/secrets (0600).
// The TUI writes [ascensus] api_key/model there; env vars remain the override.
function readSecrets(): { apiKey: string; model: string } {
  try {
    const p = path.join(os.homedir(), ".vitriol", "secrets")
    const text = fs.readFileSync(p, "utf8")
    let apiKey = ""
    let model = ""
    for (const line of text.split("\n")) {
      const t = line.trim()
      if (!t || t.startsWith("#") || t.startsWith("[")) continue
      const i = t.indexOf("=")
      if (i < 0) continue
      const k = t.slice(0, i).trim()
      const v = t.slice(i + 1).trim()
      if (k === "api_key") apiKey = v
      else if (k === "model") model = v
    }
    return { apiKey, model }
  } catch {
    return { apiKey: "", model: "" }
  }
}

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
  // Sessions that already received the Pymander doctrine block (inject once).
  const doctrineGiven = new Set<string>()
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
      if (part.text.startsWith("[Hermetis context]") || part.text.startsWith("[compaction capture]") || part.text.startsWith("[Pymander doctrine]")) return
      // synthetic text parts carry tool/tool-output content; plain text is assistant prose.
      const role = part.synthetic ? "tool" : "assistant"
      await store(role, part.text, sessionId)
    }
  }

  // Rolling window (B): on a new user turn, selectively retrieve relevant memory and
  // inject it as a labeled noReply context part. Selective: skips when nothing is
  // relevant enough (min_score) or when the turn is a continuation of the current
  // window (is_new_topic=false) — avoids re-injecting what the window already carries.
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
          min_score: CONTEXT_MIN_SCORE,
          session_id: sessionId,
        }),
      })
      if (!res.ok) return
      const data = await res.json()
      const block = data?.context
      const topScore = Number(data?.top_score ?? 0)
      const isNewTopic = data?.is_new_topic !== false
      if (!block?.trim()) return
      if (topScore < CONTEXT_MIN_SCORE) return
      if (!isNewTopic) return
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

  // Doctrine (Pymander, static *how*) injected once per session start, bounded,
  // labeled so it is never re-ingested. Uses the project's selected domains.
  async function injectDoctrine(sessionId: string): Promise<void> {
    if (doctrineGiven.has(sessionId)) return
    try {
      const res = await fetch(`${HERMETIS_URL}/pymander/context`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          project_id: projectId,
          budget_tokens: CONTEXT_BUDGET,
          top_k: CONTEXT_TOP_K,
        }),
      })
      if (!res.ok) return
      const data = await res.json()
      const block: string = data?.context ?? ""
      if (!block?.trim()) return
      doctrineGiven.add(sessionId)
      const labeled = `[Pymander doctrine]\n${block}`
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
          await injectDoctrine(sessionId)
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
      pymander_search: tool({
        description:
          "Search Pymander, the curated reference mind (how this project does a domain well). Static hand-authored atomic nodes, distinct from episodic Hermetis memory. Use for a domain pattern/convention; pass a domain (or omit and let it fall back to the project's selected domains).",
        args: {
          domain: tool.schema.string().optional().describe("Pymander domain to search (e.g. systems). Omit to use the project's selected domains."),
          query: tool.schema.string().describe("The search query"),
          top_k: tool.schema.number().optional().describe("Number of results (default 3)"),
        },
        async execute(args, _context) {
          try {
            const domains = args.domain
              ? [args.domain]
              : await getSelectedDomains(projectId)
            const out: string[] = []
            for (const d of domains) {
              const res = await fetch(`${HERMETIS_URL}/pymander/search`, {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ domain: d, query: args.query, top_k: args.top_k ?? 3 }),
              })
              if (!res.ok) continue
              const data = await res.json()
              for (const r of (data?.results ?? [])) {
                out.push(`[${d} ${r.score}] ${r.label}: ${r.summary}`)
              }
            }
            if (!out.length) return "No Pymander doctrine found for that query."
            return out.join("\n\n")
          } catch {
            return "Pymander search failed (service unreachable)."
          }
        },
      }),
      ascensus: tool({
        description:
          "Escalate a genuinely-hard inquiry to a configured cloud model (Google Gemini) that takes over the wheel. Use only when the question is beyond your reliable local ability or needs a second opinion. The query and your reasoning attempt are sent; no file contents or secrets leave the machine. Escalations are stored to memory so the system learns and self-reduces future escalation.",
        args: {
          query: tool.schema.string().describe("The user's hard inquiry, as-is."),
          reasoning: tool.schema.string().optional().describe("Your local reasoning attempt, so the cloud model can improve on it."),
        },
        async execute(args, _context) {
          const secrets = readSecrets()
          const key = process.env.GEMINI_API_KEY || secrets.apiKey
          if (!key) {
            return "Ascensus not configured: no GEMINI_API_KEY in env or ~/.vitriol/secrets. Set it in the SUBSYSTEMS tab, or export GEMINI_API_KEY. Reply locally instead."
          }
          try {
            const model = process.env.GEMINI_MODEL || secrets.model || "gemini-2.5-flash"
            const maxTokens = Number(process.env.GEMINI_MAX_TOKENS ?? 2048)
            const payload = {
              contents: [{
                parts: [{ text: args.reasoning
                  ? `User inquiry: ${args.query}\n\nLocal reasoning attempt:\n${args.reasoning}`
                  : `User inquiry: ${args.query}` }],
              }],
              generationConfig: { maxOutputTokens: maxTokens },
            }
            const res = await fetch(
              `https://generativelanguage.googleapis.com/v1beta/models/${model}:generateContent?key=${key}`,
              { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(payload) },
            )
            if (!res.ok) {
              const err = await res.text()
              return `Ascensus call failed (HTTP ${res.status}). ${err.slice(0, 300)}`
            }
            const data = await res.json()
            const answer = data?.candidates?.[0]?.content?.parts?.map((p: any) => p.text).join("\n")
            if (!answer) {
              return "Ascensus returned no text. Reply locally instead."
            }
            // Learning loop: store the escalation so Hermetis can learn from it.
            await store("tool", `[ascensus] model=${model}\n${args.query}\n→\n${answer.slice(0, 8000)}`, "default")
            return `[Ascensus — cloud answer from ${model}]\n${answer}`
          } catch (e) {
            return `Ascensus call failed: ${e instanceof Error ? e.message : String(e)}`
          }
        },
      }),
    },
  }
}

// Pymander: read the project's selected domains from selection.json via /pymander/context.
async function getSelectedDomains(projectId: string): Promise<string[]> {
  try {
    const res = await fetch(`${HERMETIS_URL}/pymander/context`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ project_id: projectId, budget_tokens: 1, top_k: 1 }),
    })
    if (!res.ok) return []
    const data = await res.json()
    // context is empty when nothing selected; parse the ## domain markers.
    const ctx: string = data?.context ?? ""
    return ctx
      .split("\n")
      .filter((l) => l.startsWith("## "))
      .map((l) => l.slice(3).trim())
  } catch {
    return []
  }
}
