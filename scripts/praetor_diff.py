#!/usr/bin/env python3
"""Diff two `praetor validate --json` runs; report diagnostics NEW to the working
tree that are absent from the committed (OLD) version.

Only genuinely-new diagnostics should gate a commit (AGENTS.md §8: pre-existing
diagnostics in changed files are the project baseline). Matches by
(relative_path, line, source, message).

Usage: praetor_diff.py NEW_JSON OLD_JSON NEW_PREFIX OLD_PREFIX
Exit 1 if any new diagnostics exist.
"""
import json
import sys


def load(path):
    """Load the failures list from a praetor --json output file (empty on error)."""
    try:
        with open(path) as f:
            data = json.load(f)
        return data.get("failures", [])
    except Exception:
        return []


def diag_key(diag, prefix):
    """Normalize a diagnostic to (relpath, line, source, message) for diffing."""
    rel = diag.get("file", "")
    if rel.startswith(prefix):
        rel = rel[len(prefix):]
    return (rel, diag.get("line", 0), diag.get("source", ""), diag.get("message", ""))


def main():
    """Compare runs, print new diagnostics, return 1 if any are new."""
    if len(sys.argv) != 5:
        print("usage: praetor_diff.py NEW_JSON OLD_JSON NEW_PREFIX OLD_PREFIX")
        return 2
    new_path, old_path, new_prefix, old_prefix = sys.argv[1:5]

    old_keys = {diag_key(d, old_prefix) for d in load(old_path)}
    new_diags = [d for d in load(new_path) if diag_key(d, new_prefix) not in old_keys]

    for d in new_diags:
        print("  %s:%d | %s | %s" % (d.get("file", "?"), d.get("line", 0),
                                     d.get("source", ""), d.get("message", "")))
    print("NEW diagnostics: %d" % len(new_diags))
    return 1 if new_diags else 0


if __name__ == "__main__":
    sys.exit(main())
