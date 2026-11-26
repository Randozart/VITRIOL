// Model and provider pickers over the live Pi registry. The native TUI
// renders the existing RPC select/input dialogs.
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { ExtensionAPI, ExtensionContext, ExtensionCommandContext } from "@earendil-works/pi-coding-agent";
import { EXACT_PROVIDER } from "../caduceus/catalog.ts";
import { applyModel } from "../_shared/routing/apply.ts";
import { readRouteEntries, recordRoute } from "../_shared/route-ledger.ts";
import {
  AUTO_PROVIDER, LOCAL_PROVIDERS, distinctRoutes, groupModels, identity, keyOf,
  ownerOf, pickableModels, selectRoute, type ChatModel, type ModelKey,
} from "./catalog.ts";
import { currentEligibility, eligibleRoutes, waitMs } from "./policy.ts";
import { recentModels, rememberModel } from "./recent.ts";

const TTL = 5 * 60_000;
function workshopFile(name: string): string {
  return join(homedir(), ".vitriol", "officina", name);
}
const SEARCH = "Search catalog…";

function readJson(path: string): Record<string, unknown> {
  try { return JSON.parse(readFileSync(path, "utf8")) ?? {}; } catch { return {}; }
}

function gatewayPolicy() {
  const raw = readJson(workshopFile("caduceus-models.json")).data;
  const marks = readJson(process.env.CADUCEUS_ROUTER_STATE || workshopFile("caduceus-router-state.json")).parked;
  const active = new Map(Object.entries(marks ?? {}).flatMap(([id, until]) =>
    typeof until === "number" && until > Date.now() ? [[id, until] as const] : [],
  ));
  return currentEligibility(Array.isArray(raw) ? raw : [], active);
}

function allModels(ctx: ExtensionContext): ChatModel[] {
  return ctx.modelRegistry.getAvailable();
}

export function createDiscovery() {
  let attemptedAt = -Infinity;
  let updatedAt = 0;
  let failed = false;
  let pending: Promise<void> | undefined;
  return {
    refresh(ctx: Pick<ExtensionContext, "modelRegistry">) {
      if (pending || Date.now() - attemptedAt < TTL) return pending;
      attemptedAt = Date.now();
      pending = Promise.resolve().then(() => {
        const providers = new Set(ctx.modelRegistry.getAvailable().map((model) => model.provider));
        for (const provider of ctx.modelRegistry.getRegisteredProviderIds()) {
          if (ctx.modelRegistry.getProviderAuthStatus(provider).configured) providers.add(provider);
        }
        providers.delete(AUTO_PROVIDER);
        providers.delete(EXACT_PROVIDER);
        return ctx.modelRegistry.refresh({
          allowNetwork: true, providers: [...providers], signal: AbortSignal.timeout(12_000),
        });
      }).then((result) => {
        failed = result.aborted || result.errors.size > 0;
        if (!failed) updatedAt = Date.now();
      }).catch(() => { failed = true; }).finally(() => { pending = undefined; });
      return pending;
    },
    status() {
      if (pending) return "cached · refreshing";
      if (failed) return updatedAt ? `last refresh ${Math.floor((Date.now() - updatedAt) / 60_000)}m ago · some listings unavailable` : "cached · some listings unavailable";
      return updatedAt ? `discovered ${Math.floor((Date.now() - updatedAt) / 60_000)}m ago` : "cached";
    },
  };
}

function groupTitle(models: ChatModel[]): string {
  return models[0]!.name || identity(models[0]!);
}

function routeLabel(model: ChatModel): string {
  if (model.provider === AUTO_PROVIDER) return "Auto-route";
  return model.provider === EXACT_PROVIDER ? `${ownerOf(model)} via Caduceus` : ownerOf(model);
}

function previousRoute(row: ReturnType<typeof readRouteEntries>[number]): ModelKey | undefined {
  if (row.outcome !== "ok" || !row.provider || !row.model) return;
  return { provider: row.provider === "caduceus" ? EXACT_PROVIDER : row.provider, id: row.model };
}

function availableRecent(models: ChatModel[]): ChatModel[] {
  const selectable = new Map(models.map((model) => [keyOf(model), model]));
  const ordered = new Map(recentModels(workshopFile("recent-models.jsonl")).map((model) => [keyOf(model), model]));
  for (const row of readRouteEntries(7).reverse()) {
    if (ordered.size >= 16) break;
    const route = previousRoute(row);
    if (route && selectable.has(keyOf(route))) ordered.set(keyOf(route), route);
  }
  return [...ordered.keys()].flatMap((key) => selectable.get(key) ?? []);
}

