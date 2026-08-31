"""Isolate the state dir for all tool tests (no live-store pollution)."""

import os
import tempfile
from pathlib import Path

import pytest

_tmp = Path(tempfile.mkdtemp(prefix="tris-test-state-"))
os.environ["TRIS_STATE_DIR"] = str(_tmp)
os.environ.setdefault("HOME", str(Path.home()))

collect_ignore = []


def pytest_configure(config):
    os.environ["TRIS_STATE_DIR"] = str(_tmp)
