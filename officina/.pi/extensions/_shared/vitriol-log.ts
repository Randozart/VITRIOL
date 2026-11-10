// Tail reader for the VITRIOL engine log (`/tmp/opencode/vitriol_gen.log`).
//
// The engine already prints exact prompt progress and decode rates:
//   task N | prompt processing, n_tokens = 2688, progress = 0.88, t = ... / 362 t/s
//   task N | n_gen = 284, tg = 28.30 t/s, tg_3s = 30.80 t/s
//   task N | draft acceptance = 0.8037 ..., mean len = 3.41
//   task N | prompt eval time / eval time / release
// This module tails the last chunk and returns the newest task snapshot so the
// TUI can show log truth without duplicating the engine's counters.

import { closeSync, existsSync, openSync, readSync, statSync } from "node:fs";

export interface VitriolLogSnapshot {
  task: number | null;
  phase: "idle" | "prefill" | "decode" | "done";
  promptTokens: number | null;
  progress: number | null;
  prefillTps: number | null;
  generated: number | null;
  tg: number | null;
  tg3s: number | null;
  evalTps: number | null;
  draftAcceptance: number | null;
  draftMeanLen: number | null;
  ageMs: number;
  updatedAt: number | null;
}

interface TaskState {
  task: number;
  phase: VitriolLogSnapshot["phase"];
  promptTokens?: number;
  progress?: number;
  prefillTps?: number;
  generated?: number;
  tg?: number;
  tg3s?: number;
  evalTps?: number;
  draftAcceptance?: number;
  draftMeanLen?: number;
  updatedAt?: number;
}

export function vitriolLogPath(): string {
  return process.env.VITRIOL_LOG_PATH || "/tmp/opencode/vitriol_gen.log";
}

function readTail(path: string, maxBytes = 96 * 1024): string {
  try {
    const stat = statSync(path);
    if (!stat.isFile() || stat.size === 0) return "";
    const start = Math.max(0, stat.size - maxBytes);
    const length = stat.size - start;
    const buf = Buffer.allocUnsafe(length);
    const fd = openSync(path, "r");
    try {
      readSync(fd, buf, 0, length, start);
    } finally {
      closeSync(fd);
    }
    return buf.toString("utf8");
  } catch {
    return "";
  }
}

const TASK = /task\s+(\d+)\s*\|/;
const PREFILL = /prompt processing,\s*n_tokens\s*=\s*(\d+),\s*progress\s*=\s*([\d.]+),\s*t\s*=\s*([\d.]+)\s*s\s*\/\s*([\d.]+)\s*tokens per second/;
const DECODE = /n_gen\s*=\s*(\d+),\s*tg\s*=\s*([\d.]+)\s*t\/s,\s*tg_3s\s*=\s*([\d.]+)\s*t\/s/;
const EVAL = /eval time\s*=\s*[\d.]+\s*ms\s*\/\s*\d+\s*tokens\s*\(\s*[\d.]+\s*ms per token,\s*([\d.]+)\s*tokens per second\)/;
const DRAFT = /draft acceptance\s*=\s*([\d.]+).*?mean len\s*=\s*([\d.]+)/;
const RELEASE = /stop processing|slot\s+release/;

/** Parse a log chunk. Pure, for tests. */
export function parseVitriolLog(text: string): VitriolLogSnapshot | null {
  const tasks = new Map<number, TaskState>();
  let latestTask = -1;
  for (const line of text.split("\n")) {
    const tm = TASK.exec(line);
    if (!tm) continue;
    const task = Number(tm[1]);
    if (!Number.isFinite(task)) continue;
    latestTask = Math.max(latestTask, task);
    const st = tasks.get(task) ?? { task, phase: "idle" };
    tasks.set(task, st);
    st.updatedAt = Date.now();
    let m = PREFILL.exec(line);
    if (m) {
      st.phase = "prefill";
      st.promptTokens = Number(m[1]);
      st.progress = Number(m[2]);
      st.prefillTps = Number(m[4]);
      continue;
    }
    m = DECODE.exec(line);
    if (m) {
      st.phase = "decode";
      st.generated = Number(m[1]);
      st.tg = Number(m[2]);
      st.tg3s = Number(m[3]);
      continue;
    }
    m = EVAL.exec(line);
    if (m) {
      st.evalTps = Number(m[1]);
      continue;
    }
    m = DRAFT.exec(line);
    if (m) {
      st.draftAcceptance = Number(m[1]);
      st.draftMeanLen = Number(m[2]);
      continue;
    }
    if (RELEASE.test(line)) st.phase = "done";
  }
  const st = latestTask >= 0 ? tasks.get(latestTask) : undefined;
  if (!st) return null;
  return {
    task: st.task,
    phase: st.phase,
    promptTokens: st.promptTokens ?? null,
    progress: st.progress ?? null,
    prefillTps: st.prefillTps ?? null,
    generated: st.generated ?? null,
    tg: st.tg ?? null,
    tg3s: st.tg3s ?? null,
    evalTps: st.evalTps ?? null,
    draftAcceptance: st.draftAcceptance ?? null,
    draftMeanLen: st.draftMeanLen ?? null,
    ageMs: 0,
    updatedAt: st.updatedAt ?? null,
  };
}

let cachedAt = 0;
let cached: VitriolLogSnapshot | null = null;

/** Latest task snapshot; re-reads at most every 400 ms. */
export function getVitriolLogSnapshot(): VitriolLogSnapshot | null {
  const now = Date.now();
  if (now - cachedAt < 400) return cached;
  cachedAt = now;
  const path = vitriolLogPath();
  if (!existsSync(path)) {
    cached = null;
    return null;
  }
  const text = readTail(path);
  const snap = parseVitriolLog(text);
  if (!snap) {
    cached = null;
    return null;
  }
  try {
    snap.ageMs = Math.max(0, Date.now() - statSync(path).mtimeMs);
  } catch {
    snap.ageMs = 0;
  }
  cached = snap;
  return snap;
}
