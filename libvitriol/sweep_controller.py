#!/usr/bin/env python3
"""VITRIOL Sweep Controller — empirically finds optimal pin/MTP/ubatch config.
Calls llama-server directly with env vars (same pattern as vitriol serve).
"""

import argparse
import csv
import json
import math
import os
import signal
import subprocess
import sys
import time
import urllib.request
import urllib.error
from pathlib import Path
from dataclasses import dataclass, field, asdict

# Paths
PROJECT_DIR = Path(__file__).resolve().parent.parent
LLAMA_DIR = PROJECT_DIR / "llama.cpp"
SERVER_BINARY = LLAMA_DIR / "build-rebis" / "bin" / "llama-server"
SWEEP_PORT = int(__import__("os").environ.get("REBIS_SWEEP_PORT", "8290"))
SKIP_STOP = __import__("os").environ.get("REBIS_SWEEP_SKIP_STOP", "") == "1"
CONFIG_DIR = Path.home() / ".vitriol"
SERVER_LOG = CONFIG_DIR / "sweep-server.log"
RESULTS_DIR = CONFIG_DIR / "sweep-results"

# Benchmark prompt — short enough to fit in context, long enough to measure
BENCH_PROMPT = "The theory of relativity was developed by Albert Einstein in 1905"

# Default sweep space
DEFAULT_PIN_VALUES = [0, 4, 8, 12, 16]
DEFAULT_MTP_VALUES = [0, 2, 3, 4, 5, 6]
DEFAULT_UBATCH = 128
DEFAULT_CTX = 65536
DEFAULT_TEMP = 0.0
BENCH_N_TOKENS = 64
WARMUP_ROUNDS = 1
MEASURE_ROUNDS = 3
SERVER_START_TIMEOUT = 60  # seconds
SERVER_READY_POLL = 0.3    # seconds between polls
MEASURE_TIMEOUT = 120       # seconds per benchmark round


@dataclass
class SweepConfig:
    pin: int = 0
    mtp: int = 0
    ubatch: int = DEFAULT_UBATCH
    ctx: int = DEFAULT_CTX
    ts: str = ""


@dataclass
class BenchResult:
    config: SweepConfig = field(default_factory=SweepConfig)
    tokens_per_sec: float = 0.0
    error: str = ""


def parse_args():
    """Parse CLI arguments for sweep space configuration."""
    parser = argparse.ArgumentParser(description="VITRIOL sweep controller")
    parser.add_argument("--model", "-m", required=True, help="Path to GGUF model")
    parser.add_argument("--pin", type=int, nargs="+", default=DEFAULT_PIN_VALUES,
                        help=f"Pin values to sweep (default: {' '.join(map(str, DEFAULT_PIN_VALUES))})")
    parser.add_argument("--mtp", type=int, nargs="+", default=DEFAULT_MTP_VALUES,
                        help=f"MTP values to sweep (default: {' '.join(map(str, DEFAULT_MTP_VALUES))})")
    parser.add_argument("--ubatch", type=int, default=DEFAULT_UBATCH, help="Ubatch size")
    parser.add_argument("--ctx", type=int, default=DEFAULT_CTX, help="Context size")
    parser.add_argument("--quick", action="store_true", help="Only test calibration optimal + neighbors")
    parser.add_argument("--devices", type=int, nargs="*", default=None,
                        help="CUDA device indices to use (multi-GPU layer split, e.g. '0 1'). "
                             "Omit to use whatever CUDA_VISIBLE_DEVICES the environment allows.")
    parser.add_argument("--ts", type=str, nargs="*", default=[],
                        help="Tensor split values to sweep (e.g. '26,10' '22,14'). Empty = auto.")
    parser.add_argument("--output", "-o", default=None, help="Output CSV path")
    args = parser.parse_args()
    global _sweep_devices
    _sweep_devices = args.devices
    return args


_sweep_devices = None


def stop_server():
    """Stop any running llama-server from PID files."""
    for pid_file in ["server.pid", "llama-server.pid"]:
        pf = CONFIG_DIR / pid_file
        if pf.exists():
            pid = int(pf.read_text().strip())
            try:
                os.kill(pid, signal.SIGTERM)
                print(f"  Stopped server PID {pid}")
            except ProcessLookupError:
                pass
            pf.unlink(missing_ok=True)
    # Also kill by process name as fallback
    subprocess.run(["pkill", "-f", "llama-server"], capture_output=True)
    time.sleep(2)


