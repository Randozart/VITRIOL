#!/usr/bin/env python3
"""Pymander — the reference mind: static, curated, domain-specific knowledge.

Pymander is VITRIOL's *static* knowledge basis — *what we are / how we do a
domain well* — as opposed to Hermetis (episodic memory: *what happened*).
Content is hand-authored **atomic nodes** (small pieces of relevant knowledge
tied to the thing being written). This is the P1 store + ingest path only:
doctrine injection into the window and the `pymander` opencode tool arrive in
P2.

Store: each domain is a distinct Hermetis memory root
`~/.vitriol/pymander/<domain>/memory.db`, reached through the existing
`hermetis.db` machinery (project_id = `pymander/<domain>`), so node
versioning (git_rev supersede), embedding cache, and strength all come free.

Ingest format (markdown): `## Heading` starts an atomic node; the body up to
the next `##` is its summary. `#` titles and prose before the first `##` are
domain metadata and are skipped.

PROVENANCE: user-repo — VITRIOL's own architecture (Hermetis db/retrieval),
re-derived from this project's docs; no third-party source consulted.
"""
import argparse
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from hermetis import db, retrieval

DOMAIN_PREFIX = "pymander/"
DOMAIN_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")


def _selection_path():
    """JSON file mapping project_id -> [active domain names]."""
    return os.path.join(db.MEMORY_DIR, "pymander", "selection.json")


def _load_selection() -> dict:
    """Read selection.json (missing/corrupt -> empty)."""
    try:
        with open(_selection_path(), "r", encoding="utf-8") as fh:
            data = json.load(fh)
            return data if isinstance(data, dict) else {}
    except (FileNotFoundError, json.JSONDecodeError, OSError):
        return {}


def _save_selection(data: dict):
    """Atomically write selection.json."""
    path = _selection_path()
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=True)
    os.replace(tmp, path)


def sanitize_domain(domain: str) -> str:
    """Validate + normalize a domain name.

    Domain becomes a path component under MEMORY_DIR/pymander/, so it must be a
    safe filesystem name. Raises ValueError on anything else.
    """
    if not DOMAIN_RE.match(domain):
        raise ValueError(
            f"invalid domain {domain!r}: use letters/digits/._- , no slashes"
        )
    return domain


def domain_project_id(domain: str) -> str:
    """Hermetis project_id backing a Pymander domain (pymander/<domain>)."""
    return DOMAIN_PREFIX + sanitize_domain(domain)


def list_domains() -> list[str]:
    """Return the installed Pymander domains (dirs holding a memory.db)."""
    root = os.path.join(db.MEMORY_DIR, "pymander")
    if not os.path.isdir(root):
        return []
    out = []
    for name in sorted(os.listdir(root)):
        full = os.path.join(root, name)
        if os.path.isdir(full) and os.path.exists(os.path.join(full, "memory.db")):
            out.append(name)
    return out


def _parse_markdown(md_text: str) -> list[tuple[str, str]]:
    """Split markdown into (label, summary) atomic nodes.

    A `## Heading` opens a node; its body (until the next `##` or end) is the
    summary. Content before the first `##` is treated as domain header and
    skipped. Empty summaries are dropped.
    """
    nodes: list[tuple[str, str]] = []
    label = None
    body: list[str] = []
    for line in md_text.splitlines():
        if line.startswith("## "):
            _flush_node(nodes, label, body)
            label = line[3:].strip()
            body = []
        elif label is not None:
            body.append(line)
    _flush_node(nodes, label, body)
    return nodes


def _flush_node(nodes: list[tuple[str, str]], label, body: list[str]):
    """Append a completed node when it has a label and non-empty summary."""
    if label is None:
        return
    summary = "\n".join(body).strip()
    if summary:
        nodes.append((label, summary))


def _embed_best_effort(project_id: str, summary: str):
    """Cache an embedding for a node summary when the embed server is up.

    Best-effort only: a missing/unreachable embed server must never fail an
    ingest (keyword scoring remains the fallback in retrieval).
    """
    try:
        from hermetis import embed
        if not embed.is_available():
            return
        vec = embed.encode(summary)
        if vec is None:
            return
        import numpy as np
        blob = np.array(vec, dtype="float32").tobytes()
        ch = db._content_hash(summary)
        conn = db._get_conn(project_id)
        db._store_cached_embedding(conn, ch, "pymander_node", blob)
        conn.commit()
    except Exception:
        pass


def ingest_markdown(domain: str, md_text: str, git_rev: str = "") -> dict:
    """Ingest a markdown corpus as atomic nodes for a domain.

    Returns {domain, stored, refreshed, embedded}. Same (label, git_rev) ->
    refresh in place; new git_rev -> supersede (never hard-discard), matching
    db.store_node's versioned semantics.
    """
    domain = sanitize_domain(domain)
    pid = domain_project_id(domain)
    nodes = _parse_markdown(md_text)
    conn = db._get_conn(pid)
    stored = 0
    refreshed = 0
    embedded = 0
    for label, summary in nodes:
        # 2026-08-07: detect refresh by checking the (label, git_rev) row that
        # store_node will update in place vs a fresh insert (new version).
        existing = conn.execute(
            "SELECT 1 FROM knowledge_nodes WHERE label=? AND git_rev=?",
            (label, git_rev)).fetchone()
        db.store_node(pid, label, summary,
                      meta={"git_rev": git_rev, "strength": 1.0})
        if existing:
            refreshed += 1
        else:
            stored += 1
        _embed_best_effort(pid, summary)
        if _has_embedding(pid, summary):
            embedded += 1
    return {"domain": domain, "nodes": len(nodes),
            "stored": stored, "refreshed": refreshed, "embedded": embedded}


