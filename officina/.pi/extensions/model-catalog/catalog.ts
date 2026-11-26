import type { Api, Model } from "@earendil-works/pi-ai";
import { EXACT_PROVIDER } from "../caduceus/catalog.ts";

export const AUTO_PROVIDER = "officina-auto";
export const LOCAL_PROVIDERS = new Set(["llamacpp", "llama.cpp", "vitriol"]);

export type ChatModel = Model<Api>;
export type ModelKey = Pick<ChatModel, "provider" | "id">;

export function keyOf(model: ModelKey): string {
  return JSON.stringify([model.provider, model.id]);
}

export function ownerOf(model: ModelKey): string {
  return model.provider === EXACT_PROVIDER ? model.id.split("/")[0]! : model.provider;
}

/** Strip only the proven gateway transport prefix. Preserve case, namespace,
 * dated IDs and variants. Virtual lane IDs have their own namespace. */
export function identity(model: ChatModel): string {
  if (model.api === "pi-virtual") return keyOf(model);
  return model.provider === EXACT_PROVIDER
    ? model.id.slice(model.id.indexOf("/") + 1)
    : model.id;
}

export function pickableModels(models: ChatModel[]): ChatModel[] {
  return models.filter((model) => model.provider !== "caduceus" && model.provider !== AUTO_PROVIDER);
}

export function groupModels(models: ChatModel[]): Map<string, ChatModel[]> {
  const groups = new Map<string, ChatModel[]>();
  for (const model of models) {
    const id = identity(model);
    const group = groups.get(id) ?? [];
    group.push(model);
    groups.set(id, group);
  }
  return groups;
}

export function distinctRoutes(models: ChatModel[]): boolean {
  return new Set(models.map(ownerOf)).size > 1;
}

export function selectRoute(
  models: ChatModel[], tried: ReadonlySet<string>, preferred?: ModelKey,
): ChatModel | undefined {
  const available = models.filter((model) => !tried.has(keyOf(model)));
  return available.find((model) => preferred && keyOf(model) === keyOf(preferred)) ?? available[0];
}
