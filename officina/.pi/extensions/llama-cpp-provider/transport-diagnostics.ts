import { channel } from "node:diagnostics_channel";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";

interface LocalTransportDiagnostics {
  origins: Set<string>;
  subscribed: boolean;
}

/** Preserve the raw transport error code without logging headers, credentials or prompts. */
function recordTransportError(value: unknown, origins: Set<string>): void {
  try {
    const event = value as { request?: { origin?: unknown; path?: unknown }; error?: { code?: unknown } } | null;
    if (!event?.request) return;
    const origin = String(event.request.origin ?? "");
    const path = String(event.request.path ?? "").split("?")[0];
    if (!origins.has(origin) || !path.endsWith("/chat/completions")) return;
    const code = typeof event.error?.code === "string" ? event.error.code : "UNKNOWN_TRANSPORT_ERROR";
    emitHarnessEvent(harnessEvent("lc-local-http", "transport_error", {
      detail: `pid=${process.pid} ${code} origin=${origin} route=chat/completions`,
    }));
  } catch {
    // Diagnostics must never interrupt a request or expose raw exception data.
  }
}

/** One process-wide observer survives jiti isolation/reloads without duplicating subscriptions. */
export function observeLocalTransport(baseUrl: string | undefined): void {
  try {
    if (!baseUrl) return;
    const origin = new URL(baseUrl).origin;
    const global = globalThis as { __officinaLocalTransportDiagnostics?: LocalTransportDiagnostics };
    const state = global.__officinaLocalTransportDiagnostics ??= { origins: new Set(), subscribed: false };
    state.origins.add(origin);
    if (state.subscribed) return;
    channel("undici:request:error").subscribe((event) => recordTransportError(event, state.origins));
    state.subscribed = true;
  } catch {
    // Invalid endpoint configuration is reported by the provider itself.
  }
}