def wait_for_server(url: str, timeout: int = SERVER_START_TIMEOUT) -> bool:
    """Wait until the server is fully loaded (not just listening)."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            # First check /health responds at all
            resp = urllib.request.urlopen(f"{url}/health", timeout=2)
            if resp.status == 200:
                # Now check that model is actually loaded via /completion pre-check
                body = json.dumps({"prompt": "test", "n_predict": 1}).encode()
                preq = urllib.request.Request(
                    f"{url}/completion", data=body,
                    headers={"Content-Type": "application/json"},
                    method="POST",
                )
                try:
                    presp = urllib.request.urlopen(preq, timeout=5)
                    if presp.status == 200:
                        data = json.loads(presp.read())
                        if "content" in data:
                            return True
                except urllib.error.HTTPError as e:
                    if e.code == 503:
                        time.sleep(2)
                        continue
                    raise
        except (urllib.error.URLError, ConnectionRefusedError, OSError):
            pass
        time.sleep(SERVER_READY_POLL)
    return False


def start_server(model_path: str, cfg: SweepConfig, port: int = SWEEP_PORT) -> subprocess.Popen | None:
    """Start llama-server with the given config. Returns Popen or None."""
    model_path = str(Path(model_path).resolve())
    host = "127.0.0.1"

    env = os.environ.copy()
    if _sweep_devices is not None and len(_sweep_devices) > 0:
        env["CUDA_VISIBLE_DEVICES"] = ",".join(str(d) for d in _sweep_devices)
    env.update({
        "VITRIOL_MODE": "stream",
        "VITRIOL_PIN_FIRST_N_LAYERS": str(cfg.pin),
        "VITRIOL_LRU_MB": "0",
        "VITRIOL_KV_QUANT": "tq3_0",
        "VITRIOL_ENGINE_MODE": "vitriol-dma",
        "VITRIOL_MODEL_PATH": model_path,
        "LD_LIBRARY_PATH": f"{LLAMA_DIR / 'build' / 'bin'}:{env.get('LD_LIBRARY_PATH', '')}",
    })

    kv_cache_args = ["--cache-type-k", "tq3_0", "--cache-type-v", "tq3_0"]
    spec_args = []
    if cfg.mtp > 0:
        # Use the model itself as draft for MTP
        spec_args = ["--spec-draft-n-max", str(cfg.mtp), "--model", model_path]

    cmd = [
        str(SERVER_BINARY),
        "-m", model_path,
        "-ngl", "99",
        "-c", str(cfg.ctx),
        "--host", host,
        "--port", str(port),
        "--parallel", "1",
        "--no-mmap",
        "-t", "4",
        *(["-sm", "layer"] if _sweep_devices is not None and len(_sweep_devices) > 1 else []),
        *(["-ts", cfg.ts] if cfg.ts else []),
        *kv_cache_args,
        *spec_args,
        "-fa", "on",
        "--ubatch-size", str(cfg.ubatch),
        "--kv-unified",
        "--cache-idle-slots",
    ]

    SERVER_LOG.parent.mkdir(parents=True, exist_ok=True)
    log_f = open(SERVER_LOG, "w")
    print(f"  Starting server: pin={cfg.pin} mtp={cfg.mtp} ubatch={cfg.ubatch} ctx={cfg.ctx} ts='{cfg.ts}'")
    try:
        proc = subprocess.Popen(cmd, env=env, stdout=log_f, stderr=subprocess.STDOUT,
                                preexec_fn=os.setsid)
    except FileNotFoundError:
        print(f"  ERROR: Server binary not found at {SERVER_BINARY}")
        return None

    url = f"http://{host}:{port}"
    if wait_for_server(url):
        print(f"  Server ready on {url}")
        return proc
    else:
        print(f"  ERROR: Server did not start within {SERVER_START_TIMEOUT}s")
        proc.kill()
        return None


def benchmark_server(url: str, prompt: str, n_tokens: int = BENCH_N_TOKENS,
                     rounds: int = MEASURE_ROUNDS, warmup: int = WARMUP_ROUNDS) -> BenchResult:
    """Run benchmark rounds against the server. Returns averaged result."""
    result = BenchResult()

    # Warmup rounds (not measured)
    for _ in range(warmup):
        _send_completion(url, prompt, n_tokens)

    # Measured rounds
    speeds = []

    for r in range(rounds):
        try:
            data = _send_completion(url, prompt, n_tokens)
            if data and "tokens_predicted" in data and "timings" in data:
                tg = data["tokens_predicted"]
                if tg > 0:
                    # Use predicted_n / predicted_ms for tokens/sec
                    timings = data["timings"]
                    predicted_ms = timings.get("predicted_ms", 0)
                    if predicted_ms > 0:
                        tps = tg / (predicted_ms / 1000.0)
                        speeds.append(tps)

        except Exception as e:
            print(f"  Round {r + 1} error: {e}")

    if not speeds:
        result.error = "No valid measurements"
        return result

    # Average (exclude best and worst for stability)
    if len(speeds) >= 3:
        speeds.sort()
        speeds = speeds[1:-1]

    result.tokens_per_sec = sum(speeds) / len(speeds)
    return result


def _send_completion(url: str, prompt: str, n_tokens: int) -> dict:
    """Send a single completion request. Returns parsed JSON response."""
    body = json.dumps({
        "prompt": prompt,
        "n_predict": n_tokens,
        "temperature": 0.0,
        "cache_prompt": False,
    }).encode()
    req = urllib.request.Request(
        f"{url}/completion",
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    resp = urllib.request.urlopen(req, timeout=MEASURE_TIMEOUT)
    return json.loads(resp.read())


def build_sweep_space(args) -> list[SweepConfig]:
    """Build the list of configs to sweep."""
    configs = []
    ts_vals = args.ts if args.ts else [""]
    for pin in args.pin:
        for mtp in args.mtp:
            for ts in ts_vals:
                configs.append(SweepConfig(pin=pin, mtp=mtp, ubatch=args.ubatch, ctx=args.ctx, ts=ts))
    return configs


def main():
    args = parse_args()
    model_path = args.model

    if not Path(model_path).exists():
        print(f"ERROR: Model not found: {model_path}")
        sys.exit(1)
    if not SERVER_BINARY.exists():
        print(f"ERROR: Server binary not found: {SERVER_BINARY}")
        sys.exit(1)

    # Build sweep space
    configs = build_sweep_space(args)
    if args.quick:
        # Pin around calibration optimal (16), MTP around 5
        configs = [c for c in configs if c.pin in (12, 14, 16, 18) and c.mtp in (3, 4, 5, 6)]
    print(f"Sweep space: {len(configs)} configs")
    print(f"  pins={sorted(set(c.pin for c in configs))} mtps={sorted(set(c.mtp for c in configs))} ts={sorted(set(c.ts for c in configs))}")

    if not SKIP_STOP:
        print("Stopping any running server (heads will need relaunch)...")
        stop_server()
    else:
        print("REBIS_SWEEP_SKIP_STOP set — leaving running servers alone")

    results = []
    total = len(configs)

    for idx, cfg in enumerate(configs):
        print(f"\n[{idx + 1}/{total}] Testing pin={cfg.pin} mtp={cfg.mtp} ts='{cfg.ts}'...")

        # Start server with this config
        proc = start_server(model_path, cfg)
        if proc is None:
            results.append(BenchResult(config=cfg, error="Server start failed"))
            continue

        try:
            # Benchmark
            url = f"http://127.0.0.1:{SWEEP_PORT}"
            result = benchmark_server(url, BENCH_PROMPT)
            result.config = cfg

            if result.error:
                print(f"  ERROR: {result.error}")
            else:
                print(f"  OK: {result.tokens_per_sec:.2f} t/s")

            results.append(result)

        finally:
            # Stop server
            proc.terminate()
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()
            time.sleep(1)

    # Print summary
    print("\n" + "=" * 60)
    print("SWEEP RESULTS")
    print("=" * 60)
    print(f"{'pin':>4} {'mtp':>3} {'t/s':>8}  {'error'}")
    print("-" * 60)

    best = None
    for r in results:
        if r.error:
            print(f"{r.config.pin:>4} {r.config.mtp:>3} {'ERR':>8}  {r.error}")
        else:
            print(f"{r.config.pin:>4} {r.config.mtp:>3} {r.tokens_per_sec:>7.2f}")
            if best is None or r.tokens_per_sec > best.tokens_per_sec:
                best = r

    if best:
        print("-" * 60)
        print(f"BEST: pin={best.config.pin} mtp={best.config.mtp} "
              f"ubatch={best.config.ubatch} ctx={best.config.ctx}")
        print(f"  {best.tokens_per_sec:.2f} t/s")

    return 0


if __name__ == "__main__":
    sys.exit(main())
