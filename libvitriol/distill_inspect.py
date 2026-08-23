#!/usr/bin/env python3
"""Inspect REBIS training-data capture — read-only summaries over
~/.vitriol/distill/*.jsonl.

Modes:
  summary   per-file event counts, sessions, token spend     (default)
  turns     one line per captured draft/turn with outcome
  pairs     correction pairs (rejected draft → final) for DPO review
  export    SFT chat-format preview (packet → accepted output)

Usage: python3 libvitriol/distill_inspect.py [mode] [--dir PATH] [--session KEY]
"""

import argparse
import glob
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path

DEFAULT_DIR = str(Path.home() / ".vitriol" / "distill")


def load(directory: str) -> list[tuple[str, dict]]:
    records = []
    for f in sorted(glob.glob(f"{directory}/*.jsonl")):
        for line in open(f, errors="replace"):
            try:
                records.append((Path(f).name, json.loads(line)))
            except json.JSONDecodeError:
                continue
    return records


def fmt_tok(n) -> str:
    return f"{n:,}" if isinstance(n, (int, float)) else str(n)


def summary(records) -> None:
    by_file = defaultdict(Counter)
    sessions = set()
    spend = {"drafter": 0, "verifier": 0}
    for fname, ev in records:
        by_file[fname][ev.get("type", "?")] += 1
        if ev.get("session"):
            sessions.add(ev["session"])
        u = ev.get("usage") or {}
        spend["drafter"] += int(u.get("completion_tokens") or 0)
    print(f"files: {len(by_file)} | events: {len(records)} | sessions: {len(sessions)}")
    for fname, counter in sorted(by_file.items()):
        print(f"\n{fname}")
        for kind, n in counter.most_common():
            print(f"  {kind:<20} {n}")
    turns = [ev for _, ev in records if ev.get("type") == "turn"]
    for t in turns:
        du = t.get("drafter_usage") or {}
        spend["drafter"] += int(du.get("completion_tokens") or 0)
    print(f"\ndrafter completion tokens (turn records): {fmt_tok(spend['drafter'])}")


def turns(records) -> None:
    for fname, ev in records:
        t = ev.get("type")
        if t == "turn" and "iteration" in ev:
            print(f"{fname[:28]:<28} it{ev['iteration']} "
                  f"compile={'✓' if ev.get('compile_ok') else '✗'} "
                  f"verdict={'✓' if ev.get('verdict_pass') else '✗'} "
                  f"delta={len(ev.get('delta') or [])}")
        elif ev.get("type") == "draft" and "text" in ev:
            print(f"{fname[:28]:<28} draft {len(ev['text'])} chars")


def pairs(records) -> None:
    """Group by task file: last rejected draft vs accepted final per run."""
    runs = defaultdict(list)
    for fname, ev in records:
        runs[fname].append(ev)
    n = 0
    for fname, events in runs.items():
        drafts = [e for e in events if e.get("type") == "draft"]
        closes = [e for e in events if e.get("type") == "run_close"]
        accepted = any(c.get("accepted") for c in closes)
        if len(drafts) >= 2 and accepted:
            n += 1
            print(f"{fname}: {len(drafts)} drafts, accepted ✓ "
                  f"→ {len(drafts)-1} preference pair(s) available")
    if n == 0:
        print("no multi-draft accepted runs yet (pairs form once a run "
              "iterates before accepting)")


def export(records) -> None:
    """SFT chat-format preview from accepted runs."""
    for fname, ev in records:
        if ev.get("type") == "run_open":
            print(json.dumps({
                "messages": [
                    {"role": "user", "content": ev.get("objective", "")},
                    {"role": "assistant",
                     "content": "<accepted final from this run's files_after>"},
                ],
                "_invariants": ev.get("invariants"),
            }, ensure_ascii=False)[:200])
    print("(preview — wire files_after content for full export)")


def main() -> int:
    p = argparse.ArgumentParser(description="inspect REBIS training data")
    p.add_argument("mode", nargs="?", default="summary",
                   choices=["summary", "turns", "pairs", "export"])
    p.add_argument("--dir", default=DEFAULT_DIR)
    args = p.parse_args()

    records = load(args.dir)
    if not records:
        print(f"no records in {args.dir}")
        return 1
    {"summary": summary, "turns": turns,
     "pairs": pairs, "export": export}[args.mode](records)
    return 0


if __name__ == "__main__":
    sys.exit(main())
