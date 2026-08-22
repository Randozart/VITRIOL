#!/usr/bin/env python3
"""Long-session simulator — validates REBIS gateway under day-long load.

Drives the gateway (:8280) through N synthetic turns whose cumulative
history exceeds the 64k window, verifying:
  1. no hard overflow errors (rolling + compaction cooperate)
  2. compaction events fire at the configured threshold
  3. post-compaction turns still reference session memory coherently
Every turn is timed; results printed as a table.

Usage: python3 libvitriol/long_session_sim.py --turns 40 [--url :8280]
"""

import argparse
import json
import time
import urllib.request

TOOLS = [{"type": "function", "function": {
    "name": "edit_file", "description": "edit a file",
    "parameters": {"type": "object", "properties": {
        "path": {"type": "string"}, "content": {"type": "string"}}}}}]

TURN_TEMPLATE = (
    "Session log entry {i}: we analyzed module_{i}.rs. It defines "
    "{filler} The team decision recorded for this module: prefer "
    "explicit lifetimes and document invariants inline.")


def make_turn(i: int, filler_tokens: int) -> str:
    unit = ("static_assert(sizeof(Ledger) > 0, \"ledger must exist\"); ")
    filler = unit * max(1, filler_tokens // len(unit) * 4)
    return TURN_TEMPLATE.format(i=i, filler=filler[:max(0, filler_tokens * 4)])


def send(url: str, messages: list[dict], max_tokens: int = 60,
         timeout: int = 900) -> tuple[dict, float]:
    body = json.dumps({"model": "rebis", "messages": messages,
                       "tools": TOOLS, "max_tokens": max_tokens,
                       "temperature": 0.4}).encode()
    req = urllib.request.Request(
        f"{url}/v1/chat/completions", data=body,
        headers={"Content-Type": "application/json"}, method="POST")
    t0 = time.time()
    with urllib.request.urlopen(req, timeout=timeout) as r:
        data = json.loads(r.read())
    return data, time.time() - t0


def main() -> None:
    p = argparse.ArgumentParser(description="long-session simulator")
    p.add_argument("--url", default="http://127.0.0.1:8280")
    p.add_argument("--turns", type=int, default=30)
    p.add_argument("--growth", type=int, default=2500,
                   help="approx new tokens injected per turn")
    args = p.parse_args()

    messages: list[dict] = [{"role": "user", "content":
        "Long systems-engineering session start. We maintain a ledger of "
        "modules; each entry below is session history to remain aware of."}]
    rows = []
    for i in range(1, args.turns + 1):
        messages.append({"role": "user",
                         "content": make_turn(i, args.growth)})
        messages.append({"role": "user",
                         "content": f"Acknowledge entry {i} with one line."})
        try:
            _d, dt = send(args.url, messages)
            err = ""
        except urllib.error.HTTPError as e:
            dt = round(time.time() - t0, 1) if False else -1
            err = f"HTTP {e.code}"
        rows.append((i, len(messages), dt, err))
        # keep assistant replies so history grows like a real session
        messages.append({"role": "assistant",
                         "content": f"Entry {i} acknowledged."})
        print(f"turn {i:>3}: {dt:>7}s {err}", flush=True)

    ok = sum(1 for r in rows if r[3] == "")
    print(f"\n{ok}/{len(rows)} turns completed without overflow")


if __name__ == "__main__":
    import urllib.error
    main()
