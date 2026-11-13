// ask — vendored 2026-08-31 from @earendil-works/pi-coding-agent 0.83.0
// examples/extensions/question.ts (Apache-2.0).
//
// [officina P-ask1] 2026-11-13: the vendored version used ctx.ui.custom (a
// JS-drawn component) and bailed when ctx.mode !== "tui" — the Rust TUI
// runs pi in RPC mode, so the question never popped up there, and custom
// components have no Rust renderer. Rewritten on the standard dialog API
// (ui.select → ui.input for the free-text escape) which the Rust TUI
// renders as a real modal in every agent mode; only non-UI (print) runs
// get the honest error. Behaviour otherwise unchanged: options + "Type
// something." escape, answer returned as the tool result.
// NOTICE: Apache-2.0 attribution chain in .pi/extensions/NOTICE.

/**
 * Question Tool - Single question with options
 * Full custom UI: options list + inline editor for "Type something..."
 * Escape in editor returns to options, Escape in options cancels
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";

interface OptionWithDesc {
	label: string;
	description?: string;
}

interface QuestionDetails {
	question: string;
	options: string[];
	answer: string | null;
	wasCustom?: boolean;
}

// Options with labels and optional descriptions
const OptionSchema = Type.Object({
	label: Type.String({ description: "Display label for the option" }),
	description: Type.Optional(Type.String({ description: "Optional description shown below label" })),
});

const QuestionParams = Type.Object({
	question: Type.String({ description: "The question to ask the user" }),
	options: Type.Array(OptionSchema, { description: "Options for the user to choose from" }),
});

export default function question(pi: ExtensionAPI) {
	pi.registerTool({
		name: "question",
		label: "Question",
		description: "Ask the user a question and let them pick from options. Use when you need user input to proceed.",
		parameters: QuestionParams,
		executionMode: "sequential",

		async execute(_toolCallId, params, _signal, _onUpdate, ctx) {
			const base = {
				question: params.question,
				options: params.options.map((o: { label: string }) => o.label),
				answer: null as string | null,
			};
			if (ctx.hasUI === false) {
				return {
					content: [{ type: "text", text: "Error: UI not available (running in non-interactive mode)" }],
					details: base as QuestionDetails,
				};
			}
			if (params.options.length === 0) {
				return {
					content: [{ type: "text", text: "Error: No options provided" }],
					details: base as QuestionDetails,
				};
			}

			const OTHER = "Type something.";
			const labels = [...params.options.map((o: { label: string }) => o.label), OTHER];
			const picked = await ctx.ui.select(params.question, labels);
			if (picked === null || picked === undefined) {
				return {
					content: [{ type: "text", text: "User cancelled the question." }],
					details: base as QuestionDetails,
				};
			}
			let answer = String(picked);
			let wasCustom = false;
			if (picked === OTHER) {
				const typed = await ctx.ui.input(params.question, "type your answer");
				if (typed === null || typed === undefined || !String(typed).trim()) {
					return {
						content: [{ type: "text", text: "User cancelled the question." }],
						details: base as QuestionDetails,
					};
				}
				answer = String(typed).trim();
				wasCustom = true;
			}
			return {
				content: [{ type: "text", text: `User answered: ${answer}` }],
				details: { ...base, answer, wasCustom } as QuestionDetails,
			};
		},
	});
}
