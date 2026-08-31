import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { LAPIS, loadCouplings, providerTagOf, type CouplingDef } from "../_shared/couplings.ts";

// coupling (2026-08-31, owner request): the default coupling is
// Lapis Occultus – VITRIOL — "we use whatever settings VITRIOL is running
// with, regardless of which model that is". This ext lets the owner
// connect to a DIFFERENT provider and hot-swap mid-session (pi.setModel
// preserves the conversation), with the VITRIOL endpoint one command away
// again. Groundwork for a built-in ascensus (euro-capped cloud escalation)
// tool: an ascensus coupling in couplings.json is already all it takes.
//
// Couplings file: ~/.vitriol/officina/couplings.json (shape:
// officina/couplings.example.json). Kill switch: OFFICINA_COUPLING=0.

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_COUPLING === "0") return; // Rule 15

  const couplings: CouplingDef[] = loadCouplings();

  for (const c of couplings) {
    pi.registerProvider(providerTagOf(c), {
      baseUrl: c.baseUrl,
      apiKey: c.apiKeyEnv ? `$${c.apiKeyEnv}` : (c.apiKey ?? "none"),
      api: c.api,
      models: c.models.map((m) => ({
        id: m.id,
        name: m.name,
        reasoning: false,
        input: ["text"] as ("text")[],
        contextWindow: m.contextWindow,
        maxTokens: m.maxTokens,
        cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      })),
    });
  }

  pi.registerCommand("coupling", {
    description:
      "Show couplings, or hot-swap: /coupling <id> (default: lapis-occultus = whatever VITRIOL is serving)",
    handler: async (args: string, ctx: { ui: any; modelRegistry: any; model?: { provider?: string } }) => {
      const wanted = args.trim();
      const lines: string[] = [`◈ ${LAPIS.name} (default)`];
      for (const c of couplings) {
        const modelList = c.models.map((m) => m.id).join(", ");
        lines.push(`◈ ${c.name} — models: ${modelList}`);
      }

      if (!wanted) {
        const currentProvider = ctx.model?.provider;
        const current =
          !currentProvider || currentProvider === "llamacpp"
            ? LAPIS.name
            : (couplings.find((c) => providerTagOf(c) === currentProvider)?.name ?? currentProvider);
        lines.push("");
        lines.push(`current: ${current}`);
        lines.push(`switch:  /coupling ${couplings[0]?.id ?? "<id>"}`);
        await ctx.ui.select(lines.join("\n"), couplings.map((c) => c.id).concat(["__stay__"]))
          .catch(() => undefined);
        return;
      }

      if (wanted === LAPIS.id || wanted === "vitriol" || wanted === "lapis") {
        // Back to the stone: whatever VITRIOL is serving right now.
        const model = ctx.modelRegistry.find("llamacpp", undefined as never) ?? undefined;
        const anyLlamacpp =
          model ??
          (ctx.modelRegistry as { getAll?: () => Array<{ provider: string }> }).getAll?.().find(
            (m) => m.provider === "llamacpp",
          );
        if (!anyLlamacpp) {
          await ctx.ui.notify?.("no llamacpp model registered — is the VITRIOL provider loaded?", "error");
          return;
        }
        const ok = await pi.setModel(anyLlamacpp as never);
        await ctx.ui.notify?.(ok ? `coupled: ${LAPIS.name}` : "switch refused", ok ? "info" : "warning");
        return;
      }

      const def = couplings.find((c) => c.id === wanted);
      if (!def) {
        await ctx.ui.notify?.(`unknown coupling '${wanted}' — /coupling lists them`, "error");
        return;
      }
      const model = ctx.modelRegistry.find(providerTagOf(def), def.models[0]!.id);
      if (!model) {
        await ctx.ui.notify?.(`coupling '${wanted}' registered no usable model`, "error");
        return;
      }
      const ok = await pi.setModel(model as never);
      await ctx.ui.notify?.(ok ? `coupled: ${def.name}` : "switch refused", ok ? "info" : "warning");
    },
  });
}
