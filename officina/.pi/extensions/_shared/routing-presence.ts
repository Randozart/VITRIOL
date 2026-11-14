import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { onEngineUpdate } from "./engine.ts";
import { heartbeatRegistryEntry, removeRegistryShard } from "./routing-registry.ts";

/** Keep active decisions visible on the existing telemetry heartbeat, with immutable session age. */
export function routingPresence(pi: ExtensionAPI, tuiPid: number): () => number {
  let sessionStartedAt = Date.now();
  let active = false;
  let lastHeartbeat = 0;
  const heartbeat = () => {
    const now = Date.now();
    if (!active || now - lastHeartbeat < 10_000) return;
    lastHeartbeat = now;
    try {
      heartbeatRegistryEntry(tuiPid, now);
    } catch {
      // An unavailable registry must not interrupt inference or telemetry.
    }
  };
  let unsubscribe: (() => void) | undefined = onEngineUpdate(heartbeat);

  pi.on("session_start", () => {
    sessionStartedAt = Date.now();
    active = false;
    lastHeartbeat = 0;
    removeRegistryShard(tuiPid);
    unsubscribe ??= onEngineUpdate(heartbeat);
  });
  pi.on("agent_start", () => { active = true; });
  pi.on("agent_settled", () => { active = false; });
  pi.on("session_shutdown", () => {
    active = false;
    removeRegistryShard(tuiPid);
    unsubscribe?.();
    unsubscribe = undefined;
  });
  return () => sessionStartedAt;
}