def _has_embedding(project_id: str, summary: str) -> bool:
    """True if a cached embedding row exists for the summary."""
    try:
        ch = db._content_hash(summary)
        conn = db._get_conn(project_id)
        return conn.execute(
            "SELECT 1 FROM embeddings WHERE content_hash=?",
            (ch,)).fetchone() is not None
    except Exception:
        return False


def list_nodes(domain: str) -> list[dict]:
    """Return the current (superseded=0) nodes of a domain."""
    pid = domain_project_id(domain)
    return db.search_nodes(pid, "", limit=100000)


def search(domain: str, query: str, top_k: int = 5) -> list[dict]:
    """Retrieve the most relevant nodes of a domain for a query."""
    pid = domain_project_id(domain)
    candidates = retrieval.retrieve(pid, query, top_k=top_k, cascade_depth=0)
    return [c for c in candidates if c.get("_type") == "node"]


def set_selection(project_id: str, domains: list[str]) -> dict:
    """Set the active Pymander domains for a project (persisted)."""
    data = _load_selection()
    clean = [sanitize_domain(d) for d in domains]
    data[project_id] = clean
    _save_selection(data)
    return {"project_id": project_id, "domains": clean}


def get_selection(project_id: str) -> list[str]:
    """Return the active Pymander domains for a project (empty if unset)."""
    return list(_load_selection().get(project_id, []))


def _read_file_or_stdin(path: str) -> str:
    """Read a corpus file, or stdin when path is '-'.

    2026-08-07: '-' allows `cat corpus.md | vitriol pymander ingest dom -`,
    so authoring can pipe from $EDITOR or generated content.
    """
    if path == "-":
        return sys.stdin.read()
    with open(path, "r", encoding="utf-8") as fh:
        return fh.read()


def _cmd_ingest(args) -> int:
    domain = sanitize_domain(args.domain)
    md = _read_file_or_stdin(args.file)
    git_rev = args.rev if args.rev is not None else _repo_rev(args.file)
    res = ingest_markdown(domain, md, git_rev)
    print(json.dumps(res, indent=2, sort_keys=True))
    return 0


def _repo_rev(path: str) -> str:
    """Best-effort git HEAD of the corpus file's repo ('' when not in a repo)."""
    if path == "-" or not os.path.isfile(path):
        return ""
    try:
        import subprocess
        base = os.path.dirname(os.path.abspath(path))
        out = subprocess.run(
            ["git", "-C", base, "rev-parse", "HEAD"],
            capture_output=True, text=True, timeout=5)
        return out.stdout.strip() if out.returncode == 0 else ""
    except Exception:
        return ""


def _cmd_list(_args) -> int:
    domains = list_domains()
    print(json.dumps(domains, indent=2, sort_keys=True))
    return 0


def _cmd_nodes(args) -> int:
    nodes = list_nodes(args.domain)
    print(json.dumps(nodes, indent=2, sort_keys=True, default=str))
    return 0


def _cmd_search(args) -> int:
    res = search(args.domain, args.query, args.top_k)
    print(json.dumps(res, indent=2, sort_keys=True, default=str))
    return 0


def _cmd_select(args) -> int:
    res = set_selection(args.project_id, args.domains)
    print(json.dumps(res, indent=2, sort_keys=True))
    return 0


def _cmd_active(args) -> int:
    print(json.dumps(get_selection(args.project_id), sort_keys=True))
    return 0


def build_doctrine(project_id: str, query: str = "", budget_tokens: int = 3000,
                   top_k: int = 3) -> str:
    """Build a budgeted doctrine block for the project's selected domains.

    Aggregates the top nodes of each selected domain into a labeled, budgeted
    text block intended for window injection (the [Pymander doctrine] context).
    The model's static *how* lives here, selected per project. Falls back to the
    first installed domain when the project has no explicit selection.
    """
    domains = get_selection(project_id) or list_domains()[:1]
    sections = []
    used = 0
    for domain in domains:
        text = _domain_section(domain, query, top_k, budget_tokens - used)
        if not text:
            continue
        text_toks = retrieval.estimate_tokens(text) + 1
        if used + text_toks > budget_tokens and used > 0:
            break
        used += text_toks
        sections.append(text)
    return "\n\n".join(sections)


