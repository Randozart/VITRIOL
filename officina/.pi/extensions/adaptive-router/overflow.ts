// Cache-aware overflow policy — pure. No I/O, no pi runtime.
//
// Local-first with cloud spillover. The local engine has one slot, so
// "contended" means processing/deferred/queue wait. Switching tiers is
// expensive (local KV prefill + provider prompt cache), so the policy uses
// hysteresis: minimum dwell, max switches, and a bounded cloud-token budget
// before returning to local.

import { decidePeerYield, type PeerDecision, type PeerView, type SelfView } from "./peer-policy.ts";

export type Tier = "local" | "cloud";
export type Privacy = "safe" | "sensitive" | "confidential";

export interface OverflowConfig {
  queueWaitMs: number;
  gpuLoadCutoff: number;
  minDwell: number;
  maxSwitches: number;
  returnIdleMs: number;
  returnTokenBudget: number;
  cloudTokenBudget: number;
  sensitiveLocal: boolean;
  /** Emergency cloud when the local engine is actually DOWN (not just busy). */
  failover: boolean;
}

export function configFromEnv(env: NodeJS.ProcessEnv = process.env): OverflowConfig {
  const num = (key: string, dflt: number): number => {
    const n = Number(env[key]);
    return Number.isFinite(n) && n >= 0 ? n : dflt;
  };
  return {
    queueWaitMs: num("OFFICINA_OVERFLOW_QUEUE_MS", 1500),
    gpuLoadCutoff: num("OFFICINA_OVERFLOW_GPU_LOAD", 0.85),
    minDwell: Math.max(1, Math.floor(num("OFFICINA_OVERFLOW_MIN_DWELL", 3))),
    maxSwitches: Math.max(0, Math.floor(num("OFFICINA_OVERFLOW_MAX_SWITCHES", 2))),
    returnIdleMs: num("OFFICINA_OVERFLOW_RETURN_IDLE_MS", 60_000),
    returnTokenBudget: num("OFFICINA_OVERFLOW_RETURN_TOKENS", 16_000),
    cloudTokenBudget: num("OFFICINA_OVERFLOW_CLOUD_BUDGET", 2_000_000),
    sensitiveLocal: env.OFFICINA_OVERFLOW_SENSITIVE !== "0",
    failover: env.OFFICINA_OVERFLOW_FAILOVER !== "0",
  };
}

export interface LoadSample {
  processing: number;
  deferred: number;
  queueWaitMs: number;
  stalled: boolean;
  gpuLoad: number | null;
}

export interface RouterState {
  tier: Tier;
  switches: number;
  turnsOnTier: number;
  cloudTokens: number;
}

export interface DecideInput {
  state: RouterState;
  load: LoadSample;
  privacy: Privacy;
  cloudFits: boolean;
  budgetLeft: boolean;
  /** Milliseconds since the local engine was last seen busy; null if busy now. */
  localIdleForMs: number | null;
  /** Local engine is unreachable (after at least one poll). */
  localDown: boolean;
  /** Cloud tier was entered because the engine was down (not contention). */
  failoverActive: boolean;
  // ── P3 multi-session slot arbitration (all optional: absent = pre-P3
  //    behaviour exactly, which keeps single-session decisions identical) ──
  /** This session's identity for seniority ranking. */
  self?: PeerSelf;
  /** Peer sessions from the routing registry (expired ones already pruned). */
  peers?: PeerPeer[];
  /** Engine slots not currently processing. */
  freeSlots?: number;
}

export type PeerSelf = import("./peer-policy.ts").SelfView;
export type PeerPeer = import("./peer-policy.ts").PeerView;
export interface Decision {
  tier: Tier;
  reason: string;
  /** True when this decision was a peer-arbitration demotion (the caller
   *  must not charge it against the cache-affinity switch cap). */
  peerDemoted?: boolean;
}

export function localBusy(load: LoadSample, cfg: OverflowConfig): boolean {
  return (
    load.processing > 0 ||
    load.deferred > 0 ||
    load.queueWaitMs >= cfg.queueWaitMs ||
    load.stalled ||
    (load.gpuLoad != null && load.gpuLoad >= cfg.gpuLoadCutoff)
  );
}

