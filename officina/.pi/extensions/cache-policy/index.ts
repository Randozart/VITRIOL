// cache-policy — publishes whether the active provider is cloud/local so other
// extensions can pick cache-safe behavior. State lives in `_shared/cache-mode.ts`
// (globalThis-backed because jiti isolates extension modules).

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { setCacheModeFromProvider } from "../_shared/cache-mode.ts";

function providerOf(event: unknown, ctx: { model?: { provider?: string }; getModel?: () => { provider?: string } | undefined }): string | undefined {
  const direct = (event as { model?: { provider?: string } } | undefined)?.model?.provider;
  return direct ?? ctx?.model?.provider ?? ctx?.getModel?.()?.provider;
}

export default function (pi: ExtensionAPI) {
  pi.on("session_start", (_event, ctx) => {
    setCacheModeFromProvider(providerOf(undefined, ctx as never));
  });

  pi.on("model_select", (event, ctx) => {
    setCacheModeFromProvider(providerOf(event, ctx as never));
  });
}
