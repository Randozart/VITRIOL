// Coupling registry (pure; testable) — 2026-08-31.
//
// The DEFAULT coupling is always "lapis-occultus": whatever VITRIOL is
// currently serving, regardless of which model or settings — the endpoint
// is the coupling, the model behind it is incidental. Alternative
// couplings (e.g. cloud "ascensus" escalation) come from
// ~/.vitriol/officina/couplings.json (or ~/.config/officina/couplings.json),
// shape documented in officina/couplings.example.json.

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

export const LAPIS = { id: "lapis-occultus", name: "Lapis Occultus – VITRIOL" };

export interface CouplingDef {
  id: string;
  name: string;
  baseUrl: string;
  api: string;
  apiKeyEnv?: string;
  apiKey?: string;
  models: Array<{ id: string; name: string; contextWindow: number; maxTokens: number }>;
}

export function couplingPaths(home: string): string[] {
  return [join(home, ".vitriol", "officina", "couplings.json"), join(home, ".config", "officina", "couplings.json")];
}

export function loadCouplings(env: NodeJS.ProcessEnv = process.env): CouplingDef[] {
  const home = env.HOME || env.USERPROFILE || "";
  if (!home) return [];
  for (const p of couplingPaths(home)) {
    if (!existsSync(p)) continue;
    try {
      const parsed = JSON.parse(readFileSync(p, "utf-8")) as { couplings?: unknown };
      const arr = Array.isArray(parsed.couplings) ? parsed.couplings : [];
      const out: CouplingDef[] = [];
      for (const c of arr as Array<Record<string, unknown>>) {
        const id = typeof c.id === "string" ? c.id : "";
        const name = typeof c.name === "string" ? c.name : id;
        const baseUrl = typeof c.baseUrl === "string" ? c.baseUrl : "";
        const models = Array.isArray(c.models) ? (c.models as CouplingDef["models"]) : [];
        if (!id || !baseUrl || models.length === 0) continue;
        out.push({
          id,
          name,
          baseUrl,
          api: typeof c.api === "string" ? c.api : "openai-completions",
          apiKeyEnv: typeof c.apiKeyEnv === "string" ? c.apiKeyEnv : undefined,
          apiKey: typeof c.apiKey === "string" ? c.apiKey : undefined,
          models,
        });
      }
      return out; // first present file wins (same precedence as llama-cpp-provider)
    } catch {
      continue; // rotten file = no couplings, never a crash
    }
  }
  return [];
}

// Display name for the CURRENT coupling, given the active model's provider.
// "llamacpp" (the VITRIOL endpoint provider) is Lapis Occultus whatever
// model id sits behind it; officina-registered couplings map to their name.
export function providerTagOf(c: CouplingDef): string {
  return `officina-${c.id}`;
}

export function couplingDisplay(
  provider: string | undefined,
  modelId: string | undefined,
  couplings: CouplingDef[],
  vitriolModelId?: string,
): string {
  if (!provider || provider === "llamacpp") {
    return `${LAPIS.name}${vitriolModelId ? ` · ${vitriolModelId}` : ""}`;
  }
  const def = couplings.find((c) => providerTagOf(c) === provider);
  if (def) return `${def.name} · ${modelId ?? ""}`.trim();
  return modelId ?? provider;
}
