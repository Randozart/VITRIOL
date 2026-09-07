# Session Stall Postmortem + Long-Tool UX Gap

**Date**: 2026-09-07
**Status**: INVESTIGATED / PARTIALLY FIXED
**Owner**: randozart
**Trigger**: owner reports a session "just stopped and didn't pick back up";
follow-up: scratchpad denial citing a "limit of 200".

---

## Incident chain (morning 2026-09-07)

1. **Engine down since 23:40:57 sep-06** — `vitriol-server.service`
   `inactive (dead)`. fish history timestamp `23:40:45 vitriol stop`:
   **the owner stopped the engine manually last night** (clean exit,
   `Restart=always` does not restart an explicit stop). Sidecar had saved
   `slot0.bin` at 23:21 (31,229 tokens / 733 MB) before the stop.
2. Ontic/Officina session (`2026-09-06T20-19-09` — ontic project) stalled
   after that: any turn on Lapis Occultus hit a dead port 8279.
3. **Restart via unit** (`systemctl start vitriol-server.service`, polkit
   rule, no sudo): health 200 in ~5s, fingerprint EXACT match to blessed
   (`ts=24,12 c=81920 kv=q4_0/q4_0 spec=mtp:1 par=1 score=probe cram=256`),
   warm resume restored 31,229 tokens in 1.4s. VRAM ~81% both GPUs.
4. **Re-hang on first turn in the restarted TUI** — investigated at the
   kernel level: pi host alive, event loop idle in `epoll_wait`, no child
   process, session file ending at an assistant `bash` toolCall (cargo
   test, `timeout: 1800`) with no toolResult. Engine served the completion
   (POST 200, input 31,306, 3-min prefill of the restored context). The
   wedge **self-healed**; the tool eventually completed (toolResult 11:32).
   Unexplained — logged below as a watch-item.

## Findings

### 1. Manual `vitriol stop` = undiagnosable stall
Nothing wrong with the engine; a stopped engine looks identical to a hang
from the TUI. The engine-status row existed but nothing surfaced it at turn
time.

### 2. The pi "park" (unresolved)
pi (node) idle in `epoll_wait`, all threads parked, between "assistant
toolCall appended" (10:54:52) and "tool subprocess spawn". No coredump, no
OOM, no children, no second POST. Self-healed. **Suspects**: (a) RPC-mode
tool approval round-trip with no answer (TUI launched with `-a`, but the
`-a` semantics in the minified cli are unverified for tool permissions);
(b) resume-with-dangling-toolCall reconciliation; (c) our extension
globalThis rewrite hitting an async handler the test harness can't reach
(538 vitest green, but vitest ≠ jiti runtime paths). Next occurrence:
**run `/diag` in the TUI first** (pi stderr ring buffer, bridge.rs:689)
before any other diagnosis.

### 3. Long bash reads as a hang (FIXED)
`cargo test 2>&1 | tail -30` buffers every byte until EOF — the TUI's
`tool_execution_update` streaming gets nothing for 12+ minutes. No elapsed
timer existed. **Fix** (`15098c5`): running tool rows now render elapsed
wall time live (cache tick advances once per second on the 500ms heartbeat
redraw), the declared timeout, and after 30s of empty output a
"no output yet — pipe-buffered?" hint. Installed to `~/.local/bin/officina`
(needs TUI restart to load).

### 4. Scratchpad "limit of 200" confusion (FIXED)
`maxLineChars: 200` is a **per-line char** budget, but the tool description
only advertised the 512-line cap — so a denial read as a contradictory
limit. Fix (`earlier commit`): denial now names the dimension explicitly
(`context[2]: 312 chars — exceeds 200-char max per line (separate from the
512-line cap)`) and the description states the per-line budget. Cap left at
200 per owner. Tests assert the new phrasing.

## Cleanup also performed

- **Frozen process pair**: opencode 3573612 + `praetor` child (sep-02,
  SIGSTOP'd, VITRIOL cwd) — SIGTERM sat pending on the stopped pair;
  SIGKILL removed them.
- **`mongod-vitriol.service` disabled**: crash-looping every 5s since
  ~Aug-31 (124,630 restarts) — `/usr/bin/mongod` vanished in a package
  update. Pymander vector store dead since then; `vitriol_rag.py` running
  degraded. Re-enable only after reinstalling mongod.

## Watch-items

- pi park (item 2) — `/diag` first, then decide.
- A stopped engine looks like a hang: consider surfacing engine-down in the
  turn path (not just the sidebar row).
- The running TUI still executes the old binary (inode swapped aside) —
  restart `officina` to pick up the elapsed-timer fix.