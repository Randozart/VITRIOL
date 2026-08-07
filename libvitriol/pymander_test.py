#!/usr/bin/env python3
"""Pymander P1 tests — store, ingest, versioning, selection.

Run: python3 -m unittest libvitriol.pymander_test
     (or: vitriol pymander selftest once wired)

Isolated from the real corpus: VITRIOL_MEMORY_DIR is redirected to a temp dir
so no test writes to ~/.vitriol/pymander/.
"""
import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

_TMP = tempfile.mkdtemp(prefix="pymander_test_")
os.environ["VITRIOL_MEMORY_DIR"] = _TMP
os.environ["VITRIOL_SEMANTIC_MODE"] = "off"

import pymander  # noqa: E402


CORPUS = """# Systems Programming Doctrine

Prose before the first node is domain header; it is skipped.

## Memory Safety Rules

Rust borrow checker guarantees at compile time; no data races in safe code.

## Zero-Copy I/O

Prefers buffers owned by the driver; avoids memcpy in the hot path.
"""


class PymanderStoreTests(unittest.TestCase):

    def setUp(self):
        self.domain = "systems"
        # Fresh domain per test: drop cached connections (they point at the
        # previous test's db file) and remove the on-disk domain directory.
        pymander.db._local.conns = {}
        import shutil
        shutil.rmtree(os.path.join(_TMP, "pymander"), ignore_errors=True)

    def test_ingest_creates_atomic_nodes(self):
        res = pymander.ingest_markdown(self.domain, CORPUS)
        self.assertEqual(res["nodes"], 2)
        self.assertEqual(res["stored"], 2)
        nodes = pymander.list_nodes(self.domain)
        labels = {n["label"] for n in nodes}
        self.assertEqual(labels, {"Memory Safety Rules", "Zero-Copy I/O"})
        # The `#` title and pre-node prose were skipped — no "Systems" node.
        self.assertNotIn("Systems Programming Doctrine", labels)

    def test_same_rev_refreshes_in_place(self):
        pymander.ingest_markdown(self.domain, CORPUS, git_rev="abc123")
        pymander.ingest_markdown(self.domain, CORPUS, git_rev="abc123")
        rows = pymander._parse_markdown(CORPUS)
        conn = pymander.db._get_conn(pymander.domain_project_id(self.domain))
        # Same rev -> each label exists once, current, not superseded.
        for label, _ in rows:
            cur = conn.execute(
                "SELECT COUNT(*) FROM knowledge_nodes WHERE label=? AND superseded=0",
                (label,)).fetchone()[0]
            total = conn.execute(
                "SELECT COUNT(*) FROM knowledge_nodes WHERE label=?", (label,)
            ).fetchone()[0]
            self.assertEqual(cur, 1)
            self.assertEqual(total, 1)

    def test_new_rev_supersedes_not_discards(self):
        pymander.ingest_markdown(self.domain, CORPUS, git_rev="rev1")
        pymander.ingest_markdown(self.domain, CORPUS, git_rev="rev2")
        conn = pymander.db._get_conn(pymander.domain_project_id(self.domain))
        rows = conn.execute("SELECT label, git_rev, superseded FROM knowledge_nodes").fetchall()
        revs = {r["label"]: (r["git_rev"], r["superseded"]) for r in rows}
        for label in ("Memory Safety Rules", "Zero-Copy I/O"):
            # Old rev superseded, new rev current — versioned supersede, never discard.
            self.assertEqual(revs[label], ("rev2", 0))
            older = conn.execute(
                "SELECT superseded FROM knowledge_nodes WHERE label=? AND git_rev='rev1'",
                (label,)).fetchone()
            self.assertEqual(older["superseded"], 1)

    def test_domain_isolation(self):
        pymander.ingest_markdown(self.domain, CORPUS)
        pymander.ingest_markdown("systems2", CORPUS)
        self.assertEqual(len(pymander.list_domains()), 2)
        nodes1 = pymander.list_nodes(self.domain)
        nodes2 = pymander.list_nodes("systems2")
        self.assertEqual(len(nodes1), len(nodes2))

    def test_domain_name_sanitization(self):
        for bad in ("../evil", "a/b", "a b", "", "..", "a:b"):
            with self.assertRaises(ValueError):
                pymander.sanitize_domain(bad)
        self.assertEqual(pymander.sanitize_domain("Systems.Rust-1"), "Systems.Rust-1")

    def test_project_id_namespace(self):
        self.assertEqual(pymander.domain_project_id("systems"),
                         "pymander/systems")
        db_path = pymander.db._get_db_path(pymander.domain_project_id("systems"))
        self.assertIn(os.path.join("pymander", "systems", "memory.db"), db_path)

    def test_selection_round_trip(self):
        pymander.set_selection("proj_a", ["systems", "systems2"])
        self.assertEqual(pymander.get_selection("proj_a"), ["systems", "systems2"])
        # Per-project isolation.
        self.assertEqual(pymander.get_selection("proj_b"), [])
        pymander.set_selection("proj_a", ["systems"])
        self.assertEqual(pymander.get_selection("proj_a"), ["systems"])

    def test_search_returns_nodes(self):
        pymander.ingest_markdown(self.domain, CORPUS)
        res = pymander.search(self.domain, "memory safety rust borrow checker")
        self.assertTrue(res)
        self.assertTrue(all(r["_type"] == "node" for r in res))
        top = res[0]
        self.assertEqual(top["label"], "Memory Safety Rules")

    def test_cli_list_json(self):
        pymander.ingest_markdown(self.domain, CORPUS)
        out = pymander._cmd_list(pymander.build_parser().parse_args(["list"]))
        self.assertEqual(out, 0)

    def test_build_doctrine_uses_selection(self):
        pymander.ingest_markdown(self.domain, CORPUS)
        pymander.ingest_markdown("systems2", CORPUS)
        pymander.set_selection("proj_x", ["systems", "systems2"])
        block = pymander.build_doctrine("proj_x", "memory safety")
        self.assertIn("## systems", block)
        self.assertIn("Memory Safety Rules", block)
        self.assertIn("## systems2", block)

    def test_build_doctrine_budget_empty_with_no_selection(self):
        pymander.ingest_markdown(self.domain, CORPUS)
        block = pymander.build_doctrine("unselected_proj", "memory safety")
        # Falls back to the first installed domain, so still non-empty.
        self.assertTrue(block)


if __name__ == "__main__":
    unittest.main()
