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
import { mkdirSync, renameSync, rmSync, writeFileSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { homedir } from "node:os";
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

// ── Facts-file flow (2026-11-13) ─────────────────────────────────────────
// The RPC dialog bridging hung live (tool-context select awaited a
// response that never crossed the wire — counsel session, 2026-11-13
// 21:22, two question calls, no results). In RPC mode the question is
// staged as ~/.vitriol/officina/question.json and the Rust TUI renders it
// as a panel ON TOP OF THE PROMPT, writing question-response.json. The
// extension polls with a hard timeout — fail-open, never hangs.
// [officina P-ask2]

const QUESTION_FILE = join(homedir(), ".vitriol", "officina", "question.json");
const RESPONSE_FILE = join(homedir(), ".vitriol", "officina", "question-response.json");
const QUESTION_TIMEOUT_MS = 10 * 60 * 1000;

interface StagedQuestion {
  version: number;
  id: string;
  question: string;
  options: string[];
  ts: number;
}

interface QuestionResponse {
  id: string;
  answer: string | null;
  cancelled: boolean;
}

function stageQuestion(doc: StagedQuestion): void {
  mkdirSync(dirname(QUESTION_FILE), { recursive: true });
  // a stale response from a previous question must never answer this one
  try {
    rmSync(RESPONSE_FILE);
  } catch { /* absent is fine */ }
  const tmp = `${QUESTION_FILE}.tmp-${process.pid}`;
  writeFileSync(tmp, `${JSON.stringify(doc, null, 2)}\n`);
  renameSync(tmp, QUESTION_FILE);
}

function readResponse(id: string): QuestionResponse | null {
  try {
    const doc = JSON.parse(readFileSync(RESPONSE_FILE, "utf8")) as QuestionResponse;
    if (doc.id !== id) return null;
    return doc;
  } catch {
    return null;
  }
}

function cleanupQuestionFiles(): void {
  try {
    rmSync(QUESTION_FILE);
  } catch { /* absent is fine */ }
  try {
    rmSync(RESPONSE_FILE);
  } catch { /* absent is fine */ }
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** RPC-mode question via the facts files. Returns null on timeout. */
async function askViaFacts(
  question: string,
  labels: string[],
): Promise<{ answer: string; cancelled: boolean } | null> {
  const id = `q-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  stageQuestion({
    version: 1,
    id,
    question,
    options: labels,
    ts: Date.now(),
  });
  const deadline = Date.now() + QUESTION_TIMEOUT_MS;
  while (Date.now() < deadline) {
    await sleep(400);
    const resp = readResponse(id);
    if (resp) {
      cleanupQuestionFiles();
      if (resp.cancelled || resp.answer === null) return null;
      return { answer: resp.answer, cancelled: false };
    }
  }
  cleanupQuestionFiles();
  return null;
}

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
			const optionLabels = params.options.map((o: { label: string }) => o.label);

			// RPC mode (the Rust TUI): stage via the facts files — the
			// tool-context dialog bridging hung live (P-ask2). The TUI
			// renders the question on top of the prompt and writes the
			// response; 10 min timeout, fail-open. NOTE: the free-text
			// "Type something." escape is TUI-mode only — facts files
			// carry the real options only.
			if (ctx.mode === "rpc") {
				const factResult = await askViaFacts(params.question, optionLabels);
				if (!factResult) {
					return {
						content: [{ type: "text", text: "No answer within 10 minutes — re-ask if still needed." }],
						details: { ...base, answer: null } as QuestionDetails,
					};
				}
				return {
					content: [{ type: "text", text: `User answered: ${factResult.answer}` }],
					details: { ...base, answer: factResult.answer, wasCustom: factResult.cancelled } as QuestionDetails,
				};
			}

			const labels = [...optionLabels, OTHER];
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
