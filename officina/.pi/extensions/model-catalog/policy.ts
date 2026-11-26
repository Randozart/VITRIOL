import { EXACT_PROVIDER, toolCandidates } from "../_shared/caduceus-catalog.ts";
import { applyPrefs, allowPaidOwners, readPrefs } from "../_shared/caduceus-prefs.ts";
import { budgetStatus } from "../_shared/budget.ts";
import { readGatewayMarks } from "../_shared/gateway-health.ts";
import { cooldownRemaining } from "../_shared/provider-health.ts";
import { LOCAL_PROVIDERS, ownerOf, type ChatModel } from "./catalog.ts";

type Marks = Pick<ReturnType<typeof readGatewayMarks>, "models" | "owners" | "pausedOwners" | "invalid">;

export interface Eligibility {
  raw: unknown[];
  prefs: ReturnType<typeof readPrefs>;
  budget: Pick<ReturnType<typeof budgetStatus>, "paidAllowed" | "allowed">;
  marks: Marks;
  parked: ReadonlyMap<string, number>;
  cooldownMs: (provider: string, model: string) => number;
}

function routeBlocked(model: ChatModel, settings: Eligibility, includeCooling: boolean): boolean {
  if (model.api === "pi-virtual") return true;
  const gateway = model.provider === EXACT_PROVIDER;
  const marks = settings.marks;
  if (gateway && marks.invalid.has(model.id)) return true;
  if (includeCooling) return false;
  if (waitMs(model, settings) > 0) return true;
  const owner = ownerOf(model);
  return gateway && (marks.models.has(model.id) || marks.owners.has(owner) || marks.pausedOwners.has(owner));
}

function paidAllowed(model: ChatModel, owner: string, settings: Eligibility, paidOwners: Set<string>): boolean {
  const { prefs, budget } = settings;
  const priced = model.cost.input > 0 || model.cost.output > 0;
  return priced && budget.paidAllowed && budget.allowed && (prefs?.free_only === false || paidOwners.has(owner));
}

/** The group has already been matched by exact identity. This only filters
 * eligibility and orders providers, so it cannot add a different model. */
export function eligibleRoutes(models: ChatModel[], settings: Eligibility, includeCooling = false): ChatModel[] {
  const paidOwners = allowPaidOwners(settings.prefs);
  const groupIds = new Set(models.filter((model) => model.provider === EXACT_PROVIDER).map(({ id }) => id));
  const rawGroup = settings.raw.filter((row) => groupIds.has((row as { id?: string } | null)?.id ?? ""));
  const freeIds = new Set(toolCandidates(rawGroup, { freeOnly: false }).filter((model) => model.free).map(({ id }) => id));
  const eligible = models.flatMap((model) => {
    if (routeBlocked(model, settings, includeCooling)) return [];
    const owner = ownerOf(model);
    const gateway = model.provider === EXACT_PROVIDER;
    const free = LOCAL_PROVIDERS.has(owner) || (gateway && freeIds.has(model.id));
    if (!free && !paidAllowed(model, owner, settings, paidOwners)) return [];
    return [{ id: gateway ? model.id : `${model.provider}/${model.id}`, owner, model, free }];
  });
  eligible.sort((a, b) => Number(b.free) - Number(a.free));
  return applyPrefs(eligible, settings.prefs).map(({ model }) => model);
}

export function waitMs(model: ChatModel, settings: Eligibility): number {
  const gateway = model.provider === EXACT_PROVIDER;
  return Math.max(
    settings.cooldownMs(gateway ? "caduceus" : model.provider, model.id),
    gateway ? (settings.parked.get(model.id) ?? 0) - Date.now() : 0,
  );
}

export function currentEligibility(raw: unknown[], parked: ReadonlyMap<string, number>): Eligibility {
  return { raw, prefs: readPrefs(), budget: budgetStatus(), marks: readGatewayMarks(), parked, cooldownMs: cooldownRemaining };
}
