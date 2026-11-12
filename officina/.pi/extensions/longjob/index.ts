// longjob (2026-11-12) — agents can't hang on bash by default. Known
// long-runners are dispatched to a detached process; the session is woken
// with the result (triggerTurn when idle) plus a desktop notification.
// I1 fail-open · I2 never-kill · I5 single-announce. Kill switches:
//   OFFICINA_NO_LONGJOB=1  removes everything
//   OFFICINA_NO_AUTOBG=1   keeps the tools, stops the guard rewrite

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { Type } from "@sinclair/typebox";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import {
  isLongRunner,
  jobDir,
  jobsToAnnounce,
  rewriteToDispatch,
  shellQuote,
  snapshotJob,
} from "./longjob.ts";

const JOBS_ROOT = join(homedir(), ".local/share", "officina", "longjobs");

function notifyDesktop(body: string): void {
  try {
    spawnSync("notify-send", ["-a", "officina", "longjob", body], { timeout: 5_000 });
  } catch {
    // cosmetic
  }
}

function announceText(root: string, id: string): string | null {
  const snap = snapshotJob(root, id);
  if (!snap) return null;
  const tail = snap.tail.trim() === "" ? "(no output)" : snap.tail.trim().split("\n").slice(-8).join("\n");
  return `longjob ${id} finished · exit ${snap.exit}\n${tail}`;
}

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_NO_LONGJOB === "1") return;
  mkdirSync(JOBS_ROOT, { recursive: true });
  let ui: any = null;
  let idleTick: ReturnType<typeof setInterval> | null = null;

  pi.on("session_start", (_event, ctx) => {
    ui = (ctx as { hasUI?: boolean; ui?: unknown }).hasUI ? (ctx as { ui: any }).ui : null;
  });

  // I5: announce finished jobs exactly once. triggerTurn wakes the idle
  // agent with the result; a busy run only gets the desktop ping now and
  // the message on the next idle tick.
  idleTick = setInterval(() => {
    for (const id of jobsToAnnounce(JOBS_ROOT)) {
      const text = announceText(JOBS_ROOT, id) ?? `longjob ${id} finished`;
      notifyDesktop(`longjob ${id} · exit ${snapshotJob(JOBS_ROOT, id)?.exit ?? "?"}`);
      try {
        writeFileSync(join(JOBS_ROOT, id, "announced"), "1");
      } catch {
        // worst case: re-announce (still never kills)
      }
      try {
        // deliverAs followUp + triggerTurn (pi docs): when idle this starts
        // a turn with the result; during a run it waits for the agent to
        // finish and delivers before the next prompt. The desktop notify
        // already fired either way.
        pi.sendMessage?.(
          {
            customType: "longjob",
            content: `${text}\n(continue the work, or acknowledge and move on)`,
            display: true,
          },
          { triggerTurn: true, deliverAs: "followUp" },
        );
      } catch {
        // injection is best-effort; the desktop notify already fired
      }
    }
  }, 5_000);
  idleTick.unref?.();

  pi.registerTool({
    name: "bash_bg",
    label: "Bash Background",
    description:
      "Dispatch a shell command as a DETACHED background job and return immediately. " +
      "Returns the job id + log path. Poll with bg_status, wait bounded with bg_wait, " +
      "or end your turn — you are woken with the result. Use for builds/tests/compiles " +
      "(cargo, make, brievc, npm, pytest, …). Append `#fg` to a bash command to force " +
      "foreground instead.",
    parameters: Type.Object({
      command: Type.String({ description: "The shell command to run" }),
      max_minutes: Type.Number({ description: "Optional kill-at-expiry (explicit intent)" }),
    }),
    async execute(_id, input) {
      const command = String((input as { command?: string }).command ?? "").trim();
      const maxMinutes = Number((input as { max_minutes?: number }).max_minutes ?? 0);
      if (!command) {
        return errText("bash_bg: 'command' is required");
      }
      const job = `j-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
      const paths = jobDir(JOBS_ROOT, job);
      try {
        mkdirSync(paths.dir, { recursive: true, mode: 0o700 });
        const dispatch = rewriteToDispatch(command, paths, job);
        const killLine =
          maxMinutes > 0
            ? ` (sleep ${Math.trunc(maxMinutes * 60)}; kill -- -$(cat '${paths.pid}') 2>/dev/null; echo exit=124 > '${paths.exit}') &`
            : "";
        const res = spawnSync("bash", ["-c", `${dispatch}${killLine}`], {
          encoding: "utf8",
          timeout: 10_000,
        });
        if (res.error || res.status !== 0) {
          // I1 fail-open: caller should fall back to foreground bash.
          return errText(`bash_bg: dispatch failed (${String(res.stderr ?? res.error ?? "").trim()}) — run it in the foreground instead`);
        }
        return okText(
          String(res.stdout ?? "").trim() ||
            `longjob ${job} dispatched`,
        );
      } catch (error) {
        return errText(
          `bash_bg: dispatch failed (${error instanceof Error ? error.message : String(error)}) — run it in the foreground instead`,
        );
      }
    },
  });

  pi.registerTool({
    name: "bg_status",
    label: "Longjob Status",
    description: "Snapshot of a background job: running? exit code? last 40 output lines.",
    parameters: Type.Object({ id: Type.String({ description: "Job id (j-…)" }) }),
    async execute(_id, input) {
      const job = String((input as { id?: string }).id ?? "").trim();
      const snap = job ? snapshotJob(JOBS_ROOT, job) : null;
      if (!job || !snap) return errText(`bg_status: unknown job '${job}'`);
      const tail = snap.tail.trim() || "(no output yet)";
      return okText(
        snap.running
          ? `longjob ${job}: RUNNING\ntail:\n${tail}`
          : `longjob ${job}: EXIT ${snap.exit}\ntail:\n${tail}`,
      );
    },
  });

  pi.registerTool({
    name: "bg_wait",
    label: "Longjob Wait",
    description: "Bounded wait for a background job (never hangs forever). Returns as soon as it exits or max_s elapses.",
    parameters: Type.Object({
      id: Type.String(),
      max_s: Type.Number({ description: "1-1200 seconds (default 120)" }),
    }),
    async execute(_id, input) {
      const job = String((input as { id?: string }).id ?? "").trim();
      const maxS = Math.max(1, Math.min(1200, Number((input as { max_s?: number }).max_s ?? 120)));
      const snap = job ? snapshotJob(JOBS_ROOT, job) : null;
      if (!job || !snap) return errText(`bg_wait: unknown job '${job}'`);
      const deadline = Date.now() + maxS * 1000;
      let cur = snap;
      while (cur.running && Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, 2_000));
        cur = snapshotJob(JOBS_ROOT, job) ?? cur;
      }
      const tail = cur.tail.trim() || "(no output yet)";
      return okText(
        cur.running
          ? `longjob ${job}: still RUNNING after ${maxS}s (use bg_wait again or end your turn — completion is announced)\ntail:\n${tail}`
          : `longjob ${job}: EXIT ${cur.exit}\ntail:\n${tail}`,
      );
    },
  });

  pi.registerTool({
    name: "bg_kill",
    label: "Longjob Kill",
    description: "Explicitly terminate a background job (the only terminator — the watcher never kills).",
    parameters: Type.Object({ id: Type.String() }),
    async execute(_id, input) {
      const job = String((input as { id?: string }).id ?? "").trim();
      const paths = job ? jobDir(JOBS_ROOT, job) : null;
      if (!job || !paths || !existsSync(paths.dir)) return errText(`bg_kill: unknown job '${job}'`);
      try {
        const pid = readFileSync(paths.pid, "utf8").trim();
        const r = spawnSync("kill", ["--", `-${pid}`], { timeout: 5_000 });
        writeFileSync(paths.exit, `exit=killed(${r.status ?? "?"})`);
        return okText(`longjob ${job}: killed (process group ${pid})`);
      } catch (error) {
        return errText(`bg_kill failed: ${error instanceof Error ? error.message : String(error)}`);
      }
    },
  });

  // Stage 2 note: the permissions-guard imports rewriteToDispatch +
  // isLongRunner from ./longjob.ts for the auto-dispatch hook — single
  // source of truth for the matcher. shellQuote re-exported for it too.
  void shellQuote;
}

function errText(text: string) {
  return { content: [{ type: "text" as const, text }], details: {}, isError: true };
}

function okText(text: string) {
  return { content: [{ type: "text" as const, text }], details: {} };
}
