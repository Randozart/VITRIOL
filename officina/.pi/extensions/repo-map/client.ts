import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Client for the repo-map shim (shim.py, JSONL over stdio).
//
// Why persistent: importing repo-map's FastMCP server costs ~4s; one process
// per session amortizes that, so tool calls land in ~10-300ms (2026-08-29,
// measured on CachyOS / Qwen host). The client is lazy: the shim only starts
// on the first repomap_* tool call, and respawns once if it dies.
//
// Kill switch (Golden Rule 15): TRIS_NO_REPO_MAP=1 disables the whole
// extension; the unified config mirrors it at injection.repo_map.enabled.

/** Extension config, resolved from env with CachyOS-era defaults. */
export interface RepoMapConfig {
  enabled: boolean;
  pythonBin: string;
  repomapDir: string;
  shimPath: string;
  requestTimeoutMs: number;
  maxOutputChars: number;
}

/** Resolve the extension config from the environment. Pure. */
export function repoMapConfig(env: NodeJS.ProcessEnv = process.env): RepoMapConfig {
  const home = env.HOME || homedir();
  const repomapDir =
    env.OFFICINA_REPO_MAP_DIR || env.TRIS_REPO_MAP_DIR || resolve(home, "Desktop/Projects/repo-map");
  // SS3 (2026-08-31): OFF unless a real checkout exists at the resolved dir.
  // No external clone assumed; the ext degrades honestly when absent.
  const enabled = env.TRIS_NO_REPO_MAP !== "1" && existsSync(repomapDir);
  return {
    enabled,
    pythonBin: env.TRIS_REPO_MAP_PY || resolve(home, "venvs/repo-map/bin/python"),
    repomapDir,
    shimPath: fileURLToPath(new URL("shim.py", import.meta.url)),
    requestTimeoutMs: 120_000,
    maxOutputChars: 2000, // entry-side budget guard: outline/symbol capped (~500 tok)
  };
}

/** Cap tool output so a giant index/outline can never flood context. Pure. */
export function capText(text: string, maxChars: number): string {
  if (text.length <= maxChars) return text;
  return text.slice(0, maxChars) + `\n… [truncated ${text.length - maxChars} chars — full result on disk via repo-map cache]`;
}

interface Pending {
  resolve: (text: string) => void;
  reject: (err: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

/** One warm shim process with id-correlated JSONL requests. */
export class RepoMapClient {
  private proc: ChildProcessWithoutNullStreams | null = null;
  private buf = "";
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private indexed = new Set<string>();
  private spawns = 0;

  constructor(private cfg: RepoMapConfig) {}

  /** True when both the interpreter and the repo-map sources exist. */
  isAvailable(): boolean {
    return existsSync(this.cfg.pythonBin) && existsSync(this.cfg.shimPath) && existsSync(this.cfg.repomapDir + "/server.py");
  }

  /** Spawn the shim if not running; wait for its READY marker. */
  async ensure(): Promise<void> {
    if (this.proc && !this.proc.killed) return;
    if (this.spawns > 3) throw new Error("repo-map shim keeps dying — check TRIS_REPO_MAP_PY / repo-map install");
    if (!this.isAvailable()) throw new Error(`repo-map unavailable: need ${this.cfg.pythonBin} + ${this.cfg.repomapDir}`);
    this.spawns += 1;
    const proc = spawn(this.cfg.pythonBin, [this.cfg.shimPath, this.cfg.repomapDir], {
      stdio: ["pipe", "pipe", "pipe"],
    });
    proc.stdout.on("data", (chunk: Buffer) => this.onStdout(chunk));
    proc.on("exit", () => {
      this.proc = null;
      this.indexed.clear();
      this.rejectAll(new Error("repo-map shim exited"));
    });
    this.proc = proc;
    await this.waitForReady(proc);
  }

  /** Handshake: the shim prints READY on stderr once imports finish (~4s cold). */
  private waitForReady(proc: ChildProcessWithoutNullStreams): Promise<void> {
    return new Promise((res, rej) => {
      const to = setTimeout(() => rej(new Error("repo-map shim not READY within 60s")), 60_000);
      const onData = (chunk: Buffer) => {
        if (chunk.toString().includes("READY")) {
          clearTimeout(to);
          proc.stderr.off("data", onData);
          res();
        }
      };
      proc.stderr.on("data", onData);
      proc.on("exit", () => { clearTimeout(to); rej(new Error("shim died during import")); });
    });
  }

  private onStdout(chunk: Buffer): void {
    this.buf += chunk.toString();
    for (;;) {
      const nl = this.buf.indexOf("\n");
      if (nl < 0) return;
      const line = this.buf.slice(0, nl).trim();
      this.buf = this.buf.slice(nl + 1);
      if (!line) continue;
      this.dispatchLine(line);
    }
  }

  /** Resolve/reject one pending request from a parsed shim reply line. */
  private dispatchLine(line: string): void {
    let msg: { id?: number; ok?: boolean; text?: string; error?: string };
    try {
      msg = JSON.parse(line);
    } catch {
      return; // non-JSON noise on stdout is ignored, never fatal
    }
    const p = msg.id === undefined ? undefined : this.pending.get(msg.id);
    if (!p) return;
    this.pending.delete(msg.id as number);
    clearTimeout(p.timer);
    if (msg.ok) p.resolve(String(msg.text ?? ""));
    else p.reject(new Error(String(msg.error ?? "repo-map tool failed").slice(0, 400)));
  }

  private rejectAll(err: Error): void {
    for (const [, p] of this.pending) {
      clearTimeout(p.timer);
      p.reject(err);
    }
    this.pending.clear();
  }

  /** Send one command; reject on timeout or shim error reply. */
  async request(cmd: string, args: Record<string, unknown>): Promise<string> {
    await this.ensure();
    const proc = this.proc;
    if (!proc) throw new Error("repo-map shim not running");
    const id = this.nextId++;
    return await new Promise<string>((res, rej) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        rej(new Error(`repo-map ${cmd} timed out after ${this.cfg.requestTimeoutMs}ms`));
      }, this.cfg.requestTimeoutMs);
      this.pending.set(id, { resolve: res, reject: rej, timer });
      proc.stdin.write(JSON.stringify({ id, cmd, args }) + "\n");
    });
  }

  /** Index a repo (idempotent per process — repo-map itself caches incrementally). */
  async index(path: string): Promise<string> {
    const text = await this.request("index", { path });
    this.indexed.add(path);
    return text;
  }

  /** Index if this process has never targeted the repo. */
  async ensureIndexed(path: string): Promise<void> {
    if (this.indexed.has(path)) return;
    await this.index(path);
  }

  /** Close the shim (session teardown / tests). */
  shutdown(): void {
    if (!this.proc) return;
    this.proc.stdin.end();
    this.proc.kill();
    this.proc = null;
  }
}
