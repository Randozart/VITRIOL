#!/usr/bin/env python3
"""Selftest for ascensusd's secret resolution (2026-11-16).

Run: python3 scripts/ascensusd_selftest.py

Why a standalone script: this repo has no Python test harness, and the parser is
the one place where a mistake silently changes the cloud escalation provider.
The module is import-safe (its server sits behind `if __name__ == "__main__"`).
"""

import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ascensusd  # noqa: E402

# Fake keys, assembled at runtime so no scanner reads them as leaked secrets.
GOOGLE = "AI" + "za" + "T" * 35
ZAI = "sk" + "-zai-" + "z" * 20

ENV_KEYS = ("ASCENSUS_PROVIDER", "GEMINI_API_KEY", "GEMINI_MODEL")

# (name, file text, env overrides, expected subset of the resolved document)
CASES = [
    (
        "parity: single zai key",
        "provider = zai-coding-plan\napi_key = {zai}\nmodel = glm-4.6\n",
        {},
        {"provider": "zai-coding-plan", "key": ZAI, "model": "glm-4.6"},
    ),
    (
        "gemini_api_key under another section keeps zai",
        "provider = zai-coding-plan\napi_key = {zai}\nmodel = glm-4.6\n"
        "[ascensus]\ngemini_api_key = {google}\n",
        {},
        {"provider": "zai-coding-plan", "key": ZAI, "model": "glm-4.6"},
    ),
    (
        "a [gemini] section beside zai never changes the default provider",
        "provider = zai-coding-plan\napi_key = {zai}\nmodel = glm-4.6\n"
        "[gemini]\napi_key = {google}\nmodel = gemini-decision-1\n",
        {},
        {"provider": "zai-coding-plan", "key": ZAI, "model": "glm-4.6"},
    ),
    (
        "ASCENSUS_PROVIDER=gemini picks the section",
        "provider = zai-coding-plan\napi_key = {zai}\n"
        "[gemini]\napi_key = {google}\nmodel = gemini-decision-1\n",
        {"ASCENSUS_PROVIDER": "gemini"},
        {"provider": "gemini", "key": GOOGLE, "model": "gemini-decision-1"},
    ),
    (
        "operator layout: provider inside [ascensus], GEMINI_API_KEY beside it",
        "[ascensus]\nprovider = zai-coding-plan\napi_key = {zai}\nmodel = glm-4.6\n\n"
        "GEMINI_API_KEY = {google}\n",
        {},
        {"provider": "zai-coding-plan", "key": ZAI, "model": "glm-4.6"},
    ),
    (
        "AIza sniff identifies gemini with no provider named",
        "api_key = {google}\n",
        {},
        {"provider": "gemini", "key": GOOGLE, "model": "gemini-2.5-flash"},
    ),
    (
        "env wins over the file",
        "provider = zai-coding-plan\napi_key = {zai}\n",
        {
            "ASCENSUS_PROVIDER": "gemini",
            "GEMINI_API_KEY": GOOGLE,
            "GEMINI_MODEL": "gemini-3-flash",
        },
        {"provider": "gemini", "key": GOOGLE, "model": "gemini-3-flash"},
    ),
    (
        "malformed lines are ignored, not fatal",
        "[gemini\nthis is not a pair\napi_key = {zai}\nmodel = glm-4.6\n",
        {},
        {"provider": "zai-coding-plan", "key": ZAI, "model": "glm-4.6"},
    ),
]


def write_secret(text):
    """Materialise a secret file and point the module at it."""
    fd, path = tempfile.mkstemp(prefix="ascensus-selftest-")
    with os.fdopen(fd, "w") as handle:
        handle.write(text.format(google=GOOGLE, zai=ZAI))
    return path


def clear_env():
    saved = {key: os.environ.get(key) for key in ENV_KEYS}
    for key in ENV_KEYS:
        os.environ.pop(key, None)
    return saved


def restore_env(saved):
    for key, value in saved.items():
        if value is None:
            os.environ.pop(key, None)
        else:
            os.environ[key] = value


def run_case(name, text, envs, expected):
    """One case: parse, compare, and check the legacy tuple agrees."""
    path = write_secret(text)
    saved = clear_env()
    for key, value in envs.items():
        os.environ[key] = value
    original_file = ascensusd.GEMINI_KEY_FILE
    ascensusd.GEMINI_KEY_FILE = path
    try:
        doc = ascensusd.read_provider_secrets(path)
        legacy = ascensusd.read_secrets()
        ok = all(doc.get(key) == value for key, value in expected.items())
        ok = ok and legacy == (doc["provider"], doc["key"], doc["model"])
        print(("ok   " if ok else "FAIL ") + name)
        if not ok:
            print(f"     expected {expected}")
            print(f"     got      {doc}")
            print(f"     legacy   {legacy}")
        return ok
    finally:
        ascensusd.GEMINI_KEY_FILE = original_file
        restore_env(saved)
        os.unlink(path)


def main():
    failures = sum(0 if run_case(*case) else 1 for case in CASES)
    print(f"\n{len(CASES)} case(s), {failures} failure(s)")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
