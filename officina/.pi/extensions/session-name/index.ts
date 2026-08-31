// session-name — vendored 2026-08-31 from @earendil-works/pi-coding-agent 0.83.0
// examples/extensions/session-name.ts (MIT), verbatim except this header.
// /session-name [name] — named sessions render in the picker with a distinct
// color (unnamed ones fall back to first-message preview). Direct response to
// the "can't find my session" incident (unnamed rows are easy to overlook).
// Kill switch: OFFICINA_NO_SESSION_NAME=1.

/**
 * Session naming example.
 *
 * Shows setSessionName/getSessionName to give sessions friendly names
 * that appear in the session selector instead of the first message.
 *
 * Usage: /session-name [name] - set or show session name
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI) {
	if (process.env.OFFICINA_NO_SESSION_NAME === "1") return; // Rule 15

	pi.registerCommand("session-name", {
		description: "Set or show session name (usage: /session-name [new name])",
		handler: async (args, ctx) => {
			const name = args.trim();

			if (name) {
				pi.setSessionName(name);
				ctx.ui.notify(`Session named: ${name}`, "info");
			} else {
				const current = pi.getSessionName();
				ctx.ui.notify(current ? `Session: ${current}` : "No session name set", "info");
			}
		},
	});
}
