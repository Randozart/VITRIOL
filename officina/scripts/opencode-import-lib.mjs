// OpenCode -> pi/Officina session import helpers.
//
// Reads OpenCode's SQLite store read-only, normalizes messages/parts into a
// compact transcript, and writes a pi session JSONL v3 file. No writes to
// OpenCode state.

import { randomUUID } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, readFileSync, realpathSync, renameSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { basename, join, resolve } from "node:path";
import { DatabaseSync } from "node:sqlite";

export const DEFAULT_OPTIONS = {
  full: false,
  // A manageable window by default (2026-11-14, operator report: importing
  // one session pulled in the full ~4M-token history and instantly blew the
  // context). The newest ~40k tokens are kept verbatim and everything older
  // collapses into a compact digest entry. --full (or --max-chars 0)
  // restores the old "everything" behaviour; --last N narrows further.
  last: 0,
  maxChars: 160_000,
  maxToolChars: 500,
  maxToolInputChars: 200,
  maxReasoningChars: 1_000,
};

export const FULL_OPTIONS = {
  ...DEFAULT_OPTIONS,
  full: true,
  maxChars: 0,
  maxToolChars: 4_000,
  maxToolInputChars: 4_000,
  maxReasoningChars: 4_000,
};

function parseJson(value) {
  if (typeof value !== "string") return value ?? null;
  try { return JSON.parse(value); } catch { return null; }
}

export function parseSessionList(text) {
  const parsed = parseJson(text);
  return Array.isArray(parsed) ? parsed : [];
}

export function normalizeCwd(path) {
  const absolute = resolve(String(path ?? "."));
  try { return realpathSync(absolute); } catch { return absolute; }
}

export function filterSessionsByCwd(sessions, cwd) {
  const target = normalizeCwd(cwd);
  return (Array.isArray(sessions) ? sessions : [])
    .filter((session) => normalizeCwd(session.directory) === target)
    .sort((a, b) => Number(b.updated ?? 0) - Number(a.updated ?? 0));
}

function truncate(text, max) {
  const value = String(text ?? "");
  if (max <= 0 || value.length <= max) return value;
  return `${value.slice(0, max)}\n… [truncated ${value.length - max} chars]`;
}

function truncateDeep(value, max, depth = 0) {
  if (depth > 6) return "[depth truncated]";
  if (typeof value === "string") return truncate(value, max);
  if (Array.isArray(value)) {
    return value.slice(0, 40).map((item) => truncateDeep(item, max, depth + 1));
  }
  if (value && typeof value === "object") {
    const out = {};
    for (const [key, item] of Object.entries(value).slice(0, 60)) {
      out[key] = truncateDeep(item, max, depth + 1);
    }
    return out;
  }
  return value;
}

function partText(parts, type) {
  return (parts ?? [])
    .filter((part) => part?.type === type && typeof part.text === "string")
    .map((part) => part.text)
    .join("\n")
    .trim();
}

function usageOf(message) {
  const tokens = message?.tokens ?? {};
  const cache = tokens.cache ?? {};
  const cost = Number(message?.cost ?? 0);
  return {
    input: Number(tokens.input ?? 0),
    output: Number(tokens.output ?? 0),
    cacheRead: Number(cache.read ?? 0),
    cacheWrite: Number(cache.write ?? 0),
    reasoning: Number(tokens.reasoning ?? 0),
    totalTokens: Number(tokens.total ?? 0),
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: cost },
  };
}

function stopReasonOf(message, hasTools) {
  if (message?.finish === "error") return "error";
  if (message?.finish === "aborted") return "aborted";
  if (hasTools || message?.finish === "tool-calls") return "toolUse";
  return "stop";
}

