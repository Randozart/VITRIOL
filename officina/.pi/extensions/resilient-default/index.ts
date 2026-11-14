import type {
  ExtensionAPI,
  ExtensionContext,
  ModelRoute,
  ModelRouteRequest,
} from "@earendil-works/pi-coding-agent";
import { existsSync, readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { getEngineSnapshot, onEngineUpdate, startEnginePolling } from "../_shared/engine.ts";
import { classifyTurn } from "../auto-route/classifier.ts";
import { configFromEnv, decideTier, parseModelSpec, type Tier } from "../adaptive-router/overflow.ts";
import type { PeerView, SelfView } from "../adaptive-router/peer-policy.ts";
import { extractCatalogModels, toolCandidates } from "../caduceus/catalog.ts";
import {
  allowPaidOwners,
  applyPrefs,
  capCandidates,
  prefsPath,
  readPrefs,
} from "../_shared/caduceus-prefs.ts";
import {
  budgetStatus,
  budgetSummaryText,
  canSpendCloud,
} from "../_shared/budget.ts";
import { readLivePeers, writeRegistryEntry } from "../_shared/routing-registry.ts";
import { busyPeerLanes, spreadPick } from "./lane-spread.ts";
import { isVerified } from "../_shared/probe-scheduler.ts";
import { admitLane, effectiveWindow } from "../_shared/admission.ts";
import {
  markGatewayCooldown,
  markAlias,
  markGatewayInvalid,
  markModelLimit,
  markOffered,
  markOwnerPause,
  ownerOfLane,
  parseContextLimit,
  parseCooldownFromError,
  parseOverflowPromptTokens,
  parseModelNotFound,
  parseOwnerPause,
  readGatewayMarks,
} from "../_shared/gateway-health.ts";
import { formatLanes, getLaneHealth, type LaneRow } from "../_shared/lane-health.ts";
import {
  cooldownRemaining,
  cooling as healthCooling,
  recordFailure,
  recordRetryAfter,
  recordSuccess,
} from "../_shared/provider-health.ts";
import {
  lastRouteEntry,
  recordRoute,
  todaySummary,
  type RouteLedgerEntry,
} from "../_shared/route-ledger.ts";
import {
  profileAllowsCloud,
  readProfile,
  sortIdsForProfile,
  writeProfile,
  type RoutingProfile,
} from "../_shared/profiles.ts";

// resilient-default — the always-selectable virtual model `officina/lapis`
// ("Lapis Occultus"). It is keyless and therefore present in /model even when
// the VITRIOL engine or the Caduceus gateway is down; each request routes to a
// physical model:
//   user turn   local if up/idle, free/cloud if busy, cloud failover if down
//   continuation stick to the previous physical model (tool-loop cache)
//   retry       local failure -> cloud; cloud failure -> next candidate/local
//   direct      local if up, else cloud
// Sensitive/confidential turns never leave local.
//
// Multi-session (2026-11-11): lapis is the DEFAULT model, and this is the
// only router that runs under it — so the P1/P3 peer arbitration lives HERE
// too: peer context from the routing registry + /slots feeds decideTier,
// decisions publish to the registry, and peer demotions / local returns
// respect the arbitration switch-cap rules (adaptive-router behaviour
// mirrors it for non-virtual selections).

type Model = NonNullable<ExtensionContext["model"]>;

interface LapisState {
  tier: Tier;
  switches: number;
  turnsOnTier: number;
  failoverActive: boolean;
  /** Cloud attempts spent in the current turn (retry budget, 2026-11-14). */
  cloudRetries?: number;
}

/** Nimbus lane state — cloud-only, no arbitration bookkeeping needed. */
interface NimbusState {
  turnsOnTier: number;
}

interface LastDecision {
  reason: string;
  tier: Tier;
  provider: string;
  model: string;
  profile: RoutingProfile;
  localState: "up" | "down" | "busy" | "unknown";
  startedAt: number;
}

const CLOUD_RESERVE_TOKENS = 16_384;
const CATALOG = join(homedir(), ".vitriol", "officina", "caduceus-models.json");
const DEFAULT_FALLBACKS = [
  // No-catalog backstop: free-tier lanes first (2026-11-13, operator:
  // "opencode go is paid, I have a subscription" — paid lanes go last).
  "caduceus/agentrouter/gpt-5.6-sol",
  "caduceus/agentrouter/claude-opus-5",
  "caduceus/opencode-go/deepseek-v4.1-flash",
  "caduceus/opencode-go/qwen3.8-flash",
  "caduceus/opencode-go/glm-5.2",
];

const initial = (): LapisState => ({
  tier: "local",
  switches: 0,
  turnsOnTier: 0,
  failoverActive: false,
  cloudRetries: 0,
});

function messageText(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((part) => {
      if (typeof part === "string") return part;
      if (part && typeof part === "object" && (part as { type?: string }).type === "text") {
        return String((part as { text?: unknown }).text ?? "");
      }
      return "";
    })
    .join("\n");
}

/** chars/4 estimate over the request messages (text, thinking, tool args). */
function estimatePromptTokens(messages: ReadonlyArray<unknown>): number {
  let chars = 0;
  for (const raw of messages) {
    const m = raw as { content?: unknown };
    const content = m?.content;
    if (typeof content === "string") {
      chars += content.length;
    } else if (Array.isArray(content)) {
      for (const part of content) {
        const p = part as { text?: unknown; thinking?: unknown; input?: unknown };
        if (typeof p?.text === "string") chars += p.text.length;
        else if (typeof p?.thinking === "string") chars += p.thinking.length;
        else if (p?.input !== undefined) chars += JSON.stringify(p.input).length;
      }
    }
  }
  return Math.ceil(chars / 4);
}

function latestUserText(messages: readonly unknown[]): string {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i] as { role?: string; content?: unknown };
    if (m?.role === "user") return messageText(m.content);
  }
  return "";
}

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_RESILIENT_DEFAULT === "0") return;
  const cfg = configFromEnv();
  const fallbacks = (process.env.OFFICINA_OVERFLOW_CLOUD_FALLBACK || DEFAULT_FALLBACKS.join(","))
    .split(",")
    .map((s) => parseModelSpec(s))
    .filter((s): s is { provider: string; id: string } => !!s);

  // P3 identity — stable for this TUI process (mirrors adaptive-router).
  const sessionStartedAt = Date.now();
  const tuiPid = process.pid;
  const sessionRole: SelfView["role"] =
    process.env.OFFICINA_SESSION_ROLE === "subagent" ? "subagent" : "main";

  let localIdleSince: number | null = Date.now();
  let cloudCandidates: Model[] = [];
  let catalogLoaded = false;
  let healthSig = ""; // gateway lane-health marking the pool was built against
  let activeCloud: Model | undefined;
  let lastDecision: LastDecision | null = null;

  startEnginePolling();
  onEngineUpdate(() => {
    const s = getEngineSnapshot();
    const busy =
      !s.up || s.processing > 0 || s.deferred > 0 || s.stalled || s.queueWaitMs >= cfg.queueWaitMs;
    if (busy) localIdleSince = null;
    else if (localIdleSince == null) localIdleSince = Date.now();
  });

  const loadCandidates = async (ctx: ExtensionContext): Promise<void> => {
    // Gateway lane health (2026-11-11) and routing prefs (2026-11-12):
    // rebuild the pool when either marking changes, not per request.
    const health = await getLaneHealth();
    const prefs = readPrefs();
    let prefsStamp = 0;
    try {
      prefsStamp = statSync(prefsPath()).mtimeMs;
    } catch {
      // no prefs file → today's behaviour (I1)
    }
    const sig = `${[...health.hardDown, ...health.backoff].sort().join(",")}|prefs:${prefsStamp}`;
    if (catalogLoaded && sig === healthSig) return;
    catalogLoaded = true;
    healthSig = sig;
    cloudCandidates = [];
    const profile = readProfile();
    const ownerOf = (spec: { id: string }): string => (spec.id.split("/")[0] ?? "").toLowerCase();
    const pool: Array<{
      provider: string;
      id: string;
      owner: string;
      free: boolean;
      freeTier: boolean;
    }> = [];
    try {
      if (existsSync(CATALOG)) {
        const raw = JSON.parse(readFileSync(CATALOG, "utf8"));
        const found = toolCandidates(extractCatalogModels(raw), {
          freeOnly: prefs?.free_only !== false,
          allowPaidOwners: allowPaidOwners(prefs),
        });
        pool.push(
          ...found.map((c) => ({
            provider: "caduceus",
            id: c.id,
            owner: c.owner,
            free: c.free,
            freeTier: c.freeTier,
          })),
        );
      }
    } catch {
      // fall through to explicit fallbacks
    }
    // Explicit fallbacks keep their backstop role; policy still applies to
    // them (routing off / deny / rank), never enablement (I2).
    pool.push(...fallbacks.map((s) => ({ ...s, owner: ownerOf(s), free: false, freeTier: false })));
    // Two-class ordering (2026-11-13): free-tier lanes first, paid
    // subscription lanes after — a free lane exhausting (cooling/429)
    // walks the router down the line into the paid layer.
    const freeTierPool = pool.filter((c) => c.freeTier);
    const paidPool = pool.filter((c) => !c.freeTier);
    const ordered = [
      ...applyPrefs(sortIdsForProfile(freeTierPool, profile), prefs),
      ...applyPrefs(sortIdsForProfile(paidPool, profile), prefs),
    ];
    // Healthy lanes first, gateway-backoff lanes demoted to the tail,
    // gateway-hard-down owners excluded entirely (fail-open: owners the
    // gateway doesn't know about are treated as healthy).
    const healthy = ordered.filter(
      (s) => !health.hardDown.has(s.owner) && !health.backoff.has(s.owner),
    );
    const demoted = ordered.filter((s) => health.backoff.has(s.owner));
    const seen = new Set<string>();
    const unique = [...healthy, ...demoted].filter((s) => {
      const key = `${s.provider}/${s.id}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
    const envCaps = {
      perOwner: Math.max(1, Math.min(32, Number(process.env.OFFICINA_OVERFLOW_PER_OWNER) || 8)),
      max: Math.max(1, Math.min(256, Number(process.env.OFFICINA_OVERFLOW_MAX_CANDIDATES) || 48)),
    };
    // Resolve before capping: unresolvable/unauthenticated lanes must not
    // consume per-owner slots (historical behaviour, pinned by tests).
    const resolved = unique
      .map((spec) => ({ spec, model: ctx.modelRegistry.find(spec.provider, spec.id) }))
      .filter(
        (x): x is { spec: typeof unique[number]; model: NonNullable<typeof x.model> } =>
          !!x.model && ctx.modelRegistry.hasConfiguredAuth(x.model),
      );
    cloudCandidates = capCandidates(
      resolved.map((r) => ({ id: r.spec.id, owner: r.spec.owner, model: r.model })),
      prefs,
      envCaps,
    ).map((c) => c.model);
  };

  const maxCloudRetries = (): number => {
    const n = Number(process.env.OFFICINA_CLOUD_RETRIES);
    return Number.isFinite(n) && n >= 0 ? Math.floor(n) : 2;
  };

  /** Lanes allowed by the gateway facts (2026-11-14): invalid, cooling,
   *  paused owners, learned windows and the owner's safe-byte ceiling. */
  const cloudEligible = (tokens: number, now: number): Model[] => {
    let marks = {
      invalid: new Set<string>(),
      models: new Set<string>(),
      owners: new Set<string>(),
      pausedOwners: new Set<string>(),
      safeBytesByOwner: {} as Record<string, number>,
      limits: {} as Record<string, number>,
      offered: {} as Record<string, string[]>,
      aliases: {} as Record<string, string>,
      probes: {} as Record<string, import("../_shared/gateway-health.ts").ProbeFact>,
    };
    if (process.env.OFFICINA_NO_GATEWAY_MARKS !== "1") {
      try {
        marks = readGatewayMarks(now) as typeof marks;
      } catch {
        // fail-open
      }
    }
    const admitted = (m: Model): boolean => {
      const owner = ownerOfLane(m.id);
      return (
        admitLane({
          promptTokens: tokens,
          reserveTokens: CLOUD_RESERVE_TOKENS,
          lane: {
            invalid: marks.invalid.has(m.id) || marks.models.has(m.id) || marks.owners.has(owner),
            cooling: healthCooling(m.provider, m.id, now),
            ownerPaused: marks.pausedOwners.has(owner),
          },
          declared: Number(m.contextWindow ?? 0),
          learned: marks.limits[m.id] ?? null,
          safeBytes: marks.safeBytesByOwner[owner] ?? null,
        }) === "admit"
      );
    };
    const eligible = cloudCandidates.filter(admitted);
    // Learned remaps: prefer the offered replacement when it is admitted.
    const seen = new Set<string>();
    const remapped: Model[] = [];
    for (const m of eligible) {
      const rep = marks.aliases[m.id];
      const chosen = rep ? eligible.find((x) => x.id === rep) ?? m : m;
      if (!seen.has(chosen.id)) {
        seen.add(chosen.id);
        remapped.push(chosen);
      }
    }
    // Probe preference (2026-11-14): lanes with a fresh successful probe rank
    // first; lanes a probe found invalid/paused/cooling are excluded even if
    // provider-health has not caught up yet.
    const probeAllowed = remapped.filter((m) => {
      const fact = marks.probes[m.id];
      if (!fact) return true;
      if (fact.kind === "invalid") return false;
      if ((fact.kind === "cooling" || fact.kind === "paused") && Number(fact.until ?? 0) > now) {
        return false;
      }
      return true;
    });
    probeAllowed.sort(
      (a, b) =>
        Number(!isVerified(a.id, marks.probes, now)) - Number(!isVerified(b.id, marks.probes, now)),
    );
    return probeAllowed;
  };

  const pickCloud = (tokens: number, now: number): Model | undefined => {
    const eligible = cloudEligible(tokens, now);
    // Multi-session quota spread (2026-11-14): never stampede the same free
    // lane the other live sessions are already turning on; the registry is
    // written by adaptive-router and expires in 60 s.
    let busy = new Set<string>();
    if (process.env.OFFICINA_NO_LANE_SPREAD !== "1") {
      try {
        busy = busyPeerLanes(readLivePeers(process.ppid, { now }));
      } catch {
        // registry unavailable → plain priority order
      }
    }
    return spreadPick(eligible, busy);
  };

  // Publish this lane's decision so peer sessions can rank against us
  // (P1/I7 shard registry). Best-effort: a failed write never breaks a
  // request. Shared by both lanes (lapis route closure + nimbus).
  const publishPeerDecision = (
    tier: Tier,
    chosen: Model,
    tokens: number,
    ctx: ExtensionContext,
    now: number,
  ): void => {
    try {
      const sessionId =
        (ctx as { sessionManager?: { getSessionFile?: () => string } })
          .sessionManager?.getSessionFile?.()
          ?.split("/")
          .pop() ?? `tui-${tuiPid}`;
      writeRegistryEntry(
        {
          ts: now,
          tuiPid,
          sessionId,
          tier,
          model: `${chosen.provider}/${chosen.id}`,
          ctxTokens: tokens,
          role: sessionRole,
        },
        undefined,
        now,
      );
    } catch {
      // registry is advisory only
    }
  };

  const setRouteWidget = (
    ctx: ExtensionContext,
    tier: Tier,
    model?: Model,
    profile = readProfile(),
    lane = "lapis",
  ): void => {
    const text =
      tier === "cloud" && model
        ? `route ${lane} · ${model.id.split("/").pop()} · ${profile}`
        : `route ${lane} · local · ${profile}`;
    ctx.ui.setStatus("overflow", text);
    ctx.ui.setWidget("overflow-route", [text], { placement: "belowEditor" });
  };

  const route = async (
    request: ModelRouteRequest<LapisState>,
    ctx: ExtensionContext,
  ): Promise<ModelRoute<LapisState>> => {
    await loadCandidates(ctx);
    const now = Date.now();
    const profile = readProfile();
    // Local resolution (2026-11-14): the engine ignores the request's model
    // id (verified: stale and bogus ids both complete), so ANY registered
    // llamacpp model is routable. Alias first, then the first available
    // llamacpp model — stale sessions stay usable after an engine restart
    // even when the startup alias probe failed.
    const local =
      ctx.modelRegistry.find("llamacpp", "Lapis Occultus") ??
      ctx.modelRegistry.getAvailable().find((m) => m.provider === "llamacpp");
    const snap = getEngineSnapshot();
    const localDown = cfg.failover && snap.polled && !snap.up;
    const load = {
      processing: snap.polled ? (snap.up ? snap.processing : 1) : 0,
      deferred: snap.polled ? (snap.up ? snap.deferred : 0) : 0,
      queueWaitMs: snap.polled ? (snap.up ? snap.queueWaitMs : 0) : 0,
      stalled: snap.polled && snap.stalled,
      gpuLoad: snap.gpuLoad,
    };
    // Prompt size, not just session usage (2026-11-14): during overflow
    // recovery the session counter lags the request (171k prompt read as
    // ~165k), so a lane that cannot fit was still selected. Use the larger
    // of the reported usage and a message estimate.
    const failedMessage = request.failed?.message as { errorMessage?: string } | undefined;
    const failureText = failedMessage?.errorMessage;
    const overflowTokens = parseOverflowPromptTokens(failureText);
    const learnedLimit = parseContextLimit(failureText);
    const estimated = estimatePromptTokens(request.messages);
    const tokens = Math.max(ctx.getContextUsage()?.tokens ?? 0, estimated, overflowTokens ?? 0);
    const candidate =
      profileAllowsCloud(profile) && canSpendCloud() ? pickCloud(tokens, now) : undefined;
    const previous = request.previous?.model;
    const failed = request.failed?.model;
    const cls = latestUserText(request.messages)
      ? classifyTurn({
          promptText: latestUserText(request.messages),
          recentErrorCount: 0,
          churnLoops: 0,
          scratchpadContextLines: [],
          filesTouched: 0,
          turnCount: 0,
        })
      : null;
    const privacy = cls ? cls.privacy : "safe";
    let state = request.state ?? initial();
    if (request.reason === "user") state = { ...state, cloudRetries: 0 };
    let tier: Tier = state.tier;

    const remember = (chosen: Model, chosenTier: Tier): void => {
      const stateKind: LastDecision["localState"] = localDown
        ? "down"
        : load.processing > 0 || load.deferred > 0 || load.queueWaitMs >= cfg.queueWaitMs
          ? "busy"
          : "up";
      lastDecision = {
        reason: request.reason,
        tier: chosenTier,
        provider: chosen.provider,
        model: chosen.id,
        profile,
        localState: stateKind,
        startedAt: now,
      };
      publishPeerDecision(chosenTier, chosen, tokens, ctx, now);
    };

    if (request.reason === "continuation" && previous && previous.id !== "lapis") {
      // Never leave the current physical model mid tool-loop.
      remember(previous, previous.provider === "llamacpp" ? "local" : "cloud");
      return { model: previous, thinkingLevel: request.thinkingLevel, state: request.state };
    }

    if (learnedLimit && failed) markModelLimit(failed.id, learnedLimit);

    if (request.reason === "retry") {
      // Retry budget (2026-11-14): a turn must not walk the pool one 429 at a
      // time. After N cloud attempts, local (or one actionable error).
      if ((state.cloudRetries ?? 0) >= maxCloudRetries()) {
        if (local) {
          activeCloud = undefined;
          remember(local, "local");
          setRouteWidget(ctx, "local", undefined, profile);
          return {
            model: local,
            thinkingLevel: request.thinkingLevel,
            state: { ...state, tier: "local", failoverActive: true },
          };
        }
        throw new Error(
          `Lapis Occultus: cloud retry budget exhausted (${maxCloudRetries()}) and no local model is routable`,
        );
      }
      // Context overflow (2026-11-14): another cloud lane that fits is a
      // lottery; the local stone has 262k and is the deterministic answer
      // when the prompt no longer fits the failed lane.
      if ((overflowTokens || learnedLimit) && local && local.provider === "llamacpp") {
        if (state.tier !== "local") {
          ctx.ui?.notify?.(
            "route: local (Lapis Occultus) after context overflow — the first turn re-prefills",
            "info",
          );
        }
        activeCloud = undefined;
        remember(local, "local");
        setRouteWidget(ctx, "local", undefined, profile);
        return {
          model: local,
          thinkingLevel: request.thinkingLevel,
          state: { ...state, tier: "local", failoverActive: true },
        };
      }
      if (failed && failed.provider === "llamacpp") {
        recordFailure(failed.provider, failed.id, undefined, { errorClass: "local_retry" });
        tier = candidate ? "cloud" : "local";
        state = { ...state, failoverActive: true };
      } else if (failed && failed.provider === "caduceus") {
        // The gateway puts its cooldown in the 429 BODY (reset_seconds /
        // retry_after / reset_at), not in a header — record it once, cap it,
        // and share the mark with every other session.
        const errorText = failureText;
        const pause = parseOwnerPause(errorText);
        const cooldownMs = parseCooldownFromError(errorText, now);
        const notFound = parseModelNotFound(errorText);
        recordFailure(failed.provider, failed.id, undefined, {
          errorClass: notFound ? "model_not_found" : cooldownMs ? "model_cooldown" : "cloud_retry",
          retryAfterMs: cooldownMs,
        });
        if (cooldownMs) {
          markGatewayCooldown({
            model: failed.id,
            owner: ownerOfLane(failed.id),
            untilMs: now + cooldownMs,
            reason: "gateway 429 body",
          });
        }
        if (notFound) {
          // Self-heal (2026-11-14): the catalog advertised an id the upstream
          // does not offer. Exclude that exact lane for hours (shared with
          // every session), remember the owner's offered ids, and fail over
          // to a free offered alternative IN THIS TURN when one is admitted.
          markGatewayInvalid({
            model: failed.id,
            reason: `not offered; upstream: ${notFound.alternatives.join(", ").slice(0, 200) || "unknown"}`,
          });
          const owner = ownerOfLane(failed.id);
          markOffered(owner, notFound.alternatives);
          const offered = notFound.alternatives.map((alt) => `${owner}/${alt}`);
          const eligibleNow = cloudEligible(tokens, now);
          const replacement = eligibleNow.find((m) => offered.includes(m.id));
          if (replacement) {
            markAlias(failed.id, replacement.id);
            const next = { ...state, cloudRetries: (state.cloudRetries ?? 0) + 1 };
            activeCloud = replacement;
            remember(replacement, "cloud");
            setRouteWidget(ctx, "cloud", replacement, profile);
            return { model: replacement, thinkingLevel: request.thinkingLevel, state: next };
          }
        }
        if (pause) {
          // A 400 key circuit is OWNER-wide (operator decision): bench every
          // model of that owner for the stated pause and honour safe bytes.
          markOwnerPause(ownerOfLane(failed.id), pause, "400 key pause");
        }
        const next = pickCloud(tokens, now);
        tier = next ? "cloud" : "local";
        if (next) state = { ...state, failoverActive: true };
      } else {
        tier = candidate && localDown ? "cloud" : "local";
      }
    } else if (request.reason === "direct") {
      tier = localDown ? "cloud" : "local";
    } else {
      // P3 peer arbitration (2026-11-11): lapis is the default model and
      // this is the only router under it, so the peer arm lives here.
      // Same gate as adaptive-router: once /slots answered, the persisting
      // slot table arbitrates through stalls.
      const slotCount = snap.slots?.length ?? 0;
      const peersUsable =
        snap.polled && snap.up && (slotCount > 0 || snap.slotsSeen === true);
      const peerContext = peersUsable
        ? {
            self: {
              ts: sessionStartedAt,
              tuiPid,
              ctxTokens: tokens,
              role: sessionRole,
            } as SelfView,
            peers: readLivePeers(tuiPid, { now }) as PeerView[],
            freeSlots: Math.max(0, slotCount - (snap.processing ?? 0)),
          }
        : {};
      const decision = decideTier(
        {
          state: {
            tier: state.tier,
            switches: state.switches,
            turnsOnTier: state.turnsOnTier,
            cloudTokens: 0,
          },
          load,
          privacy,
          cloudFits: !!candidate,
          budgetLeft: !!candidate,
          localIdleForMs: localIdleSince != null ? now - localIdleSince : null,
          localDown,
          failoverActive: state.failoverActive,
          complexity: cls ? cls.complexity : undefined,
          ...peerContext,
        },
        cfg,
      );
      tier = decision.tier;
      const emergency = localDown && tier === "cloud";
      const peerDemoted = decision.peerDemoted === true;
      if (tier !== state.tier) {
        if (tier === "local") {
          // Returning home resets the cap (oscillation debt is settled).
          state = { tier, switches: 0, turnsOnTier: 1, failoverActive: false };
        } else {
          state = {
            tier,
            // Peer demotions are arbitration, not cache-affinity churn.
            switches: state.switches + (emergency || state.failoverActive || peerDemoted ? 0 : 1),
            turnsOnTier: 1,
            failoverActive: emergency ? true : state.failoverActive,
          };
        }
      } else {
        state = { ...state, turnsOnTier: state.turnsOnTier + 1 };
        // Slow cap recovery on an idle local tier.
        if (
          tier === "local" &&
          state.switches > 0 &&
          localIdleSince != null &&
          now - localIdleSince >= cfg.returnIdleMs
        ) {
          state = { ...state, switches: 0 };
        }
      }
    }

    if (tier === "cloud" && candidate) {
      activeCloud = candidate;
      remember(candidate, "cloud");
      setRouteWidget(ctx, "cloud", candidate, profile);
      return {
        model: candidate,
        thinkingLevel: request.thinkingLevel,
        state: { ...state, cloudRetries: (state.cloudRetries ?? 0) + 1 },
      };
    }
    if (local) {
      if (state.tier !== "local") {
        ctx.ui?.notify?.(
          "route: local (Lapis Occultus) — switching back re-prefills the prompt",
          "info",
        );
      }
      activeCloud = undefined;
      remember(local, "local");
      setRouteWidget(ctx, "local", undefined, profile);
      return { model: local, thinkingLevel: request.thinkingLevel, state: { ...state, tier: "local", failoverActive: false } };
    }
    if (candidate) {
      activeCloud = candidate;
      remember(candidate, "cloud");
      setRouteWidget(ctx, "cloud", candidate, profile);
      return {
        model: candidate,
        thinkingLevel: request.thinkingLevel,
        state: {
          ...state,
          tier: "cloud",
          cloudRetries: (state.cloudRetries ?? 0) + 1,
        },
      };
    }
    // Honest failure (2026-11-14): name the gate that actually refused.
    const budget = budgetStatus();
    const cloudWhy = !profileAllowsCloud(profile)
      ? `profile '${profile}' is local-only`
      : !canSpendCloud()
        ? `cloud budget exhausted (billed daily ${budget.dailyTokens}/${budget.dailyCap}, monthly ${budget.monthlyTokens}/${budget.monthlyCap})`
        : (() => {
            const coolingMs = cloudCandidates
              .map((m) => cooldownRemaining(m.provider, m.id, now))
              .filter((ms) => ms > 0);
            return coolingMs.length > 0
              ? `every candidate lane is cooling (earliest in ${Math.ceil(Math.min(...coolingMs) / 60_000)}m)`
              : "no free cloud lane fits this context";
          })();
    const registered = ctx.modelRegistry
      .getAll()
      .filter((m) => m.provider === "llamacpp")
      .map((m) => m.id);
    const localWhy =
      registered.length > 0
        ? `local model not registered in this session (engine alias not probed; registered: ${registered.slice(0, 3).join(", ")})`
        : "no local model configured (models.json has no llamacpp entries)";
    throw new Error(
      `Lapis Occultus: no physical model available — ${local ? "local ready" : localWhy}; engine ${snap.up ? "up" : "down"}; ${cloudWhy}`,
    );
  };

  pi.on("after_provider_response", (event) => {
    if (!activeCloud) return;
    const status = event.status;
    const retryAfterMs = recordRetryAfter(activeCloud.provider, activeCloud.id, event.headers ?? {});
    if (status && status >= 400) {
      recordFailure(activeCloud.provider, activeCloud.id, status, { retryAfterMs });
    }
  });

  pi.on("message_end", (event, ctx) => {
    if (!lastDecision) return;
    const decision = lastDecision;
    const msg = event.message as {
      role?: string;
      provider?: string;
      model?: string;
      stopReason?: string;
      usage?: {
        input?: number;
        output?: number;
        cacheRead?: number;
        cacheWrite?: number;
        totalTokens?: number;
      };
    };
    if (msg.role !== "assistant") return;
    const failed = msg.stopReason === "error";
    if (!failed) recordSuccess(decision.provider, decision.model);
    const usage = msg.usage ?? {};
    const entry: RouteLedgerEntry = {
      ts: new Date().toISOString(),
      reason: decision.reason,
      tier: decision.tier,
      provider: decision.provider,
      model: decision.model,
      profile: decision.profile,
      localState: decision.localState,
      latencyMs: Date.now() - decision.startedAt,
      tokensIn: usage.input ?? 0,
      tokensOut: usage.output ?? 0,
      cacheRead: usage.cacheRead ?? 0,
      cacheWrite: usage.cacheWrite ?? 0,
      costEstimate: 0,
      outcome: failed ? "error" : "ok",
      errorClass: failed ? "completion_error" : undefined,
    };
    recordRoute(entry);
    lastDecision = null;
  });

  pi.registerCommand("model-profile", {
    description: "Routing profile: auto | fast | deep | local",
    handler: async (args: string, ctx: { ui: any }) => {
      const raw = args.trim().toLowerCase();
      const next = raw as RoutingProfile;
      if (!["auto", "fast", "deep", "local"].includes(next)) {
        await ctx.ui.notify?.("profile: auto | fast | deep | local", "info");
        return;
      }
      writeProfile(next);
      catalogLoaded = false;
      cloudCandidates = [];
      await ctx.ui.notify?.(`profile: ${next}`, "info");
    },
  });

  pi.registerCommand("route-why", {
    description: "Explain the last routing decision",
    handler: async (_args: string, ctx: { ui: any }) => {
      const last = lastRouteEntry();
      if (!last) {
        await ctx.ui.notify?.("why: no routing entries yet", "info");
        return;
      }
      await ctx.ui.notify?.(
        `why: ${last.tier} via ${last.provider}/${last.model} · ${last.reason} · profile ${last.profile} · ${last.outcome}`,
        "info",
      );
    },
  });

  pi.registerCommand("budget", {
    description: "Free token usage for today and this month",
    handler: async (_args: string, ctx: { ui: any }) => {
      const t = todaySummary();
      await ctx.ui.notify?.(
        `cost: today ${t.tokensIn + t.tokensOut} tok · ${t.requests} req · ${t.failures} fail · ${budgetSummaryText()}`,
        "info",
      );
    },
  });

  pi.registerCommand("free", {
    description: "List free overflow candidates and cooldowns",
    handler: async (_args: string, ctx: ExtensionContext) => {
      await loadCandidates(ctx);
      const now = Date.now();
      const rows = cloudCandidates.slice(0, 20).map((m, i) => {
        const cool = cooldownRemaining(m.provider, m.id, now);
        return `${i + 1}. ${m.id} · ${Math.round((m.contextWindow ?? 0) / 1000)}k${cool > 0 ? ` · cooling ${Math.ceil(cool / 60_000)}m` : ""}`;
      });
      await (ctx.ui as any).notify?.(
        rows.length ? `free candidates (${cloudCandidates.length}):\n${rows.join("\n")}` : "free: no candidates",
        "info",
      );
    },
  });

  pi.registerCommand("nimbus", {
    description: "Free cloud lanes: gateway health, cooldowns, next pick",
    handler: async (_args: string, ctx: ExtensionContext) => {
      await loadCandidates(ctx);
      const health = await getLaneHealth();
      const now = Date.now();
      const tokens = ctx.getContextUsage()?.tokens ?? 0;
      const rows: LaneRow[] = cloudCandidates.map((m) => ({
        id: m.id,
        owner: (m.id.split("/")[0] ?? "").toLowerCase(),
        contextWindow: m.contextWindow ?? 0,
        coolingMs: Math.max(0, cooldownRemaining(m.provider, m.id, now)),
      }));
      const next = pickCloud(tokens, now);
      const table = formatLanes(rows, health, now);
      await (ctx.ui as any).notify?.(
        `nimbus · ctx ${Math.round(tokens / 1000)}k · ${rows.length} lane(s) · next: ${next ? next.id : "none fits"}\n${table || "no lanes resolved"}`,
        "info",
      );
    },
  });

  pi.registerVirtualModel<LapisState>({
    provider: "officina",
    id: "lapis",
    name: "Lapis Occultus",
    contextWindow: 262_144,
    maxTokens: 4_096,
    thinkingLevels: ["off", "low", "medium", "high"],
    route,
  });

  // Nimbus — the CLOUD-ONLY lane (owner request 2026-11-11): select
  // officina/nimbus in /model and every turn routes to the best free
  // cloud lane; the local engine is never selected, in any state. Same
  // verified-free candidate pool as lapis's cloud tier (discovery +
  // free fallbacks), same health/cooling rotation, same continuation
  // stickiness. It dies loudly when no free lane fits instead of
  // silently falling back to local.
  const routeNimbus = async (
    request: ModelRouteRequest<NimbusState>,
    ctx: ExtensionContext,
  ): Promise<ModelRoute<NimbusState>> => {
    await loadCandidates(ctx);
    const now = Date.now();
    const tokens = ctx.getContextUsage()?.tokens ?? 0;
    const previous = request.previous?.model;
    // Mid tool-loop: never swap the physical model under the agent.
    if (request.reason === "continuation" && previous && previous.id !== "nimbus") {
      return { model: previous, thinkingLevel: request.thinkingLevel, state: request.state };
    }
    // Health-aware pick first (cooling / recently-failed lanes skipped),
    // then a last-resort fit-only pick so the lane dies only when NO free
    // candidate fits the context at all.
    let candidate = pickCloud(tokens, now);
    if (!candidate) {
      candidate = cloudCandidates.find(
        (m) => tokens + CLOUD_RESERVE_TOKENS < (m.contextWindow ?? 0),
      );
    }
    if (!candidate) {
      throw new Error(
        "Nimbus: no free cloud lane fits this context (candidates cooling or too small) — switch to officina/lapis for the local engine",
      );
    }
    activeCloud = candidate;
    lastDecision = {
      reason: request.reason,
      tier: "cloud",
      provider: candidate.provider,
      model: candidate.id,
      profile: readProfile(),
      localState: getEngineSnapshot().up ? "up" : "down",
      startedAt: now,
    };
    publishPeerDecision("cloud", candidate, tokens, ctx, now);
    setRouteWidget(ctx, "cloud", candidate, readProfile(), "nimbus");
    return {
      model: candidate,
      thinkingLevel: request.thinkingLevel,
      state: { turnsOnTier: (request.state?.turnsOnTier ?? 0) + 1 },
    };
  };

  pi.registerVirtualModel<NimbusState>({
    provider: "officina",
    id: "nimbus",
    name: "Nimbus — free cloud lanes only",
    contextWindow: 1_000_000,
    maxTokens: 16_384,
    thinkingLevels: ["off", "low", "medium", "high"],
    route: routeNimbus,
  });
}