interface Option<T> { label: string; item: T }

async function pick<T>(
  ctx: ExtensionCommandContext, title: string, options: Option<T>[], initial = "",
): Promise<T | undefined> {
  let filter = initial.toLowerCase();
  while (true) {
    const shown = options.filter(({ label }) => label.toLowerCase().includes(filter));
    const labels = shown.map(({ label }) => label);
    const choice = await ctx.ui.select(
      filter ? `${title} · "${filter}"` : title, [SEARCH, ...labels],
    );
    if (!choice) return;
    if (choice !== SEARCH) return shown[labels.indexOf(choice)]?.item;
    const input = await ctx.ui.input("Filter models or providers", filter || "Enter a name or ID");
    if (input !== undefined) filter = input.trim().toLowerCase();
  }
}

function modelRows(models: ChatModel[], recent: ChatModel[]): Option<ChatModel>[] {
  const relevant = new Set(models.map(keyOf));
  return [
    ...recent.filter((model) => relevant.has(keyOf(model))).map((model) => ({
      label: `Recent · ${model.name} · ${routeLabel(model)} · ${model.id}`, item: model,
    })),
    ...models.map((model) => ({ label: `${model.name} · ${routeLabel(model)} · ${model.id}`, item: model })),
  ];
}

async function chooseModel(pi: ExtensionAPI, ctx: ExtensionCommandContext, model: ChatModel) {
  await ctx.waitForIdle();
  const current = ctx.modelRegistry.find(model.provider, model.id);
  if (!current || !await applyModel(pi, current, { owner: "model-catalog", reason: "operator selection", operator: true })) {
    ctx.ui.notify(`Could not select ${model.name} via ${routeLabel(model)}`, "warning");
    return;
  }
  rememberModel(workshopFile("recent-models.jsonl"), current);
  ctx.ui.notify(`${model.name} · ${routeLabel(model)}`, "info");
}

type RouteHistory = { tried: string[] };

function registerAuto(pi: ExtensionAPI, id: string, name: string, models: ChatModel[]) {
  pi.registerVirtualModel<RouteHistory>({
    provider: AUTO_PROVIDER,
    id,
    name: `${name} · Auto-route`,
    ...(models.length ? {
      contextWindow: Math.min(...models.map((model) => model.contextWindow)),
      maxTokens: Math.min(...models.map((model) => model.maxTokens)),
    } : {}),
    route(request, ctx) {
      request.signal?.throwIfAborted();
      const group = groupModels(pickableModels(allModels(ctx))).get(id) ?? [];
      const policy = gatewayPolicy();
      const eligible = eligibleRoutes(group, policy);
      const tried = new Set(request.reason === "retry" ? request.state?.tried : []);
      if (request.failed) tried.add(keyOf(request.failed.model));
      const model = selectRoute(eligible, tried, request.previous?.model);
      if (!model) {
        const waits = eligibleRoutes(group, policy, true).map((route) => waitMs(route, policy)).filter((wait) => wait > 0);
        const soonest = waits.length ? ` Earliest known cooldown ends in ${Math.ceil(Math.min(...waits) / 1000)}s.` : "";
        throw new Error(`No eligible provider for ${id}.${soonest}`);
      }
      return { model, thinkingLevel: "off", state: { tried: [...tried] } };
    },
  });
}

type ModelChoice = { model: ChatModel } | { identity: string };