export function loadSessionFromDb(dbPath, sessionId) {
  if (!existsSync(dbPath)) throw new Error(`OpenCode database not found: ${dbPath}`);
  const db = new DatabaseSync(dbPath, { readOnly: true });
  try {
    const session = db
      .prepare(
        "select id,title,directory,model,time_created,time_updated,cost,tokens_input,tokens_output,tokens_reasoning,tokens_cache_read,tokens_cache_write from session where id = ?",
      )
      .get(sessionId);
    if (!session) return null;

    let messageRows = db
      .prepare("select id,time_created,data from message where session_id = ? order by time_created, id")
      .all(sessionId);
    let partRows = db
      .prepare("select id,message_id,time_created,data from part where session_id = ? order by time_created, id")
      .all(sessionId);

    if (messageRows.length === 0) {
      const eventRows = db
        .prepare("select id,type,seq,time_created,data from session_message where session_id = ? order by seq")
        .all(sessionId);
      messageRows = [];
      partRows = [];
      for (const row of eventRows) {
        const data = parseJson(row.data) ?? {};
        if (data.role === "user" || data.role === "assistant") {
          messageRows.push({ id: row.id, time_created: row.time_created, data: row.data });
        } else if (data.messageID) {
          partRows.push({ id: row.id, message_id: data.messageID, time_created: row.time_created, data: row.data });
        }
      }
    }

    return {
      session: {
        ...session,
        model: parseJson(session.model) ?? null,
      },
      messages: messageRows.map((row) => ({
        id: row.id,
        time_created: Number(row.time_created),
        data: parseJson(row.data) ?? {},
      })),
      parts: partRows.map((row) => ({
        id: row.id,
        message_id: row.message_id,
        time_created: Number(row.time_created),
        data: parseJson(row.data) ?? {},
      })),
    };
  } finally {
    db.close();
  }
}

export function sessionStats(dbPath, sessionId) {
  if (!existsSync(dbPath)) return { messages: 0, parts: 0, bytes: 0 };
  const db = new DatabaseSync(dbPath, { readOnly: true });
  try {
    const messages = db.prepare("select count(*) as n from message where session_id = ?").get(sessionId)?.n ?? 0;
    const parts = db.prepare("select count(*) as n, coalesce(sum(length(data)),0) as bytes from part where session_id = ?").get(sessionId);
    return { messages: Number(messages), parts: Number(parts?.n ?? 0), bytes: Number(parts?.bytes ?? 0) };
  } finally {
    db.close();
  }
}

