// notify-done — vendored 2026-08-31 from @earendil-works/pi-coding-agent 0.83.0
// examples/extensions/notify.ts (MIT). Modifications: title "Pi" -> "officina";
// the Windows/powershell branch is dead code on this host but retained verbatim.
// Fires a native terminal notification (OSC 777/99) on agent_end — for 27B
// local decode the user regularly waits minutes; the ping is pure QoL.
// Kill switch: OFFICINA_NO_NOTIFY=1.

/**
 * Pi Notify Extension
 *
 * Sends a native terminal notification when Pi agent is done and waiting for input.
 * Supports multiple terminal protocols:
 * - OSC 777: Ghostty, iTerm2, WezTerm, rxvt-unicode
 * - OSC 99: Kitty
 * - Windows toast: Windows Terminal (WSL)
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { execFile } from "node:child_process";

function windowsToastScript(title: string, body: string): string {
	const type = "Windows.UI.Notifications";
	const mgr = `[${type}.ToastNotificationManager, ${type}, ContentType = WindowsRuntime]`;
	const template = `[${type}.ToastTemplateType]::ToastText01`;
	const toast = `[${type}.ToastNotification]::new($xml)`;
	return [
		`${mgr} > $null`,
		`$xml = [${type}.ToastNotificationManager]::GetTemplateContent(${template})`,
		`$xml.GetElementsByTagName('text')[0].AppendChild($xml.CreateTextNode('${body}')) > $null`,
		`[${type}.ToastNotificationManager]::CreateToastNotifier('${title}').Show(${toast})`,
	].join("; ");
}

function notifyOSC777(title: string, body: string): void {
	process.stdout.write(`\x1b]777;notify;${title};${body}\x07`);
}

function notifyOSC99(title: string, body: string): void {
	// Kitty OSC 99: i=notification id, d=0 means not done yet, p=body for second part
	process.stdout.write(`\x1b]99;i=1:d=0;${title}\x1b\\`);
	process.stdout.write(`\x1b]99;i=1:p=body;${body}\x1b\\`);
}

function notifyWindows(title: string, body: string): void {
		execFile("powershell.exe", ["-NoProfile", "-Command", windowsToastScript(title, body)]);
}

function notify(title: string, body: string): void {
	if (process.env.WT_SESSION) {
		notifyWindows(title, body);
	} else if (process.env.KITTY_WINDOW_ID) {
		notifyOSC99(title, body);
	} else {
		notifyOSC777(title, body);
	}
}

// Codex-infra stage 2 (2026-11-12): when the TUI is not focused, the OSC
// ping lands in an unseen terminal. A desktop notification (notify-send)
// covers that; threshold-gated so quick turns stay silent. Env:
//   OFFICINA_NO_DESKTOP_NOTIFY=1  kill switch
//   OFFICINA_DESKTOP_NOTIFY_MIN_S threshold (default 20)

/** Track the run start so agent_end can compute the turn duration. */
let runStartedAt = 0;

export function desktopNotifyEligible(
	env: NodeJS.ProcessEnv,
	durationS: number,
): boolean {
	if (env.OFFICINA_NO_DESKTOP_NOTIFY === "1") return false;
	if (!env.WAYLAND_DISPLAY && !env.DISPLAY) return false;
	const min = Number(env.OFFICINA_DESKTOP_NOTIFY_MIN_S ?? "20");
	return durationS >= (Number.isFinite(min) ? min : 20);
}

function notifyDesktop(body: string): void {
	try {
		execFile("notify-send", ["-a", "officina", "officina", body], { timeout: 5_000 });
	} catch {
		// cosmetic
	}
}

export default function (pi: ExtensionAPI) {
	if (process.env.OFFICINA_NO_NOTIFY === "1") return; // Rule 15

	pi.on("before_agent_start", () => {
		runStartedAt = Date.now();
	});

	pi.on("agent_end", async () => {
		notify("officina", "Ready for input");
		const durationS = runStartedAt > 0 ? (Date.now() - runStartedAt) / 1000 : 0;
		runStartedAt = 0;
		if (desktopNotifyEligible(process.env, durationS)) {
			notifyDesktop(
				durationS >= 90
					? `turn done (${Math.round(durationS / 60)} min)`
					: `turn done (${Math.round(durationS)} s)`,
			);
		}
	});
}