async function browseModels(
  pi: ExtensionAPI, ctx: ExtensionCommandContext, models: ChatModel[], title: string, filter: string,
) {
  const pickable = pickableModels(models);
  const groups = groupModels(pickable);
  const recent = availableRecent(models.filter((model) => model.provider !== "caduceus"));
  const master: Option<ModelChoice>[] = [
    ...recent.map((model) => ({
      label: `Recent · ${model.name} · ${routeLabel(model)} · ${model.id}`, item: { model },
    })),
    ...[...groups.entries()].map(([id, offers]) => ({
      label: `${groupTitle(offers)} · ${id}`, item: { identity: id },
    })).sort((a, b) => a.label.localeCompare(b.label)),
  ];
  const choice = await pick(ctx, title, master, filter);
  if (!choice) return;
  if ("model" in choice) return chooseModel(pi, ctx, choice.model);

  // Refresh may have completed while the first dialog was open.
  const offers = groupModels(pickableModels(allModels(ctx))).get(choice.identity) ?? [];
  if (!offers.length) {
    ctx.ui.notify("This model is no longer in the available catalog", "warning");
    return;
  }
  const choices: Option<ChatModel | "auto">[] = offers.map((model) => ({
    label: `${routeLabel(model)} · ${model.id}`, item: model,
  }));
  if (distinctRoutes(offers)) choices.unshift({ label: "Auto-route · same model", item: "auto" });
  const route = await pick(ctx, `${groupTitle(offers)} · pick provider`, choices);
  if (!route) return;
  if (route !== "auto") return chooseModel(pi, ctx, route);
  registerAuto(pi, choice.identity, groupTitle(offers), offers);
  const virtual = ctx.modelRegistry.find(AUTO_PROVIDER, choice.identity);
  if (virtual) await chooseModel(pi, ctx, virtual);
}

async function browseProviders(
  pi: ExtensionAPI, ctx: ExtensionCommandContext, models: ChatModel[], title: string, filter: string,
) {
  const owners = [...new Set(pickableModels(models).map(ownerOf))].sort();
  const provider = await pick(ctx, title, owners.map((owner) => ({ label: owner, item: owner })), filter);
  if (!provider) return;
  const available = pickableModels(allModels(ctx)).filter((model) => ownerOf(model) === provider);
  if (!available.length) {
    ctx.ui.notify("This provider has no available models", "warning");
    return;
  }
  const model = await pick(ctx, `${provider} · pick model`, modelRows(available, availableRecent(available)));
  if (model) await chooseModel(pi, ctx, model);
}

function recordUses(pi: ExtensionAPI) {
  pi.on("message_end", (event, ctx) => {
    if (event.message.role !== "assistant") return;
    const message = event.message;
    if (message.stopReason !== "error" && message.stopReason !== "aborted") {
      rememberModel(workshopFile("recent-models.jsonl"), ctx.model ?? { provider: message.provider, id: message.model });
    }
    if (ctx.model?.provider !== AUTO_PROVIDER && ctx.model?.provider !== EXACT_PROVIDER) return;
    if (message.provider === AUTO_PROVIDER || !message.usage) return;
    recordRoute({
      ts: new Date().toISOString(),
      sessionId: ctx.sessionManager.getSessionId(),
      tier: LOCAL_PROVIDERS.has(message.provider) ? "local" : "cloud",
      reason: ctx.model.provider === AUTO_PROVIDER ? "same-model auto-route" : "exact-model selection",
      provider: message.provider,
      model: message.model,
      tokensIn: message.usage.input,
      tokensOut: message.usage.output,
      cacheRead: message.usage.cacheRead,
      cacheWrite: message.usage.cacheWrite,
      costEstimate: message.usage.cost.total,
      outcome: message.stopReason === "error" ? "error" : message.stopReason === "aborted" ? "cancelled" : "ok",
    });
  });
}

export default function modelCatalog(pi: ExtensionAPI) {
  const discovery = createDiscovery();
  // Virtual selections must exist before Pi restores the last model in a session.
  for (const previous of recentModels(workshopFile("recent-models.jsonl"))) {
    if (previous.provider === AUTO_PROVIDER) registerAuto(pi, previous.id, previous.id, []);
  }
  pi.on("session_start", (_event, ctx) => { if (ctx.hasUI) void discovery.refresh(ctx); });
  recordUses(pi);
  for (const [name, browse, title] of [
    ["models", browseModels, "Models"], ["providers", browseProviders, "Providers"],
  ] as const) {
    pi.registerCommand(name, {
      description: name === "models" ? "Pick a model, then its provider" : "Pick a provider, then its model",
      handler: async (args, ctx) => {
        const refresh = discovery.refresh(ctx);
        if (!pickableModels(allModels(ctx)).length) await refresh;
        const models = allModels(ctx);
        if (!pickableModels(models).length) {
          ctx.ui.notify("No available models. Check provider connections.", "warning");
          return;
        }
        await browse(pi, ctx, models, `${title} · ${discovery.status()}`, args.trim());
      },
    });
  }
}
