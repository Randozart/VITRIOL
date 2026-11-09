// Pure prompt-cache accounting over pi session JSONL. Mirrors pi's cache-stats
// miss heuristic: a miss is prompt tokens that were in the previous request but
// were not reported as cache reads.

const NOISE_FLOOR_TOKENS = 1024;
const CACHE_TTL_MS = 5 * 60 * 1000;

export function parseSessionJsonl(text) {
  const entries = [];
  for (const line of String(text ?? "").split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    try {
      entries.push(JSON.parse(trimmed));
    } catch {
      // A truncated final line is normal while a session is live.
    }
  }
  return entries;
}

function promptTokens(usage) {
  return (usage?.input ?? 0) + (usage?.cacheRead ?? 0) + (usage?.cacheWrite ?? 0);
}

function paidPerToken(usage) {
  const paid = (usage?.input ?? 0) + (usage?.cacheWrite ?? 0);
  const cost = (usage?.cost?.input ?? 0) + (usage?.cost?.cacheWrite ?? 0);
  return paid > 0 ? cost / paid : 0;
}

function readPerToken(usage) {
  return usage?.cacheRead > 0 ? (usage?.cost?.cacheRead ?? 0) / usage.cacheRead : 0;
}

export function computeCacheReport(entries) {
  let prev = null;
  let turns = 0;
  let input = 0;
  let output = 0;
  let cacheRead = 0;
  let cacheWrite = 0;
  let missedTokens = 0;
  let missedCost = 0;
  let missCount = 0;
  let idleMisses = 0;
  let modelSwitches = 0;
  let lastModel = "";
  let first = null;
  let last = null;

  for (const entry of entries) {
    if (entry?.type === "compaction" || entry?.type === "branch_summary") {
      prev = null;
      continue;
    }
    if (entry?.type !== "message" || entry?.message?.role !== "assistant") continue;
    const message = entry.message;
    const usage = message.usage ?? {};
    const tokens = promptTokens(usage);
    const modelKey = `${message.provider ?? ""}/${message.model ?? ""}`;
    turns += 1;
    input += usage.input ?? 0;
    output += usage.output ?? 0;
    cacheRead += usage.cacheRead ?? 0;
    cacheWrite += usage.cacheWrite ?? 0;
    if (lastModel && modelKey !== lastModel) modelSwitches += 1;
    lastModel = modelKey;
    if (!first) first = message.timestamp ?? null;
    last = message.timestamp ?? last;

    if (prev && tokens > 0 && (usage.cacheRead + usage.cacheWrite > 0 || prev.reportedCache)) {
      const missed = Math.min(prev.tokens, tokens) - (usage.cacheRead ?? 0);
      if (missed > NOISE_FLOOR_TOKENS) {
        missedTokens += missed;
        missCount += 1;
        const idle = Math.max(0, (message.timestamp ?? 0) - prev.timestamp);
        if (idle > CACHE_TTL_MS) idleMisses += 1;
        missedCost += missed * Math.max(0, paidPerToken(usage) - readPerToken(usage));
      }
    }

    prev = {
      tokens,
      timestamp: message.timestamp ?? 0,
      reportedCache: Boolean(prev?.reportedCache || usage.cacheRead + usage.cacheWrite > 0),
    };
  }

  const promptTotal = input + cacheRead + cacheWrite;
  return {
    turns,
    input,
    output,
    cacheRead,
    cacheWrite,
    promptTokens: promptTotal,
    hitRate: promptTotal > 0 ? cacheRead / promptTotal : 0,
    missCount,
    missedTokens,
    missedCost,
    idleMisses,
    modelSwitches,
    startedAt: first,
    endedAt: last,
  };
}
