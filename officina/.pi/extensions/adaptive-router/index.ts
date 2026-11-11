import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import {
  getEngineSnapshot,
  onEngineUpdate,
  startEnginePolling,
} from "../_shared/engine.ts";
import { classifyTurn } from "../auto-route/classifier.ts";
import { extractCatalogModels, freeToolCandidates } from "../caduceus/catalog.ts";
import { canSpendCloud } from "../_shared/budget.ts";
import { readLivePeers, writeRegistryEntry } from "../_shared/routing-registry.ts";
import type { PeerView, SelfView } from "./peer-policy.ts";
import {
  cooling as healthCooling,
  recordFailure,
  recordRetryAfter,
  recordSuccess,
} from "../_shared/provider-health.ts";
import {
  configFromEnv,
  decideTier,
  localBusy,
  parseModelSpec,
  type OverflowConfig,
  type Privacy,
  type Tier,
} from "./overflow.ts";

// adaptive-router — local-first, cloud spillover (2026-11-10).
//
// Arms with OFFICINA_OVERFLOW=1. At each user-turn boundary it samples the
// local engine: if the slot is processing/queued (or a foreign tenant is
// filling the GPU), the turn spills to a cloud model. Cache-aware hysteresis
// keeps provider switching rare:
//   - decisions only at turn boundaries (never mid-turn)
//   - minimum dwell on a tier
//   - max switches per session
//   - return to local only after a long idle + a small cloud-token delta
//   - sensitive/confidential prompts never leave local
//
// Kill: OFFICINA_NO_OVERFLOW=1 (or unset OFFICINA_OVERFLOW).

type Model = NonNullable<ExtensionContext["model"]>;

const LOCAL_LIKE = /^(llamacpp|ollama|lmstudio|small-lane)$/i;
const CLOUD_RESERVE_TOKENS = 16_384;

