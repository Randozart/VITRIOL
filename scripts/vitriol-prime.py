#!/usr/bin/env python3
# 2026-11-18 13:58 — Opt-in client readiness tool; resident engine is never restarted.
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "libvitriol"))
from prompt_prime import main

if __name__ == "__main__":
    raise SystemExit(main())
