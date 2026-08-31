import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

// injection-guard (TS port, 2026-08-31, SS2 gateway fold-in).
//
// Provenance: ported from trismegistus/hermes-plugins/injection-guard/guards.py
// @ 237e424 (owner-authored, MIT; OmniRoute §4.5 / REPORT-02 step 21).
// Same patterns, same mode discipline, new home: ingested content (browser
// extractions, webfetch output) passes the guard BEFORE entering context.
//
// Modes (env TRIS_GUARD_MODE):
//   log    DEFAULT — annotate the result, never drop content (false
//          positives must not degrade ingestion)
//   block  replace flagged spans with a quarantine stub (opt-in)
//
// Kill switch: OFFICINA_INJECTION_GUARD=0 (Rule 15).

const INJECTION_PATTERNS: Array<[string, RegExp]> = [
  ["override-instructions", /ignore\s+(?:all\s+|any\s+|the\s+)?(?:previous|prior|above|earlier)\s+(?:instructions|prompts|rules|messages)/i],
  ["disregard-rules", /disregard\s+(?:all\s+)?(?:your|the|any)\s+(?:instructions|rules|guidelines|safety)/i],
  ["fake-role-line", /^\s*(?:system|assistant|developer)\s*:\s/im],
  ["chat-template-token", /<\|(?:(?:im_start|im_end|endoftext|system|user|assistant)[^|]*)\|>/i],
  ["jailbreak-mode", /\b(?:DAN|do anything now|jailbreak mode|developer\s+mode\s+enabled)\b/i],
  ["prompt-exfil", /(reveal|print|output|leak)\s+(?:your|the)\s+(?:system\s+prompt|hidden\s+instructions|initial\s+prompt)/i],
  ["tool-impersonation", /<\s*(?:antml|tool_call|function_call)\s*>/i],
  ["new-instructions", /new\s+instructions?\s*[:-]/i],
];

const SECRET_PATTERNS: Array<[string, RegExp]> = [
  ["aws-key", /\bAKIA[0-9A-Z]{16}\b/],
  ["github-token", /\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,}\b/],
  ["github-fine", /\bgithub_pat_[A-Za-z0-9_]{40,}\b/],
  ["slack-token", /\bxox[baprs]-[A-Za-z0-9-]{10,}\b/],
  ["openai-key", /\bsk-[A-Za-z0-9_-]{20,}\b/],
  ["anthropic-key", /\bsk-ant-[A-Za-z0-9_-]{20,}\b/],
  ["jwt", /\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b/],
  ["private-key", /-----BEGIN (?:RSA |EC |OPENSSH |PGP )?PRIVATE KEY-----/],
  ["bearer", /\bauthorization:\s*bearer\s+[A-Za-z0-9._~+/=-]{20,}/i],
];

const QUARANTINE_STUB =
  "[injection-guard: content quarantined — {n} pattern(s) matched: {kinds}; raw kept out of context per config mode=block]";

export function scanInjections(text: string): string[] {
  return INJECTION_PATTERNS.filter(([, rx]) => rx.test(text)).map(([kind]) => kind).sort();
}

export function maskSecrets(text: string): { masked: string; kinds: string[] } {
  let masked = text;
  const kinds: string[] = [];
  for (const [kind, rx] of SECRET_PATTERNS) {
    if (rx.test(masked)) {
      masked = masked.replace(new RegExp(rx.source, rx.flags.replace("g", "") + "g"), `[masked:${kind}]`);
      kinds.push(kind);
    }
  }
  return { masked, kinds };
}

export function guardText(text: string, mode: string): { text: string; notes: string[] } {
  const notes: string[] = [];
  const inj = scanInjections(text);
  if (inj.length > 0) {
    notes.push(`injection patterns: ${inj.join(", ")}`);
  }
  const sec = maskSecrets(text);
  if (sec.kinds.length > 0) {
    text = sec.masked;
    notes.push(`secrets masked: ${sec.kinds.join(", ")}`);
  }
  if (inj.length > 0 && mode === "block") {
    const kinds = [...inj, ...sec.kinds].join(", ");
    return { text: QUARANTINE_STUB.replace("{n}", String(inj.length)).replace("{kinds}", kinds), notes };
  }
  if (notes.length > 0) {
    return { text: text + `\n[injection-guard: ${notes.join("; ")}]`, notes };
  }
  return { text, notes };
}

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_INJECTION_GUARD === "0") return; // Rule 15
  const mode = process.env.TRIS_GUARD_MODE === "block" ? "block" : "log";

  // Gate INGESTED content: browser extractions and webfetch results are the
  // attack surface (PDFs/web pages containing instructions aimed at the
  // agent). Locally-authored tool output is not screened — the operator
  // already controls it.
  pi.on("tool_result", (event) => {
    const ingestTools = new Set(["browser-extract", "webfetch", "browser_extract", "web_fetch"]);
    if (!ingestTools.has(event.toolName)) return;
    try {
      const content = (event as { content?: Array<{ type?: string; text?: string }> }).content ?? [];
      for (const part of content) {
        if (part.type === "text" && part.text) {
          const g = guardText(part.text, mode);
          part.text = g.text;
        }
      }
    } catch {
      // guard must never break the tool result
    }
  });
}