export function normalizeOpenCodeSession(source, options = {}) {
  const opts = { ...DEFAULT_OPTIONS, ...options };
  const partsByMessage = new Map();
  for (const part of source.parts ?? []) {
    if (!partsByMessage.has(part.message_id)) partsByMessage.set(part.message_id, []);
    partsByMessage.get(part.message_id).push(part);
  }

  const blocks = [];
  let skipped = 0;
  for (const message of source.messages ?? []) {
    const data = message.data ?? {};
    const parts = (partsByMessage.get(message.id) ?? []).map((part) => part.data ?? {});
    if (data.role === "user") {
      const text = partText(parts, "text");
      if (!text) { skipped += 1; continue; }
      blocks.push({ kind: "user", text, timestamp: Number(data.time?.created ?? message.time_created) });
      continue;
    }
    if (data.role !== "assistant") { skipped += 1; continue; }

    const content = [];
    const toolResults = [];
    for (const part of parts) {
      if (part.type === "text" && typeof part.text === "string" && part.text.trim()) {
        content.push({ type: "text", text: part.text });
      } else if (part.type === "reasoning" && typeof part.text === "string" && part.text.trim()) {
        content.push({ type: "thinking", thinking: truncate(part.text, opts.maxReasoningChars) });
      } else if (part.type === "tool") {
        const state = part.state ?? {};
        const callId = String(part.callID ?? part.id ?? randomUUID());
        const toolName = String(part.tool ?? "tool");
        const input = truncateDeep(state.input ?? {}, opts.maxToolInputChars);
        content.push({ type: "toolCall", id: callId, name: toolName, arguments: input });
        const output = typeof state.output === "string" ? state.output : (state.title ?? "");
        toolResults.push({
          toolCallId: callId,
          toolName,
          output: truncate(output, opts.maxToolChars),
          isError: state.status === "error" || state.isError === true,
          timestamp: Number(state.time?.end ?? data.time?.completed ?? message.time_created),
        });
      }
    }
    if (content.length === 0) { skipped += 1; continue; }
    const hasTools = content.some((item) => item.type === "toolCall");
    blocks.push({
      kind: "assistant",
      content,
      toolResults,
      timestamp: Number(data.time?.created ?? message.time_created),
      provider: data.providerID ?? source.session?.model?.providerID ?? "caduceus",
      model: data.modelID ?? source.session?.model?.id ?? "opencode-import",
      usage: usageOf(data),
      stopReason: stopReasonOf(data, hasTools),
      errorMessage: data.errorMessage,
    });
  }

  let selected = opts.last > 0 ? blocks.slice(-opts.last) : blocks;
  // Start the compact window at the user turn that opened it, not mid-tool-loop.
  if (selected.length > 0 && selected[0].kind !== "user") {
    const firstIndex = blocks.length - selected.length;
    let userIndex = firstIndex - 1;
    while (userIndex >= 0 && blocks[userIndex].kind !== "user") userIndex -= 1;
    if (userIndex >= 0) selected = [blocks[userIndex], ...selected];
  }
  let chars = selected.reduce((sum, block) => sum + JSON.stringify(block).length, 0);
  let start = 0;
  if (opts.maxChars > 0) {
    while (start < selected.length - 1 && chars > opts.maxChars) {
      chars -= JSON.stringify(selected[start]).length;
      start += 1;
    }
    // Never open the window mid-tool-loop: advance to the next user turn.
    while (start < selected.length - 1 && selected[start].kind !== "user") start += 1;
    chars = selected
      .slice(start)
      .reduce((sum, block) => sum + JSON.stringify(block).length, 0);
  }

  const firstKept = blocks.length - selected.length + start;
  const omitted = blocks.slice(0, firstKept);
  const kept = selected.slice(start);

  return {
    blocks: kept,
    digest: buildImportDigest(omitted, opts),
    stats: {
      sourceMessages: (source.messages ?? []).length,
      sourceParts: (source.parts ?? []).length,
      blocks: blocks.length,
      selected: kept.length,
      omittedBlocks: omitted.length,
      omittedChars: omitted.reduce((sum, block) => sum + JSON.stringify(block).length, 0),
      skipped,
      chars,
      full: opts.full === true,
    },
  };
}

/**
 * One hidden context entry describing what was dropped by the default cap.
 * Bounded: a header plus the most recent 25 user prompts (oldest first,
 * clipped) so the model knows the arc without carrying the payload.
 */
export function buildImportDigest(omitted, opts = DEFAULT_OPTIONS) {
  if (!Array.isArray(omitted) || omitted.length === 0) return "";
  const chars = omitted.reduce((sum, block) => sum + JSON.stringify(block).length, 0);
  const allUsers = omitted.filter((b) => b.kind === "user");
  const head = allUsers.slice(0, 3);
  const tail = allUsers.length > 25 ? allUsers.slice(-22) : [];
  const lines = [
    `Imported from an OpenCode session: ${omitted.length} earlier message block(s) (~${chars} chars) were omitted to keep the context manageable; the transcript below is the most recent portion.`,
  ];
  if (allUsers.length > 0) {
    lines.push("Earlier user prompts (start of the arc + most recent, truncated):");
    const emit = (u) => {
      const one = String(u.text ?? "").replace(/\s+/g, " ").trim();
      if (one) lines.push(`- ${one.length > 120 ? `${one.slice(0, 119)}…` : one}`);
    };
    head.forEach(emit);
    if (tail.length > 0) {
      lines.push("  …");
      tail.forEach(emit);
    } else {
      allUsers.slice(head.length).forEach(emit);
    }
  }
  lines.push(
    "If the omitted history matters, ask the operator to re-import with `/import-opencode <id> full` (CLI: `--full`), or narrow with `--last N`.",
  );
  return lines.join("\n");
}

export function sessionDirForCwd(cwd, sessionsRoot = join(homedir(), ".pi", "agent", "sessions")) {
  const absolute = resolve(cwd);
  const safe = `--${absolute.replace(/^[/\\]/, "").replace(/[/\\:]/g, "-")}--`;
  return join(sessionsRoot, safe);
}

function shortId() {
  return randomUUID().replace(/-/g, "").slice(0, 16);
}

