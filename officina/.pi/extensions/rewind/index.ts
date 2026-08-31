import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { execFile } from "node:child_process";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { turnFilename, turnRef } from "../_shared/turnkeys.ts";
import { formatPlan, parseRefs, pairTurns, type TurnPair } from "./rewind.ts";

// /rewind [turn] — paired code+KV time travel (gap ④).
//   no arg  -> list available snapshots (refs/trismegistus/turns/*)
//   <turn>  -> confirm dialog (TUI only) -> git checkout ref -- . AND
//              POST /slots/<id>?action=restore of <session>-turn-<n>.bin
// Degrades honestly: a half with no file is reported, never guessed.
// Requires TRIS_SNAPSHOT=1 (snapshot ext) to HAVE refs; KV half depends on
// checkpoint autosave. Headless: refuses (a destructive op needs a human).
// Kill switch: TRIS_NO_REWIND=1.

export interface RewindDeps {
  git: (argv: string[]) => Promise<string>;
  restore: (filename: string) => Promise<{ ok: boolean; note: string }>;
}

const defaultDeps: RewindDeps = {
  git: (argv) => new Promise((res, rej) =>
    execFile("git", argv, { cwd: process.cwd(), maxBuffer: 4 << 20 }, (e, out) => (e ? rej(e) : res(out)))),
  async restore(filename) {
    const base = (process.env.VITRIOL_BASE_URL || "http://127.0.0.1:8279/v1").replace(/\/v1\/?$/, "");
    const slot = process.env.TRIS_CKPT_SLOT ?? "0";
    try {
      const r = await fetch(`${base}/slots/${slot}?action=restore`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ filename }),
        signal: AbortSignal.timeout(120_000),
      });
      return r.ok ? { ok: true, note: "KV slot restored" } : { ok: false, note: `engine refused restore (HTTP ${r.status})` };
    } catch (e) {
      return { ok: false, note: `restore failed: ${(e as Error).message.slice(0, 80)}` };
    }
  },
};

export default function (pi: ExtensionAPI, deps: RewindDeps = defaultDeps) {
  if (process.env.TRIS_NO_REWIND === "1") return;

  async function listPairs(ctx: { cwd: string }): Promise<TurnPair[]> {
    const prefix = (process.env.TRIS_SNAPSHOT_PREFIX || "refs/trismegistus/turns") + "/";
    const out = await deps.git(["for-each-ref", "--format=%(refname)", prefix]).catch(() => "");
    const current = Number(process.env.TRIS_REWIND_CURRENT ?? 9999);
    return pairTurns(parseRefs(out), current);
  }

  pi.registerCommand("rewind", {
    description: "Rewind worktree + engine KV to a snapshot turn: /rewind [N] (no arg lists)",
    getArgumentCompletions: async (prefix) => {
      const pairs = await listPairs({ cwd: process.cwd() }).catch(() => [] as TurnPair[]);
      return pairs.map((p) => ({ value: String(p.turn), label: `turn ${p.turn}` }));
    },
    handler: async (args, ctx) => {
      const c = ctx as { mode?: string; hasUI?: boolean };
      if (c.mode === "print" || c.mode === "json" || !c.hasUI) {
        ctx.ui.notify("rewind: refuses in headless mode (destructive — run it interactively)", "warning");
        return;
      }
      const pairs = await listPairs(ctx as { cwd: string });
      const arg = args.trim();
      if (!arg) {
        ctx.ui.notify(pairs.length ? `rewind: turns available ${pairs.map((p) => p.turn).join(", ")}` : "rewind: no snapshot refs (arm snapshot with TRIS_SNAPSHOT=1)", "info");
        return;
      }
      const turn = Number(arg);
      if (!Number.isInteger(turn) || turn < 0) {
        ctx.ui.notify(`rewind: not a turn number: '${arg}'`, "error");
        return;
      }
      const sm = (ctx as { sessionManager?: { getSessionFile?: () => string | null } }).sessionManager;
      const stem = sm?.getSessionFile?.()?.split("/").pop()?.replace(/\.jsonl$/, "")
        || process.env.TRIS_REWIND_SESSION || "session";
      const filename = turnFilename(turn, stem);
      const go = await ctx.ui.confirm("rewind", formatPlan(turn, filename, pairs));
      if (!go) {
        ctx.ui.notify("rewind: declined — nothing touched", "info");
        emitHarnessEvent(harnessEvent("lc-ckpt", "rewind-declined", { turn, detail: filename }));
        return;
      }
      let codeNote = "code: no snapshot ref for turn " + turn;
      let codeOk = false;
      try {
        await deps.git(["rev-parse", "--verify", turnRef(turn)]);
        await deps.git(["checkout", turnRef(turn), "--", "."]);
        codeNote = "code restored";
        codeOk = true;
      } catch {
        // missing ref: report, KV half still attempted (halves are independent)
      }
      const kv = await deps.restore(filename);
      emitHarnessEvent(harnessEvent("lc-ckpt", "rewound", { turn, detail: `${codeNote}; ${kv.note}` }));
      const both = codeOk && kv.ok;
      ctx.ui.notify(`rewind turn ${turn} (${filename}): ${codeNote}; ${kv.note}${both ? " — VERIFY the conversation matches the rewound state" : ""}`, both ? "info" : "warning");
    },
  });
}