export default function (pi: ExtensionAPI) {
  const armed =
    process.env.OFFICINA_OVERFLOW === "1" &&
    process.env.OFFICINA_NO_OVERFLOW !== "1";
  if (!armed) return;

  const cfg: OverflowConfig = configFromEnv();
  // P3 multi-session identity: stable for the lifetime of this TUI process.
  const sessionStartedAt = Date.now();
  const tuiPid = process.pid;
  const sessionRole: SelfView["role"] =
    process.env.OFFICINA_SESSION_ROLE === "subagent" ? "subagent" : "main";
  const cloudEnv = (process.env.OFFICINA_OVERFLOW_CLOUD || "free").trim();
  // Free-only (default ON): every lane must be a verified-free model
  // (zero-priced or costless owner). Explicit OFFICINA_OVERFLOW_CLOUD
  // specs are honoured only when free-only is disabled.
  const freeOnly = cfg.freeOnly ?? true;
  const discoverFree = freeOnly || /^(free|auto)$/i.test(cloudEnv);
  const explicitSpecs = (discoverFree ? "" : cloudEnv)
    .split(",")
    .map((s) => parseModelSpec(s))
    .filter((s): s is { provider: string; id: string } => !!s);
  const fallbackSpecs = (
    process.env.OFFICINA_OVERFLOW_CLOUD_FALLBACK ||
    // Catalog discovery is primary; these are the no-catalog backstop
    // (2026-11-11): the opencode-go subscription lane plus the two big
    // AgentRouter lanes (both tool-capable, ~1M ctx, zero-priced).
    "caduceus/opencode-go/deepseek-v4.1-flash," +
    "caduceus/opencode-go/qwen3.8-flash," +
    "caduceus/opencode-go/glm-5.2," +
    "caduceus/agentrouter/gpt-5.6-sol," +
    "caduceus/agentrouter/claude-opus-5"
  )
    .split(",")
    .map((s) => parseModelSpec(s))
    .filter((s): s is { provider: string; id: string } => !!s);
  const catalogPath =
    process.env.OFFICINA_OVERFLOW_CATALOG ||
    join(homedir(), ".vitriol", "officina", "caduceus-models.json");

  /** Free agent lanes discovered from Caduceus's cached catalog. */
  const discoverFreeModels = (): { provider: string; id: string }[] => {
    if (!discoverFree || !existsSync(catalogPath)) return [];
    try {
      const raw = JSON.parse(readFileSync(catalogPath, "utf8"));
      return freeToolCandidates(extractCatalogModels(raw)).map((c) => ({
        provider: "caduceus",
        id: c.id,
      }));
    } catch {
      return [];
    }
  };
  const localSpec = process.env.OFFICINA_OVERFLOW_LOCAL
    ? parseModelSpec(process.env.OFFICINA_OVERFLOW_LOCAL)
    : null;

  let enabled = true;
  let tier: Tier = "local";
  let switches = 0;
  let turnsOnTier = 0;
  let cloudTokens = 0;
  let cloudTokensSinceLocal = 0;
  let localIdleSince: number | null = Date.now();
  let applying = false;
  let manual = false;
  let localModel: Model | undefined;
  let cloudCandidates: Model[] = [];
  let activeCloud: Model | undefined;
  let failoverActive = false;
  let warnedCloud = false;
  let warnedLocal = false;
  const coolingUntil: Map<string, number> = new Map();

  startEnginePolling();
  onEngineUpdate(() => {
    const s = getEngineSnapshot();
    const busy =
      !s.up ||
      s.processing > 0 ||
      s.deferred > 0 ||
      s.stalled ||
      s.queueWaitMs >= cfg.queueWaitMs;
    if (busy) {
      localIdleSince = null;
    } else if (localIdleSince == null) {
      localIdleSince = Date.now();
    }
  });

  const reset = (): void => {
    tier = "local";
    switches = 0;
    turnsOnTier = 0;
    cloudTokens = 0;
    cloudTokensSinceLocal = 0;
    localIdleSince = Date.now();
    manual = false;
    failoverActive = false;
  };

  pi.on("session_start", () => {
    reset();
  });

  const sameModel = (a: Model | undefined, b: Model | undefined): boolean =>
    !!a && !!b && a.provider === b.provider && a.id === b.id;

  const isLocalModel = (m: Model | undefined): boolean =>
    !!m && (sameModel(m, localModel) || LOCAL_LIKE.test(m.provider));

  const resolveModels = (ctx: ExtensionContext): void => {
    if (!localModel) {
      if (localSpec) {
        localModel = ctx.modelRegistry.find(localSpec.provider, localSpec.id) ?? undefined;
      } else if (ctx.model && LOCAL_LIKE.test(ctx.model.provider)) {
        localModel = ctx.model;
      }
      if (!localModel) {
        localModel = ctx.modelRegistry.find("llamacpp", "Lapis Occultus") ?? undefined;
      }
    }
    if (cloudCandidates.length === 0) {
      const seen = new Set<string>();
      const specs = [...discoverFreeModels(), ...(freeOnly ? [] : explicitSpecs), ...fallbackSpecs];
      const maxCandidates = Math.max(
        1,
        Math.min(256, Number(process.env.OFFICINA_OVERFLOW_MAX_CANDIDATES) || 48),
      );
      const perOwner = Math.max(
        1,
        Math.min(32, Number(process.env.OFFICINA_OVERFLOW_PER_OWNER) || 8),
      );
      const ownerCounts = new Map<string, number>();
      const selected: typeof specs = [];
      for (const spec of specs) {
        const owner = (spec.id.split("/")[0] ?? "").toLowerCase();
        const count = ownerCounts.get(owner) ?? 0;
        if (count >= perOwner) continue;
        ownerCounts.set(owner, count + 1);
        selected.push(spec);
        if (selected.length >= maxCandidates) break;
      }
      cloudCandidates = selected
        .map((spec) => ctx.modelRegistry.find(spec.provider, spec.id) ?? undefined)
        .filter((m): m is Model => !!m)
        .filter((m) => {
          const key = `${m.provider}/${m.id}`;
          if (seen.has(key)) return false;
          seen.add(key);
          return true;
        });
      if (cloudCandidates.length === 0 && !warnedCloud) {
        warnedCloud = true;
        ctx.ui.notify(
          `overflow: none of the cloud candidates resolved — staying local`,
          "warning",
        );
      }
    }
  };

  const cloudKey = (m: Model): string => `${m.provider}/${m.id}`;
  const ownerKey = (m: Model): string =>
    `owner:${(m.id.split("/")[0] ?? "").toLowerCase()}`;

  const cooling = (m: Model, now: number): boolean =>
    (coolingUntil.get(cloudKey(m)) ?? 0) > now ||
    (coolingUntil.get(ownerKey(m)) ?? 0) > now;

  const cool = (m: Model, ms: number, wholeOwner = false): void => {
    const until = Date.now() + ms;
    coolingUntil.set(cloudKey(m), until);
    if (wholeOwner) coolingUntil.set(ownerKey(m), until);
  };

  /// First candidate that fits the context and is not cooling down.
  const pickCloud = (tokens: number, now: number): Model | undefined =>
    cloudCandidates.find((m) => {
      const fits = tokens + CLOUD_RESERVE_TOKENS < (m.contextWindow ?? 0);
      return fits && !cooling(m, now);
    });

  const statusLine = (queueMs: number): string => {
    if (manual) return "route manual — overflow paused";
    const q = queueMs > 0 ? ` · q ${(queueMs / 1000).toFixed(1)}s` : "";
    const sw = switches > 0 ? ` · sw ${switches}` : "";
    const lane = tier === "cloud" && activeCloud
      ? ` · ${activeCloud.id.split("/").pop()}`
      : "";
    const tk = tier === "cloud" ? ` · ${Math.round(cloudTokens / 1000)}k tok` : "";
    const fo = failoverActive ? " · failover" : "";
    return `route ${tier}${lane}${q}${sw}${tk}${fo}`;
  };

  const renderStatus = (ctx: ExtensionContext, queueMs: number): void => {
    ctx.ui.setStatus("overflow", statusLine(queueMs));
    ctx.ui.setWidget("overflow-route", [statusLine(queueMs)], {
      placement: "belowEditor",
    });
  };

  pi.on("before_agent_start", async (event, ctx) => {
    if (!enabled) return undefined;
    // The resilient virtual default owns its own routing; do not fight it
    // with pi.setModel() turn-boundary switches.
    if (ctx.model?.api === "pi-virtual" || ctx.model?.provider === "officina") {
      return undefined;
    }
    resolveModels(ctx);
    if (!localModel) {
      if (!warnedLocal) {
        warnedLocal = true;
        ctx.ui.notify(
          "overflow: local model not resolved — routing disabled for this turn",
          "warning",
        );
      }
      renderStatus(ctx, 0);
      return undefined;
    }

    const prompt =
      event.messageType === "user" ? event.prompt : "";
    const cls = prompt.trim()
      ? classifyTurn({
          promptText: prompt,
          recentErrorCount: 0,
          churnLoops: 0,
          scratchpadContextLines: [],
          filesTouched: 0,
          turnCount: 0,
        })
      : null;
    const privacy: Privacy = cls ? cls.privacy : "safe";

    const snap = getEngineSnapshot();
    // Before the first poll completes, be optimistic (local). Treating the
    // unknown window as BUSY sent the first turn to cloud even on a healthy
    // engine; true DOWN is handled once `polled` is set.
    const load = {
      processing: snap.polled ? (snap.up ? snap.processing : 1) : 0,
      deferred: snap.polled ? (snap.up ? snap.deferred : 0) : 0,
      queueWaitMs: snap.polled ? (snap.up ? snap.queueWaitMs : 0) : 0,
      stalled: snap.polled && snap.stalled,
      gpuLoad: snap.gpuLoad,
    };
    const usage = ctx.getContextUsage();
    const now = Date.now();
    const localDown = cfg.failover && snap.polled && !snap.up;
    const candidate = pickCloud(usage?.tokens ?? 0, now);
    const cloudFits = !!candidate;
    const budgetLeft = cloudTokens < cfg.cloudTokenBudget && canSpendCloud();
    const idleFor = localIdleSince != null ? Date.now() - localIdleSince : null;

    // P3: peer arbitration needs an observable engine. Once /slots has
    // answered even once, the (persisting) slot table stays usable through
    // queue-backed stalls — withholding the peer arm exactly while the
    // engine is busy was backwards (2026-11-11).
    const slotCount = snap.slots?.length ?? 0;
    const peersUsable =
      snap.polled && snap.up && (slotCount > 0 || snap.slotsSeen === true);
    const peerContext = peersUsable
      ? {
          self: {
            ts: sessionStartedAt,
            tuiPid,
            ctxTokens: usage?.tokens ?? 0,
            role: sessionRole,
          } as SelfView,
          peers: readLivePeers(tuiPid, { now }) as PeerView[],
          freeSlots: Math.max(0, slotCount - (snap.processing ?? 0)),
        }
      : {};

    let decision = decideTier(
      {
        state: { tier, switches, turnsOnTier, cloudTokens: cloudTokensSinceLocal },
        load,
        privacy,
        cloudFits,
        budgetLeft,
        localIdleForMs: idleFor,
        localDown,
        failoverActive,
        complexity: cls ? cls.complexity : undefined,
        ...peerContext,
      },
      cfg,
    );

    let switched = false;
    // Publish this session's decision so peers can rank against it. Peer
    // visibility is best-effort: a failed write must never break a turn.
    try {
      const sessionId = ((ctx as { sessionManager?: { getSessionFile?: () => string } })
        .sessionManager?.getSessionFile?.() ?? `tui-${tuiPid}`)
        .split("/")
        .pop() as string;
      writeRegistryEntry(
        {
          ts: now,
          tuiPid,
          sessionId,
          tier: decision.tier,
          model:
            decision.tier === "cloud" && candidate
              ? `${candidate.provider}/${candidate.id}`
              : ctx.model
                ? `${ctx.model.provider}/${ctx.model.id}`
                : "local",
          ctxTokens: usage?.tokens ?? 0,
          role: sessionRole,
        },
        undefined,
        now,
      );
    } catch {
      // registry is advisory only
    }
    // Cloud tier may need to rotate to a different candidate after a 429 or
    // when the context outgrows a 65k free model.
    let targetCloud = decision.tier === "cloud" ? candidate : undefined;
    if (tier === "cloud" && decision.tier === "cloud" && !targetCloud) {
      targetCloud = undefined; // no viable candidate; fall through to local
    }
    if (decision.tier === "cloud" && targetCloud && !sameModel(activeCloud, targetCloud)) {
      applying = true;
      const ok = await pi.setModel(targetCloud);
      applying = false;
      if (ok) {
        tier = "cloud";
        activeCloud = targetCloud;
        // Emergency failover AND peer-arbitration demotions must not consume
        // the cache-affinity switch cap (availability / slot sharing wins).
        const emergency = failoverActive || localDown || decision.peerDemoted === true;
        if (!emergency) switches += 1;
        if (localDown) failoverActive = true;
        turnsOnTier = 1;
        cloudTokensSinceLocal = 0;
        switched = true;
      } else {
        cool(targetCloud, 60 * 60_000, true);
        ctx.ui.notify(
          `overflow: could not switch to ${targetCloud.provider}/${targetCloud.id} (auth?) — cooling 1h`,
          "warning",
        );
      }
    } else if (decision.tier === "cloud" && !targetCloud && tier === "cloud") {
      // The active cloud lane is no longer viable (context/cooling/budget).
      decision.tier = "local";
    } else if (decision.tier !== tier && decision.tier === "local") {
      applying = true;
      const ok = await pi.setModel(localModel);
      applying = false;
      if (ok) {
        tier = "local";
        activeCloud = undefined;
        // Returning home RESETS the cap (2026-11-11): the cap exists to
        // damp oscillation, not to permanently strand a session after one
        // spill+return cycle — charging the return too left sessions
        // inert after two contention events.
        switches = 0;
        failoverActive = false;
        turnsOnTier = 1;
        cloudTokensSinceLocal = 0;
        switched = true;
      } else {
        ctx.ui.notify("overflow: could not switch back to local", "warning");
      }
    } else {
      turnsOnTier += 1;
      // Slow cap recovery: back on an idle local for a full return window,
      // expired oscillation debt stops counting against future spills.
      if (
        tier === "local" &&
        switches > 0 &&
        !localBusy(load, cfg) &&
        idleFor != null &&
        idleFor >= cfg.returnIdleMs
      ) {
        switches = 0;
      }
    }

    if (localDown && decision.tier === "cloud") {
      failoverActive = true;
    }
    if (switched) {
      emitHarnessEvent(
        harnessEvent("lc-overflow", "switch", {
          detail: `${decision.tier}: ${decision.reason}`,
        }),
      );
      ctx.ui.notify(
        `overflow → ${decision.tier}: ${decision.reason}`,
        decision.tier === "cloud" ? "warning" : "info",
      );
    }
    renderStatus(ctx, load.queueWaitMs);
    return undefined;
  });

  pi.on("model_select", (event) => {
    if (applying || event.source === "restore") return;
    if (event.model?.api === "pi-virtual" || event.model?.provider === "officina") return;
    // llama-cpp-provider's startup sync selects the local alias. That is not
    // a user override.
    if (isLocalModel(event.model)) {
      tier = "local";
      failoverActive = false;
      return;
    }
    if (cloudCandidates.some((m) => sameModel(event.model, m))) {
      tier = "cloud";
      activeCloud = event.model;
      return;
    }
    manual = true;
  });

  // Provider failures cool the cloud lane down instead of hammering it.
  pi.on("after_provider_response", (event) => {
    if (tier !== "cloud" || !activeCloud) return;
    if (event.status && event.status >= 400) {
      recordFailure(activeCloud.provider, activeCloud.id, event.status, {
        retryAfterMs: recordRetryAfter(activeCloud.provider, activeCloud.id, event.headers ?? {}),
      });
    }
    if (event.status === 404 || event.status === 410) {
      cool(activeCloud, 60 * 60_000);
    } else if (event.status === 401 || event.status === 403) {
      // Missing/expired credentials: cool the whole owner for an hour.
      cool(activeCloud, 60 * 60_000, true);
    } else if (event.status === 429) {
      // Respect provider quota hints (seconds or HTTP-date), clamp 1m..24h.
      const headers = event.headers ?? {};
      const hint =
        headers["retry-after"] ??
        headers["Retry-After"] ??
        headers["x-ratelimit-reset"];
      let ms = 30 * 60_000;
      if (hint && /^\d+$/.test(hint.trim())) {
        ms = Math.min(24 * 3600_000, Math.max(60_000, Number(hint.trim()) * 1000));
      } else if (hint) {
        const at = Date.parse(hint);
        if (Number.isFinite(at)) {
          ms = Math.min(24 * 3600_000, Math.max(60_000, at - Date.now()));
        }
      }
      // GLM/Z.ai quota is account-wide; OpenCode Go limits can be per-model.
      const owner = (activeCloud.id.split("/")[0] ?? "").toLowerCase();
      cool(activeCloud, ms, owner === "glm" || owner === "zai");
    } else if (event.status >= 500) {
      cool(activeCloud, 5 * 60_000, true);
    }
  });

  pi.on("message_end", (event, ctx) => {
    const m = event.message as { role?: string; provider?: string; model?: string; stopReason?: string };
    if (m.role === "assistant" && m.provider && m.model) {
      if (m.stopReason === "error") recordFailure(m.provider, m.model, undefined, { errorClass: "completion_error" });
      else recordSuccess(m.provider, m.model);
    }
    if (tier !== "cloud") return;
    const msg = event.message as {
      role?: string;
      usage?: {
        input?: number;
        output?: number;
        cacheRead?: number;
        cacheWrite?: number;
        totalTokens?: number;
      };
    };
    if (msg.role !== "assistant") return;
    if ((msg as { stopReason?: string }).stopReason === "error") {
      if (activeCloud) {
        cool(activeCloud, 60_000);
      }
    }
    if (!msg.usage) return;
    const u = msg.usage;
    const n =
      u.totalTokens ??
      (u.input ?? 0) + (u.output ?? 0) + (u.cacheRead ?? 0) + (u.cacheWrite ?? 0);
    cloudTokens += n;
    cloudTokensSinceLocal += n;
    renderStatus(ctx, getEngineSnapshot().queueWaitMs);
  });

  pi.registerCommand("overflow-catalog", {
    description: "List Caduceus free/tool-capable overflow candidates and cooldowns",
    handler: async (_args: string, ctx: { ui: any }) => {
      resolveModels(ctx as unknown as ExtensionContext);
      if (cloudCandidates.length === 0) {
        await ctx.ui.notify?.("overflow: no cloud candidates (catalog/key missing?)", "warning");
        return;
      }
      const now = Date.now();
      const lines = cloudCandidates.map((m, i) => {
        const until = Math.max(
          coolingUntil.get(cloudKey(m)) ?? 0,
          coolingUntil.get(ownerKey(m)) ?? 0,
        );
        const cool = until > now ? ` · cool ${Math.ceil((until - now) / 60_000)}m` : "";
        return `${i + 1}. ${m.id} · ctx ${Math.round((m.contextWindow ?? 0) / 1000)}k${cool}`;
      });
      await ctx.ui.notify?.(`overflow candidates:\n${lines.join("\n")}`, "info");
    },
  });

  pi.registerCommand("overflow", {
    description: "Overflow routing: show status, or /overflow on|off",
    handler: async (args: string, ctx: { ui: any }) => {
      const a = args.trim().toLowerCase();
      if (a === "on" || a === "off") {
        enabled = a === "on";
        if (!enabled) {
          resolveModels(ctx as unknown as ExtensionContext);
          if (tier === "cloud" && localModel) {
            applying = true;
            const ok = await pi.setModel(localModel);
            applying = false;
            if (ok) {
              const wasFailover = failoverActive;
              tier = "local";
              activeCloud = undefined;
              failoverActive = false;
              if (!wasFailover) switches += 1;
              turnsOnTier = 1;
            }
          }
        }
        await ctx.ui.notify?.(`overflow routing ${enabled ? "on" : "off"}`, "info");
        return;
      }
      const snap = getEngineSnapshot();
      await ctx.ui.notify?.(
        `overflow: ${enabled ? "on" : "off"} · tier ${tier} · switches ${switches}/${cfg.maxSwitches} · cloud ${Math.round(cloudTokens / 1000)}k tok · queue ${(snap.queueWaitMs / 1000).toFixed(1)}s`,
        "info",
      );
    },
  });
}
