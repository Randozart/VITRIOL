#!/usr/bin/env python3
"""Measure streamed client latency and paired TUI overhead on an isolated engine.

Prior art: density-bench/src/engine.rs (same repository). This client uses a
single native /completion stream for both first-content latency and final
engine timings; empty or role-only chunks cannot count as a generated token.
"""
import argparse
import contextlib
import fcntl
import hashlib
import json
import math
import os
import pty
import random
import select
import statistics
import struct
import subprocess
import termios
import threading
import time
import urllib.parse
import urllib.request
from pathlib import Path

PROMPT = "Write a Rust function to merge two sorted slices. Explain edge cases.\n" * 32


def post(base, path, body):
    request = urllib.request.Request(
        base + path, json.dumps(body).encode(), {"Content-Type": "application/json"}
    )
    return urllib.request.urlopen(request, timeout=180)


def stream_sample(base, tokens):
    started = time.perf_counter()
    first = None
    timings = None
    content_hash = hashlib.sha256()
    with post(base, "/completion", {
        "prompt": PROMPT, "stream": True, "n_predict": tokens,
        "temperature": 0, "seed": 42, "ignore_eos": True, "cache_prompt": False,
    }) as response:
        for line in response:
            if not line.startswith(b"data:"):
                continue
            data = line[5:].strip()
            if data == b"[DONE]":
                break
            value = json.loads(data)
            if value.get("error"):
                raise RuntimeError(f"engine stream error: {value['error']}")
            content = value.get("content")
            if isinstance(content, str) and content:
                if first is None:
                    first = time.perf_counter() - started
                content_hash.update(content.encode())
            if value.get("stop"):
                timings = value.get("timings")
    if first is None or not isinstance(timings, dict):
        raise RuntimeError("stream lacks generated content or final engine timings")
    predicted = timings.get("predicted_n")
    duration = timings.get("predicted_ms")
    if (predicted != tokens or not isinstance(duration, (float, int))
            or isinstance(duration, bool) or not math.isfinite(duration) or duration <= 0):
        raise RuntimeError(f"incomplete or invalid engine timing: {timings}")
    return {"client_first_content_s": first, "wall_s": time.perf_counter() - started,
            "timings": timings, "decode_tps": predicted * 1000 / duration,
            "content_sha256": content_hash.hexdigest()}


def engine_identity(pid, port):
    process = Path(f"/proc/{pid}")
    args = process.joinpath("cmdline").read_bytes().rstrip(b"\0").split(b"\0")
    decoded = [os.fsdecode(arg) for arg in args]
    if "--port" not in decoded or decoded[decoded.index("--port") + 1] != str(port):
        raise ValueError("PID must name the isolated engine serving the selected port")
    stat = process.joinpath("stat").read_text().rsplit(")", 1)[1].split()
    return {"pid": pid, "start_ticks": stat[19], "argv": decoded}


def validate_target(base):
    url = urllib.parse.urlparse(base)
    if (url.scheme != "http" or url.hostname != "127.0.0.1"
            or url.port is None or url.port == 8279 or url.path not in ("", "/")
            or url.username or url.password or url.query or url.fragment):
        raise ValueError("use an isolated http://127.0.0.1:PORT endpoint; production 8279 is excluded")
    return url.port


@contextlib.contextmanager
def dashboard(binary, port, log_dir):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 38, 125, 0, 0))
    env = dict(os.environ, TERM="xterm-256color", VITRIOL_GEN_PORT=str(port),
               COPULA_LOG_DIR=str(log_dir), VITRIOL_TUI_THEME="vitriolum")
    process = None
    finished = threading.Event()

    def drain():
        while not finished.is_set():
            if select.select([master], [], [], 0.1)[0]:
                try:
                    packet = os.read(master, 65536)
                    if b"\x1b[6n" in packet:
                        os.write(master, b"\x1b[1;1R")
                except OSError:
                    return

    try:
        process = subprocess.Popen([str(binary)], stdin=slave, stdout=slave,
                                   stderr=slave, env=env)
        os.close(slave)
        slave = None
        reader = threading.Thread(target=drain, daemon=True)
        reader.start()
        time.sleep(5)  # allow the initial full poll to finish before timing
        if process.poll() is not None:
            raise RuntimeError("dashboard exited before measurement")
        yield
        if process.poll() is not None:
            raise RuntimeError("dashboard exited during measurement")
    finally:
        if process is not None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        finished.set()
        if process is not None and 'reader' in locals():
            reader.join(timeout=1)
        os.close(master)
        if slave is not None:
            os.close(slave)


