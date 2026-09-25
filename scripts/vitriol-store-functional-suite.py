#!/usr/bin/env python3
"""Functional smoke suite for a VITRIOL store-backed llama-server."""

import argparse
import datetime
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path

PROMPTS = [
    {
        "id": "factual",
        "prompt": "What is the capital of France? Answer in one sentence.",
        "n_predict": 48,
    },
    {
        "id": "arithmetic",
        "prompt": "What is 17 * 24? Show the calculation, then give the final answer.",
        "n_predict": 64,
    },
    {
        "id": "rewrite",
        "prompt": "Rewrite this sentence clearly: The report was wrote by team and it were late.",
        "n_predict": 64,
    },
    {
        "id": "code",
        "prompt": "Write a Python function that returns the median of three numbers. Include a short usage example.",
        "n_predict": 96,
    },
    {
        "id": "explanation",
        "prompt": "Explain in two sentences why ice floats on water.",
        "n_predict": 64,
    },
    {
        "id": "constrained",
        "prompt": "List exactly three primary colors as a JSON array of strings.",
        "n_predict": 48,
    },
    {
        "id": "translation",
        "prompt": "Translate 'Good morning, how are you?' into French.",
        "n_predict": 48,
    },
    {
        "id": "reasoning",
        "prompt": "A shop sells pens in packs of 5. If Lisa buys 3 packs, how many pens does she have? Answer briefly.",
        "n_predict": 64,
    },
]


def request(base_url: str, payload: dict, timeout: float) -> dict:
    body = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        base_url.rstrip("/") + "/completion",
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as res:
        if res.status != 200:
            raise RuntimeError(f"HTTP {res.status}")
        return json.loads(res.read().decode("utf-8"))


def quality_errors(prompt: str, content: str) -> list[str]:
    errors = []
    text = content.strip()
    if not text:
        return ["empty response"]
    if text == prompt.strip():
        errors.append("response echoes prompt")

    printable = sum(1 for ch in text if ch.isprintable() or ch in "\n\r\t")
    if printable / len(text) < 0.85:
        errors.append("too many non-printable characters")

    alphanumeric = sum(1 for ch in text if ch.isalnum())
    if alphanumeric < 3:
        errors.append("too few alphanumeric characters")

    collapsed = " ".join(text.split())
    words = collapsed.split(" ")
    if len(words) >= 4 and len(set(words)) / len(words) < 0.15:
        errors.append("highly repetitive words")

    chars = [ch for ch in text if not ch.isspace()]
    if chars:
        common = max(chars.count(ch) for ch in set(chars))
        if common / len(chars) > 0.9:
            errors.append("dominated by one character")

    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default="http://127.0.0.1:8404")
    parser.add_argument("--rounds", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=180.0)
    parser.add_argument("--output", help="write JSON results to this file")
    args = parser.parse_args()

    if args.rounds < 1:
        parser.error("--rounds must be positive")

    started = datetime.datetime.now(datetime.timezone.utc).isoformat()
    rounds = []
    failures = 0

    for round_index in range(1, args.rounds + 1):
        records = []
        for item in PROMPTS:
            payload = {
                "prompt": item["prompt"],
                "n_predict": item["n_predict"],
                "temperature": 0.0,
                "top_k": 1,
                "seed": 1729,
                "cache_prompt": False,
                "return_tokens": True,
            }
            record = {
                "round": round_index,
                "id": item["id"],
                "prompt": item["prompt"],
                "n_predict": item["n_predict"],
            }
            try:
                result = request(args.base_url, payload, args.timeout)
                content = result.get("content", "")
                record["content"] = content
                record["tokens"] = result.get("tokens")
                record["tokens_evaluated"] = result.get("tokens_evaluated")
                record["tokens_predicted"] = result.get("tokens_predicted")
                record["timings"] = result.get("timings")
                record["quality_errors"] = quality_errors(item["prompt"], content)
                record["ok"] = not record["quality_errors"]
            except Exception as exc:
                record["error"] = f"{type(exc).__name__}: {exc}"
                record["ok"] = False
            if not record["ok"]:
                failures += 1
            records.append(record)
            print(
                f"round={round_index} id={item['id']} ok={record['ok']} "
                f"content={record.get('content', '')[:120]!r}",
                flush=True,
            )
        rounds.append(records)

    report = {
        "started_utc": started,
        "finished_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "base_url": args.base_url,
        "rounds": args.rounds,
        "prompts": len(PROMPTS),
        "requests": args.rounds * len(PROMPTS),
        "failures": failures,
        "ok": failures == 0,
        "results": rounds,
    }
    text = json.dumps(report, indent=2, ensure_ascii=False)
    if args.output:
        output = Path(args.output)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(text + "\n", encoding="utf-8")
    else:
        print(text)

    print(f"functional_suite ok={report['ok']} failures={failures}")
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