def _domain_section(domain: str, query: str, top_k: int, budget: int) -> str:
    """One domain's doctrine lines under a per-domain budget ('' when none)."""
    try:
        hits = search(domain, query, top_k=top_k)
    except ValueError:
        return ""
    if not hits:
        return ""
    parts = [f"## {domain}"]
    used = 0
    for hit in hits:
        body = f"- {hit.get('label', '')}: {hit.get('summary', '')}"
        toks = retrieval.estimate_tokens(body) + 1
        if used + toks > budget and used > 0:
            break
        used += toks
        parts.append(body)
    return "\n".join(parts)


def _candidates_path() -> str:
    """JSON file listing curated Ascensus/Hermetis answers worth promoting."""
    return os.path.join(db.MEMORY_DIR, "pymander", "candidates.json")


def add_candidate(domain: str, label: str, summary: str, source: str = "") -> dict:
    """Add a promotion candidate (curated: a user/machine decision to fold in).

    Candidates are NOT auto-merged into the corpus — the user reviews them and
    either re-ingests via `vitriol pymander ingest` or deletes. This is the
    learning loop's second half (Ascensus → Hermetis → candidate → curated
    promotion), implemented per AGENTS cleanroom (quality + license care).
    """
    domain = sanitize_domain(domain)
    path = _candidates_path()
    try:
        with open(path, "r", encoding="utf-8") as fh:
            data = json.load(fh)
        if not isinstance(data, dict):
            data = {}
    except (FileNotFoundError, json.JSONDecodeError, OSError):
        data = {}
    data.setdefault(domain, []).append({
        "label": label, "summary": summary, "source": source})
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=True)
    return {"domain": domain, "candidate": label}


def list_candidates(domain: str = "") -> dict:
    """List promotion candidates (all domains, or one domain)."""
    try:
        with open(_candidates_path(), "r", encoding="utf-8") as fh:
            data = json.load(fh)
    except (FileNotFoundError, json.JSONDecodeError, OSError):
        return {}
    if domain:
        return {domain: data.get(domain, [])}
    return data


def _cmd_promote(args) -> int:
    if args.action == "add":
        res = add_candidate(args.domain, args.label, args.summary, args.source)
    elif args.action == "list":
        res = list_candidates(args.domain)
    else:
        print("usage: promote add <domain> <label> <summary> | promote list [domain]")
        return 2
    print(json.dumps(res, indent=2, sort_keys=True, default=str))
    return 0


def estimate_tokens_doctrine(block: str) -> int:
    """Estimate tokens of a doctrine block (4 chars ≈ 1 token)."""
    return retrieval.estimate_tokens(block)


def _cmd_doctrine(args) -> int:
    block = build_doctrine(args.project_id, args.query, args.budget)
    print(block)
    return 0


def build_parser() -> argparse.ArgumentParser:
    """CLI: vitriol pymander <cmd>. Subcommands mirror the P1 scope."""
    ap = argparse.ArgumentParser(prog="vitriol pymander",
                                 description="Pymander reference-mind store")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p_list = sub.add_parser("list", help="list installed Pymander domains")
    p_list.set_defaults(fn=_cmd_list)

    p_ingest = sub.add_parser(
        "ingest", help="ingest a markdown corpus as atomic nodes")
    p_ingest.add_argument("domain")
    p_ingest.add_argument("file", help="corpus .md, or '-' for stdin")
    p_ingest.add_argument("--rev", default=None,
                          help="git_rev; default: corpus repo HEAD ('' outside git)")
    p_ingest.set_defaults(fn=_cmd_ingest)

    p_nodes = sub.add_parser("nodes", help="list a domain's current nodes")
    p_nodes.add_argument("domain")
    p_nodes.set_defaults(fn=_cmd_nodes)

    p_search = sub.add_parser("search", help="retrieve nodes for a query")
    p_search.add_argument("domain")
    p_search.add_argument("query")
    p_search.add_argument("--top-k", type=int, default=5)
    p_search.set_defaults(fn=_cmd_search)

    p_sel = sub.add_parser(
        "select", help="set the active domains for a project")
    p_sel.add_argument("project_id")
    p_sel.add_argument("domains", nargs="+")
    p_sel.set_defaults(fn=_cmd_select)

    p_active = sub.add_parser(
        "active", help="show the active domains for a project")
    p_active.add_argument("project_id")
    p_active.set_defaults(fn=_cmd_active)

    p_doctrine = sub.add_parser(
        "doctrine", help="build a budgeted doctrine block for a project")
    p_doctrine.add_argument("project_id")
    p_doctrine.add_argument("--query", default="")
    p_doctrine.add_argument("--budget", type=int, default=3000)
    p_doctrine.set_defaults(fn=_cmd_doctrine)

    p_promote = sub.add_parser(
        "promote", help="curate Ascensus/Hermetis answers into candidates")
    p_promote.add_argument("action", choices=["add", "list"])
    p_promote.add_argument("domain", nargs="?")
    p_promote.add_argument("label", nargs="?")
    p_promote.add_argument("summary", nargs="?")
    p_promote.add_argument("--source", default="")
    p_promote.set_defaults(fn=_cmd_promote)
    return ap


def main(argv=None) -> int:
    """CLI entry point (also usable from scripts/vitriol)."""
    args = build_parser().parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