export function toSessionJsonl(source, normalized, options = {}) {
  const session = source.session ?? {};
  const cwd = options.cwd ?? session.directory ?? process.cwd();
  const now = new Date().toISOString();
  const entries = [{
    type: "session",
    version: 3,
    id: randomUUID(),
    timestamp: now,
    cwd,
  }];
  let parentId = null;

  // Hidden digest of the omitted history (default-cap imports). A custom
  // message rides in context without cluttering the visible transcript.
  if (normalized.digest) {
    const id = shortId();
    entries.push({
      type: "custom_message",
      customType: "opencode-import",
      content: normalized.digest,
      display: false,
      id,
      parentId: null,
      timestamp: now,
    });
    parentId = id;
  }

  for (const block of normalized.blocks) {
    const timestamp = new Date(block.timestamp || Date.now()).toISOString();
    if (block.kind === "user") {
      const id = shortId();
      entries.push({
        type: "message",
        id,
        parentId,
        timestamp,
        message: {
          role: "user",
          content: [{ type: "text", text: block.text }],
          timestamp: block.timestamp || Date.now(),
        },
      });
      parentId = id;
      continue;
    }

    const id = shortId();
    entries.push({
      type: "message",
      id,
      parentId,
      timestamp,
      message: {
        role: "assistant",
        content: block.content,
        api: "openai-completions",
        provider: "caduceus",
        model: block.model,
        usage: block.usage,
        stopReason: block.stopReason,
        ...(block.errorMessage ? { errorMessage: block.errorMessage } : {}),
        timestamp: block.timestamp || Date.now(),
      },
    });
    parentId = id;

    for (const result of block.toolResults ?? []) {
      const resultId = shortId();
      entries.push({
        type: "message",
        id: resultId,
        parentId,
        timestamp: new Date(result.timestamp || block.timestamp || Date.now()).toISOString(),
        message: {
          role: "toolResult",
          toolCallId: result.toolCallId,
          toolName: result.toolName,
          content: [{ type: "text", text: result.output }],
          isError: result.isError,
          timestamp: result.timestamp || block.timestamp || Date.now(),
        },
      });
      parentId = resultId;
    }
  }

  return `${entries.map((entry) => JSON.stringify(entry)).join("\n")}\n`;
}

/**
 * Compact an ALREADY-imported pi session (2026-11-14, operator report: one
 * import predates the default budget and carries ~2.6M tokens).
 *
 * Works on the JSONL text: groups entries into conversation units (a user
 * message plus everything before the next user message, so assistant calls
 * stay with their tool results), keeps the newest units within `maxChars`,
 * and replaces the dropped prefix with one hidden digest entry. Parent links
 * are rebuilt; ids are preserved.
 */
export function compactSessionText(text, options = {}) {
  const maxChars = Number.isFinite(options.maxChars) ? options.maxChars : DEFAULT_OPTIONS.maxChars;
  const rows = String(text ?? "")
    .split("\n")
    .filter((line) => line.trim())
    .map((line) => JSON.parse(line));
  const header = rows.find((row) => row.type === "session");
  if (!header || header.version !== 3) throw new Error("not a pi session v3 file");
  const body = rows.filter((row) => row !== header);

  const sizeOf = (row) => JSON.stringify(row).length;
  const isUser = (row) => row.type === "message" && row.message?.role === "user";
  // Units start at each user message; entries before the first user turn
  // (thinking_level_change, custom context) attach to the first unit.
  const units = [];
  let current = null;
  for (const row of body) {
    if (isUser(row) || current === null) {
      if (!current) {
        current = [];
        units.push(current);
      } else if (isUser(row)) {
        current = [row];
        units.push(current);
        continue;
      }
    }
    current.push(row);
  }
  if (units.length === 0) return { text, stats: { keptUnits: 0, omittedUnits: 0, chars: sizeOf(header) } };

  let keptFrom = units.length - 1;
  let chars = 0;
  for (let i = units.length - 1; i >= 0; i -= 1) {
    const unitChars = units[i].reduce((sum, row) => sum + sizeOf(row), 0);
    if (chars + unitChars > maxChars && i < units.length - 1) break;
    chars += unitChars;
    keptFrom = i;
  }

  const omitted = units.slice(0, keptFrom).flat();
  const kept = units.slice(keptFrom).flat();
  const digest = buildImportDigest(
    omitted
      .filter((row) => row.type === "message")
      .map((row) => ({
        kind: row.message?.role === "user" ? "user" : "assistant",
        text:
          row.message?.role === "user"
            ? (row.message.content ?? []).map((c) => c.text ?? "").join(" ")
            : "",
      }))
      .filter((block) => block.text),
    DEFAULT_OPTIONS,
  );

  const entries = [header];
  let parentId = null;
  if (digest) {
    const id = shortId();
    entries.push({
      type: "custom_message",
      customType: "opencode-import",
      content: digest,
      display: false,
      id,
      parentId: null,
      timestamp: new Date().toISOString(),
    });
    parentId = id;
  }
  for (const row of kept) {
    entries.push({ ...row, parentId });
    parentId = row.id;
  }
  const out = `${entries.map((entry) => JSON.stringify(entry)).join("\n")}\n`;
  return {
    text: out,
    stats: {
      keptUnits: units.length - keptFrom,
      omittedUnits: keptFrom,
      omittedEntries: omitted.length,
      keptEntries: kept.length,
      chars: Buffer.byteLength(out),
      maxChars,
    },
  };
}

