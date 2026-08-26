#!/usr/bin/env python3
"""Pymander → MongoDB vector mirror (hash-gated upserts).

Canonical source stays markdown: every `## Heading` in a domain file is one
atomic node (label = heading, body = prose under it). This script mirrors
nodes into mongod (vitriol_pymander.nodes) with local embeddings so
vitriol_rag can do cross-domain semantic recall. Markdown is never written
by this script; Mongo is a derived index.

Hash-gating: sha256(domain|label|body). Unchanged nodes are skipped entirely
(no re-embed, no write). Nodes deleted from markdown are purged from Mongo.

Usage:
  pymander_sync.py [--dry-run] [--full] [--sources DIR]...

Env:
  MONGO_URI            default mongodb://127.0.0.1:27018
  PYMANDER_SOURCES     extra colon-separated dirs to scan
  FASTEMBED_MODEL      default BAAI/bge-small-en-v1.5
"""
import argparse
import hashlib
import os
import re
import sys
import time

MONGO_URI = os.environ.get("MONGO_URI", "mongodb://127.0.0.1:27018")
MODEL_NAME = os.environ.get("FASTEMBED_MODEL", "BAAI/bge-small-en-v1.5")
DEFAULT_SOURCES = [
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "docs", "pymander"),
]
HEADING_RE = re.compile(r"^##\s+(.+?)\s*$", re.M)


def iter_sources(extra):
    seen = set()
    dirs = []
    for d in DEFAULT_SOURCES + [os.path.expanduser(p) for p in extra]:
        d = os.path.abspath(d)
        if d not in seen and os.path.isdir(d):
            seen.add(d)
            dirs.append(d)
    return dirs


def parse_domain_file(path):
    """Yield (label, body) for each '## ' section. Text before the first
    heading is domain header prose and skipped."""
    with open(path, encoding="utf-8") as f:
        text = f.read()
    matches = list(HEADING_RE.finditer(text))
    for i, m in enumerate(matches):
        start = m.end()
        end = matches[i + 1].start() if i + 1 < len(matches) else len(text)
        body = text[start:end].strip()
        if body:
            yield m.group(1).strip(), body


def collect_nodes(source_dirs):
    """[(domain, label, body)] from every *.md in the source dirs."""
    out = []
    for d in source_dirs:
        for fn in sorted(os.listdir(d)):
            if not fn.endswith(".md"):
                continue
            domain = fn[:-3]
            for label, body in parse_domain_file(os.path.join(d, fn)):
                out.append((domain, label, body))
    return out


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--full", action="store_true", help="re-embed everything")
    ap.add_argument("--sources", action="append", default=[])
    args = ap.parse_args(argv)

    from pymongo import MongoClient

    nodes = collect_nodes(iter_sources(args.sources))
    print(f"[sync] {len(nodes)} nodes parsed from markdown")

    client = MongoClient(MONGO_URI, serverSelectionTimeoutMS=4000)
    col = client["vitriol_pymander"]["nodes"]

    existing = {}
    for doc in col.find({}, {"domain": 1, "label": 1, "body_hash": 1}):
        existing[(doc["domain"], doc["label"])] = doc.get("body_hash", "")

    todo, unchanged = [], 0
    for domain, label, body in nodes:
        h = hashlib.sha256(f"{domain}|{label}|{body}".encode()).hexdigest()
        if not args.full and existing.get((domain, label)) == h:
            unchanged += 1
            continue
        todo.append((domain, label, body, h))

    stale = [k for k in existing if k not in {(d, l) for d, l, _ in nodes}]
    print(f"[sync] unchanged={unchanged} to_embed={len(todo)} stale={len(stale)}")
    if args.dry_run:
        for domain, label, _, _ in todo:
            print(f"  would embed: [{domain}] {label}")
        return 0
    if not todo and not stale:
        print("[sync] nothing to do")
        return 0

    if todo:
        from fastembed import TextEmbedding
        t0 = time.time()
        model = TextEmbedding(model_name=MODEL_NAME)
        print(f"[sync] model {MODEL_NAME} loaded in {time.time()-t0:.1f}s")
        # embed label+body: label carries intent, body carries substance
        texts = [f"{label}\n{body}" for _, label, body, _ in todo]
        vecs = list(model.embed(texts, batch_size=16))
        now = time.time()
        for (domain, label, body, h), vec in zip(todo, vecs):
            col.update_one(
                {"domain": domain, "label": label},
                {"$set": {
                    "summary": body[:280],
                    "body_hash": h,
                    "embedding": [float(x) for x in vec],
                    "dim": len(vec),
                    "updated_at": now,
                }},
                upsert=True,
            )
        print(f"[sync] upserted {len(todo)} embedded nodes")
    if stale:
        n = sum(col.delete_one({"domain": d, "label": l}).deleted_count
                for d, l in stale)
        print(f"[sync] purged {n} stale nodes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
