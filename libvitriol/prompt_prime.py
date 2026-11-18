# 2026-11-18 13:58 — Explicit bounded client-prefix priming; no default launcher changes.
# PROVENANCE: original implementation of standard HTTP/chat-template prefix reuse;
# no third-party algorithm or implementation copied.
"""Prime a resident engine's shared chat context before a client accepts work."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any


def validate_url(value: str) -> str:
    parsed = urllib.parse.urlsplit(value)
    if (
        parsed.scheme not in ("http", "https")
        or not parsed.hostname
        or parsed.username
        or parsed.password
        or parsed.query
        or parsed.fragment
    ):
        raise ValueError(
            "engine URL requires HTTP(S), hostname and no credentials/query/fragment"
        )
    return value.rstrip("/")


def digest(value: Any) -> str:
    return hashlib.sha256(
        json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


class EngineHTTP:
    def __init__(self, url: str, timeout: float) -> None:
        if not math.isfinite(timeout) or not 0 < timeout <= 600:
            raise ValueError("priming timeout must be finite and in (0,600] seconds")
        self.url = validate_url(url)
        self.deadline = time.monotonic() + timeout
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def request(
        self, route: str, body: dict | None = None, cap: float | None = None
    ) -> dict:
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError(f"priming deadline expired before {route}")
        request = urllib.request.Request(
            self.url + route,
            data=None if body is None else json.dumps(body).encode(),
            headers={"Content-Type": "application/json"},
        )
        try:
            with self.opener.open(
                request, timeout=min(remaining, cap or remaining)
            ) as response:
                result = json.load(response)
        except (OSError, ValueError, urllib.error.URLError) as exc:
            raise RuntimeError(f"engine request {route} failed: {exc}") from exc
        if time.monotonic() > self.deadline:
            raise TimeoutError(f"priming deadline expired during {route}")
        if not isinstance(result, dict):
            raise TypeError(f"engine {route} must return a JSON object")
        return result

    def ready(self) -> None:
        while True:
            try:
                if self.request("/health", cap=1).get("status") == "ok":
                    return
            except RuntimeError:
                pass
            remaining = self.deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("engine not healthy before priming deadline")
            time.sleep(min(0.1, remaining))


def prime(
    url: str,
    messages: list[dict],
    model: str | None = None,
    timeout: float = 30,
    transport: EngineHTTP | None = None,
) -> dict:
    """Evaluate only a template-derived shared prefix; return no generated answer."""
    if (
        not isinstance(messages, list)
        or not messages
        or any(
            not isinstance(m, dict)
            or not isinstance(m.get("role"), str)
            or not isinstance(m.get("content"), str)
            for m in messages
        )
    ):
        raise ValueError(
            "messages must be a nonempty JSON array of text role/content objects"
        )
    started = time.monotonic()
    client = transport or EngineHTTP(url, timeout)
    client.ready()
    token_sets = []
    for marker in ("!", "~"):
        body: dict[str, Any] = {
            "messages": messages + [{"role": "user", "content": marker}]
        }
        if model:
            body["model"] = model
        rendered = client.request("/apply-template", body).get("prompt")
        if not isinstance(rendered, str) or not rendered:
            raise ValueError("/apply-template did not return a nonempty text prompt")
        tokens = client.request(
            "/tokenize",
            {"content": rendered, "add_special": False, "parse_special": True},
        ).get("tokens")
        if (
            not isinstance(tokens, list)
            or not tokens
            or any(type(t) is not int or t < 0 for t in tokens)
        ):
            raise ValueError("/tokenize did not return nonnegative integer token IDs")
        token_sets.append(tokens)
    prefix = []
    for left, right in zip(*token_sets):
        if left != right:
            break
        prefix.append(left)
    if not prefix or len(prefix) >= min(map(len, token_sets)):
        raise ValueError("template probes must diverge after a nonempty shared prefix")
    result = client.request(
        "/completion",
        {
            "prompt": prefix,
            "n_predict": 1,
            "temperature": 0,
            "cache_prompt": True,
            "stream": False,
        },
    )
    timing = result.get("timings", {})
    counts = [timing.get(k) for k in ("cache_n", "prompt_n", "predicted_n")]
    if (
        any(type(v) is not int or v < 0 for v in counts)
        or counts[0] + counts[1] != len(prefix)
        or counts[2] != 1
        or result.get("truncated") is not False
        or result.get("stop") is not True
    ):
        raise ValueError(
            "prefix priming not admitted: need complete untruncated inference, "
            "cache_n+prompt_n=prefix length and predicted_n=1"
        )
    return {
        "version": 1,
        "ready": True,
        "url": validate_url(url),
        "model": model,
        "messages_sha256": digest(messages),
        "prefix_sha256": digest(prefix),
        "prefix_tokens": len(prefix),
        "timings": timing,
        "sampled_tokens_discarded": 1,
        "seconds": time.monotonic() - started,
        "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", type=validate_url, required=True)
    parser.add_argument("--messages-file", type=Path, required=True)
    parser.add_argument("--model")
    parser.add_argument("--timeout", type=float, default=30)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        messages = json.loads(args.messages_file.read_text())
        print(
            "VITRIOL-FINGERPRINT: "
            + json.dumps(
                {
                    "role": "client-prefix-primer",
                    "argv": sys.argv,
                    "url": args.url,
                    "model": args.model,
                    "messages_sha256": digest(messages),
                    "timeout": args.timeout,
                    "n_predict": 1,
                    "cache_prompt": True,
                    "temperature": 0,
                },
                sort_keys=True,
            ),
            flush=True,
        )
        result = prime(args.url, messages, args.model, args.timeout)
        result["argv"] = sys.argv
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile(
            mode="w", dir=args.output.parent, delete=False
        ) as stream:
            temporary = Path(stream.name)
            try:
                json.dump(result, stream, indent=2)
                stream.write("\n")
                stream.flush()
                os.replace(temporary, args.output)
            finally:
                temporary.unlink(missing_ok=True)
        print(json.dumps(result, sort_keys=True), flush=True)
        return 0
    except (OSError, TypeError, ValueError, RuntimeError, TimeoutError) as exc:
        print(f"VITRIOL prefix priming failed: {exc}", file=sys.stderr, flush=True)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