/**
 * Peer-arbitration view of the decision inputs, or null when the caller did
 * not supply peer context (single-session / pre-P3 behaviour).
 */
function peerDecision(input: DecideInput): PeerDecision | null {
  if (input.self === undefined || input.peers === undefined || input.freeSlots === undefined) {
    return null;
  }
  const self: SelfView = input.self;
  const peers: PeerView[] = input.peers;
  return decidePeerYield({
    self,
    peers,
    freeSlots: input.freeSlots,
    privacy: input.privacy,
  });
}

/**
 * One turn-boundary decision. Pure and total; caller applies the tier.
 * Note: `state.switches` counts prior switches; a switch now is allowed while
 * `switches < maxSwitches`.
 */
export function decideTier(
  input: DecideInput,
  cfg: OverflowConfig = configFromEnv({}),
): Decision {
  const { state, load, privacy } = input;
  const busy = localBusy(load, cfg);

  // Privacy beats utilization.
  if (cfg.sensitiveLocal && privacy !== "safe") {
    return state.tier === "local"
      ? { tier: "local", reason: `${privacy} — local only` }
      : { tier: "local", reason: `${privacy} — forced back local` };
  }

  // Emergency failover: local DOWN is different from local BUSY. Ignore the
  // switch cap and token budget here — availability wins. Sensitive turns
  // still stay local (privacy is not negotiable).
  if (cfg.failover && input.localDown) {
    if (!input.cloudFits) {
      return state.tier === "cloud"
        ? { tier: "cloud", reason: "local down — keep current cloud lane" }
        : { tier: "local", reason: "local down — no viable cloud candidate" };
    }
    return { tier: "cloud", reason: "local down — emergency cloud" };
  }

  // Engine recovered while in failover: return to local as soon as it is
  // idle, without waiting out the cache-affinity dwell (local is preferred).
  if (state.tier === "cloud" && input.failoverActive && !busy) {
    if (input.localIdleForMs != null && input.localIdleForMs >= 5_000) {
      return { tier: "local", reason: "engine recovered — back to local" };
    }
  }

  // P3 slot arbitration: when every engine slot is committed, the junior
  // session yields so the senior one keeps the stone. Absent inputs (peers
  // not supplied) preserve the pre-P3 behaviour exactly, and a peer
  // demotion never charges the cache-affinity switch cap.
  const peerYield = peerDecision(input);
  if (peerYield?.demote && input.cloudFits && input.budgetLeft) {
    return { tier: "cloud", reason: peerYield.reason, peerDemoted: true };
  }

  const canSwitch = state.switches < cfg.maxSwitches;

  if (state.tier === "local") {
    if (!busy) return { tier: "local", reason: "local idle" };
    if (input.cloudFits && input.budgetLeft && canSwitch) {
      return { tier: "cloud", reason: "local busy — spill to cloud" };
    }
    return { tier: "local", reason: busy ? "local busy — stay (cloud unavailable)" : "local" };
  }

  // Cloud tier.
  if (!input.cloudFits) return { tier: "local", reason: "cloud context too small" };
  if (!input.budgetLeft) return { tier: "local", reason: "cloud token budget exhausted" };
  if (!canSwitch) return { tier: "cloud", reason: "switch cap reached — stay cloud" };
  if (state.turnsOnTier < cfg.minDwell) {
    return { tier: "cloud", reason: `min dwell ${cfg.minDwell} — stay cloud` };
  }
  if (busy) return { tier: "cloud", reason: "local busy — stay cloud" };
  if (
    input.localIdleForMs != null &&
    input.localIdleForMs >= cfg.returnIdleMs &&
    state.cloudTokens <= cfg.returnTokenBudget
  ) {
    return { tier: "local", reason: "local idle + small cloud delta — return" };
  }
  return { tier: "cloud", reason: "stay cloud (cache affinity)" };
}

/** Parse `provider/model/id` into its registry pair. Provider is the first
 *  segment; the remainder is the exact model id (ids may contain slashes). */
export function parseModelSpec(spec: string): { provider: string; id: string } | null {
  const trimmed = spec.trim();
  const slash = trimmed.indexOf("/");
  if (slash <= 0 || slash >= trimmed.length - 1) return null;
  return { provider: trimmed.slice(0, slash), id: trimmed.slice(slash + 1) };
}