/** Structural conformance check for a pi session JSONL (v3). */
export function validateSessionText(text) {
  const rows = String(text ?? "")
    .split("\n")
    .filter((line) => line.trim())
    .map((line, index) => {
      try {
        return JSON.parse(line);
      } catch {
        return { __badLine: index + 1 };
      }
    });
  const issues = [];
  const header = rows.find((row) => row.type === "session");
  if (!header) issues.push("missing session header");
  else if (header.version !== 3) issues.push(`session version ${header.version} != 3`);
  const body = rows.filter((row) => row !== header);
  const seen = new Set();
  let prev = null;
  const calls = new Set();
  const results = new Set();
  for (const [index, row] of body.entries()) {
    if (row.__badLine) {
      issues.push(`line ${row.__badLine}: invalid JSON`);
      continue;
    }
    if (!row.id) {
      issues.push(`entry ${index}: missing id`);
      continue;
    }
    if (seen.has(row.id)) issues.push(`duplicate id ${row.id}`);
    seen.add(row.id);
    if (row.parentId !== prev) issues.push(`chain break at ${row.id}`);
    prev = row.id;
    const role = row.message?.role;
    if (row.type === "message" && !["user", "assistant", "toolResult", "system"].includes(role)) {
      issues.push(`entry ${row.id}: bad role ${role}`);
    }
    if (role === "assistant") {
      for (const part of row.message.content ?? []) {
        if (part.type === "toolCall") calls.add(part.id);
      }
    }
    if (role === "toolResult") results.add(row.message.toolCallId);
  }
  for (const call of calls) {
    if (!results.has(call)) issues.push(`toolCall ${call} has no toolResult`);
  }
  for (const result of results) {
    if (!calls.has(result)) issues.push(`toolResult ${result} has no toolCall`);
  }
  return {
    ok: issues.length === 0,
    issues,
    stats: {
      entries: body.length,
      chars: Buffer.byteLength(String(text ?? "")),
      toolCalls: calls.size,
      toolResults: results.size,
    },
  };
}

export function writeSession(cwd, source, normalized, options = {}) {
  const dir = sessionDirForCwd(cwd);
  mkdirSync(dir, { recursive: true, mode: 0o700 });
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const path = join(dir, `${stamp}_${randomUUID()}.jsonl`);
  const text = toSessionJsonl(source, normalized, { ...options, cwd });
  const tmp = `${path}.tmp-${process.pid}`;
  writeFileSync(tmp, text, { mode: 0o600 });
  chmodSync(tmp, 0o600);
  renameSync(tmp, path);
  chmodSync(path, 0o600);
  return { path, bytes: Buffer.byteLength(text) };
}

export function readOpenCodeSessionFile(sessionFile) {
  const text = readFileSync(sessionFile, "utf8");
  return JSON.parse(text);
}
