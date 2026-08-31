// caveman-rules — deterministic prose compression, TS port (2026-08-31).
//
// Provenance: ported from trismegistus/hermes-plugins/caveman-rules/compress.py
// @ 237e424 (owner-authored; R2.3 / REPORT-02 step 19a; measured −65% output
// tokens on the owner's own caveman skill ruleset). Same ruleset, same
// guarantees: code spans byte-preserved, output never longer than input.
//
// Applied BY RULE to compression-allowed text only (sub-coder reports,
// memory retrieval) — never to system prompts, tool schemas, plans, or code.
// Ships DARK: armed only when TRIS_CAVEMAN=1 (same as upstream).

const FILLER_DROPS =
  /\b(?:just|really|basically|actually|simply|essentially|literally|clearly|obviously|definitely|very)\s+/gi;

const SUBS: Array<[RegExp, string]> = [
  [/\bin order to\b/gi, "to"],
  [/\bat this point in time\b/gi, "now"],
  [/\ba large number of\b/gi, "many"],
  [/\bwith respect to\b/gi, "for"],
  [/\bin the event that\b/gi, "if"],
  [/\bdue to the fact that\b/gi, "because"],
  [/\b(?:please )?note that\b/gi, ""],
  [/\bit is worth noting that\b/gi, ""],
  [/\bI(?:'ll| will| am going to| will be| need to|'m going to)\s+/g, "I "],
  [/\bhave not\b/gi, "haven't"],
  [/\bcannot\b/gi, "can't"],
  [/\bwas not\b/gi, "wasn't"],
  [/\bdid not\b/gi, "didn't"],
  [/\bdo not\b/gi, "don't"],
  [/\bis not\b/gi, "isn't"],
  [/\bthe following\b/gi, ""],
];

const PREAMBLE =
  /^(?:Sure[!,]? (?:I['a]m )?(?:happy to|glad to)[^.]*\.?|Certainly[!,]?|Of course[!,]?|Great question[!,]?)[\s.]*/i;

const CODE_SPANS =
  /(```[\s\S]*?```|~~~[\s\S]*?~~~|`[^`\n]*`|^ {4,}.*$|^[+-](?![+-]).*$)/gm;

interface Span {
  token: string;
  text: string;
}

function protect(text: string): { masked: string; spans: Span[] } {
  const spans: Span[] = [];
  const masked = text.replace(CODE_SPANS, (m) => {
    const token = `\x00SP${spans.length}\x00`;
    spans.push({ token, text: m });
    return token;
  });
  return { masked, spans };
}

function restore(text: string, spans: Span[]): string {
  let out = text;
  for (const s of spans) out = out.replace(s.token, s.text);
  return out;
}

/** Caveman-lite deterministic reduction. Never inflates. */
export function compressProse(text: string): string {
  if (!text || text.length < 40) return text;
  const { masked, spans } = protect(text);
  let out = masked;
  for (const [pat, repl] of SUBS) out = out.replace(new RegExp(pat.source, pat.flags), repl);
  out = out.replace(FILLER_DROPS, "");
  out = out.replace(PREAMBLE, "");
  out = out.replace(/(?<=[ \n])(?:a|an|the) (?=[a-z]{3,} )/g, "");
  out = out.replace(/[ \t]{2,}/g, " ");
  out = restore(out, spans);
  return out.length <= text.length ? out : text;
}

export function reductionPct(original: string, compressed: string): number {
  if (!original) return 0;
  return ((original.length - compressed.length) / original.length) * 100;
}
