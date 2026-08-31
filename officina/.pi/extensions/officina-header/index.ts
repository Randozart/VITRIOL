import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

// officina watermark (2026-08-31, owner request): the VITRIOL braille logo,
// watermark-style, while the session is untouched. pi renders the header
// above the chat at startup; as soon as the conversation grows it scrolls
// away naturally — the logo greets you on an empty workshop and dissolves
// once the work begins. No state, no input handling, nothing to break.
//
// Style: single faint blue-slate tint ("vaguely brighter than the
// background"), per First-Party Mandate visual language (Vitriolum BG
// #0d1117 → logo #1c2634). Kill switch: OFFICINA_WATERMARK=0.

const here = dirname(fileURLToPath(import.meta.url));
const LOGO_PATHS = [
  join(here, "..", "..", "..", "..", "assets", "braille-logo-80c.txt"), // VITRIOL repo layout
  join(here, "..", "..", "..", "assets", "braille-logo-80c.txt"),
];

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_WATERMARK === "0") return; // Rule 15

  pi.on("session_start", (_event, ctx) => {
    let logo: string | null = null;
    for (const p of LOGO_PATHS) {
      try {
        logo = readFileSync(p, "utf-8").replace(/\n+$/, "");
        break;
      } catch {
        continue;
      }
    }
    if (!logo) return; // no asset, no watermark — silently fine

    // #1c2634: barely-there blue lift off the #0d1117 background.
    const tint = (line: string) => `[38;2;28;38;52m${line}[0m`;
    const lines = logo.split("\n").map(tint);

    ctx.ui.setHeader?.(() => ({
      render(width: number): string[] {
        void width; // logo is fixed-width art; centering happens below
        return lines;
      },
      invalidate() {},
    }));
  });
}