def overhead_summary(pairs):
    """Paired fractional decode-time change; bootstrap pairs, preserving pairing."""
    changes = [pair["off"]["decode_tps"] / pair["on"]["decode_tps"] - 1 for pair in pairs]
    rng = random.Random(42)
    means = sorted(statistics.mean(rng.choices(changes, k=len(changes))) for _ in range(5000))
    interval = [means[125], means[4874]]
    return {"pairs": len(pairs), "mean_decode_overhead_fraction": statistics.mean(changes),
            "bootstrap_95_percent_interval": interval,
            "below_one_percent": interval[1] < 0.01,
            "interpretation": "isolated workload only; unresolved if interval crosses 1%"}


def checkpoint_sample(base, operation, filename):
    started = time.perf_counter()
    with post(base, f"/slots/0?action={operation}", {"filename": filename}) as response:
        result = json.load(response)
    if result.get("pending"):
        raise RuntimeError("asynchronous save is not a completed checkpoint operation")
    return {"operation": operation, "http_wall_s": time.perf_counter() - started,
            "response": result}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--engine-log", type=Path, required=True)
    parser.add_argument("--dashboard", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--pairs", type=int, default=16)
    parser.add_argument("--tokens", type=int, default=256)
    parser.add_argument("--checkpoint", action="store_true", help="save and restore isolated slot 0")
    args = parser.parse_args()
    port = validate_target(args.base)
    if args.pairs < 6 or args.tokens < 128:
        parser.error("at least 6 pairs and 128 tokens are required")
    identity = engine_identity(args.pid, port)
    if Path(f"/proc/{args.pid}/fd/1").resolve() != args.engine_log.resolve():
        parser.error("engine stdout must match --engine-log")
    record = {"schema": 1, "base": args.base, "engine": identity,
              "benchmark_argv": __import__('sys').argv,
              "dashboard_sha256": hashlib.sha256(args.dashboard.read_bytes()).hexdigest(),
              "prompt_sha256": hashlib.sha256(PROMPT.encode()).hexdigest(),
              "tokens": args.tokens, "pairs": [], "checkpoint_io": []}
    try:
        stream_sample(args.base, args.tokens)  # untimed warmup, discarded
        for pair_number in range(args.pairs):
            pair = {}
            for arm in (["off", "on"] if pair_number % 2 == 0 else ["on", "off"]):
                if engine_identity(args.pid, port) != identity:
                    raise RuntimeError("engine identity changed")
                manager = (dashboard(args.dashboard, port, args.engine_log.parent)
                           if arm == "on" else contextlib.nullcontext())
                with manager:
                    if arm == "off":
                        time.sleep(5)  # match the dashboard startup settling interval
                    stream_sample(args.base, args.tokens)  # same warmup in both arms
                    pair[arm] = stream_sample(args.base, args.tokens)
            record["pairs"].append(pair)
            print(f"pair {pair_number + 1}/{args.pairs} off={pair['off']['decode_tps']:.2f} "
                  f"on={pair['on']['decode_tps']:.2f}", flush=True)
        record["overhead"] = overhead_summary(record["pairs"])
        if args.checkpoint:
            filename = f"telemetry-{os.getpid()}.bin"
            for operation in ["save", "restore"]:
                if engine_identity(args.pid, port) != identity:
                    raise RuntimeError("engine identity changed")
                record["checkpoint_io"].append(checkpoint_sample(args.base, operation, filename))
        if engine_identity(args.pid, port) != identity:
            raise RuntimeError("engine identity changed")
        record["status"] = "completed"
    except KeyboardInterrupt:
        record["status"] = "interrupted"
        raise
    except Exception as error:
        record["status"] = "failed"
        record["error"] = str(error)
        raise
    finally:
        record["engine_log_sha256"] = hashlib.sha256(args.engine_log.read_bytes()).hexdigest()
        args.output.write_text(json.dumps(record, indent=2, allow_nan=False) + "\n")


if __name__ == "__main__":
    main()
