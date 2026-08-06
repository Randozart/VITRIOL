"""
VITRIOL Emulated Memory — Retrieval with Cascading (Spreading Activation)

Multi-hop retrieval: direct search → edge traversal → score → rank.
When VITRIOL_SEMANTIC_MODE=on, relevance scoring uses cosine similarity
via sentence-transformers instead of keyword overlap.
"""

import math
import os
import re
from typing import Optional

from . import compact, db
from .scorer import keyword_overlap, recency_score, compute_score, estimate_tokens


# Default scoring weights — overridable via env or config
DEFAULT_TOP_K = int(os.environ.get('MEMORY_TOP_K', '5'))
DEFAULT_CASCADE_DEPTH = int(os.environ.get('MEMORY_CASCADE_DEPTH', '1'))
DEFAULT_RELEVANCE_WEIGHT = float(os.environ.get('MEMORY_RELEVANCE_WEIGHT', '0.40'))
DEFAULT_RECENCY_WEIGHT = float(os.environ.get('MEMORY_RECENCY_WEIGHT', '0.35'))
DEFAULT_HEBBIAN_WEIGHT = float(os.environ.get('MEMORY_HEBBIAN_WEIGHT', '0.15'))
DEFAULT_STRENGTH_WEIGHT = float(os.environ.get('MEMORY_STRENGTH_WEIGHT', '0.10'))

# In semantic mode, fetch more candidates for full ranking
_SEMANTIC_MODE = os.environ.get('VITRIOL_SEMANTIC_MODE', 'off').lower() == 'on'
_CANDIDATE_MULTIPLIER = 20 if _SEMANTIC_MODE else 10


def classify_intent(query: str) -> str:
    """Simple keyword-based intent classification."""
    debug_words = {'fix', 'bug', 'error', 'crash', 'broken', 'issue', 'fault', 'null'}
    question_words = {'how', 'what', 'why', 'when', 'where', 'explain', 'meaning', 'purpose'}
    create_words = {'add', 'implement', 'create', 'refactor', 'write', 'build', 'new'}

    q_lower = query.lower()
    q_words = set(re.findall(r'\w+', q_lower))

    if q_words & debug_words:
        return 'code_debug'
    if q_words & question_words:
        return 'question'
    if q_words & create_words:
        return 'code_write'
    return 'general'


def _merge_candidates(existing: list[dict], new: list[dict]) -> list[dict]:
    """Merge new candidates into existing list, de-duplicating by (type, id)."""
    seen = {(c.get('_type', 'episode'), c.get('id')) for c in existing}
    for candidate in new:
        key = (candidate.get('_type', 'episode'), candidate.get('id'))
        if key not in seen:
            existing.append(candidate)
            seen.add(key)
    return existing


def retrieve(
    project_id: str,
    query: str,
    top_k: int = DEFAULT_TOP_K,
    cascade_depth: int = DEFAULT_CASCADE_DEPTH,
    include_history: bool = False
) -> list[dict]:
    """
    Main retrieval pipeline.

    1. Hop 1: Direct search over episodes and knowledge nodes
    2. Hop 2+: Edge traversal (spreading activation)
    3. Score and rank
    include_history (2026-08-06 P3.2): include superseded node versions (default:
    current versions only).
    """
    candidates = []

    # ── Hop 1: Direct Retrieval ──
    # Use larger candidate pool in semantic mode for full ranking
    episodes = db.search_episodes(project_id, query, limit=top_k * _CANDIDATE_MULTIPLIER)
    for ep in episodes:
        ep['_type'] = 'episode'
        ep['_content'] = ep.get('content', '')
        ep['_source'] = 'hop1_direct'
    _merge_candidates(candidates, episodes)

    # In semantic mode, fetch all nodes (no pre-filtering needed)
    node_limit = top_k * (_CANDIDATE_MULTIPLIER // 3) if _SEMANTIC_MODE else top_k * 2
    nodes = db.search_nodes(project_id, query, limit=node_limit,
                            include_history=include_history)
    for n in nodes:
        n['_type'] = 'node'
        n['_content'] = n.get('summary', '')
        n['_source'] = 'hop1_direct'
    _merge_candidates(candidates, nodes)

    # ── Hop 2+: Cascading ──
    for depth in range(cascade_depth):
        hop_candidates = []
        for candidate in candidates:
            edges = db.get_outgoing_edges(
                project_id, candidate.get('_type', 'episode'), candidate['id']
            )
            for edge in edges:
                targets = db.get_edge_targets(
                    project_id, edge['from_type'], edge['from_id']
                )
                for target in targets:
                    if '_type' not in target:
                        target['_type'] = edge['to_type']
                    if '_content' not in target:
                        if 'content' in target:
                            target['_content'] = target['content']
                        elif 'summary' in target:
                            target['_content'] = target['summary']
                        else:
                            target['_content'] = ''
                    target['_source'] = f'hop{depth + 2}_cascade'
                    target['_edge_weight'] = edge.get('weight', 1.0)
                    target['_edge_relation'] = edge.get('relation', '')
                _merge_candidates(hop_candidates, targets)
        candidates = _merge_candidates(candidates, hop_candidates)

    # ── Score and Rank ──
    scored = []
    for candidate in candidates:
        content = candidate.get('_content', '')
        created_at = candidate.get('created_at')
        hebbian_w = candidate.get('_edge_weight', 0.5)
        strength = candidate.get('strength', 1.0)

        score = compute_score(
            query=query,
            content=content,
            created_at=created_at,
            hebbian_weight=hebbian_w,
            node_strength=strength,
            relevance_weight=DEFAULT_RELEVANCE_WEIGHT,
            recency_weight=DEFAULT_RECENCY_WEIGHT,
            hebbian_coeff=DEFAULT_HEBBIAN_WEIGHT,
            strength_coeff=DEFAULT_STRENGTH_WEIGHT,
        )
        candidate['_score'] = score
        # 2026-08-06 (P3.2): nodes are the current-world source; prefer them over
        # historical episodes when scores tie or are close.
        if candidate.get('_type') == 'node':
            candidate['_score'] = score + 0.05
        scored.append(candidate)

    scored.sort(key=lambda c: c['_score'], reverse=True)
    return scored[:top_k]


def context_block(project_id: str, recent_text: str,
                  budget_tokens: int = 1500, top_k: int = 5,
                  min_score: float = 0.3, session_id: str = None) -> tuple:
    """Build a budget-capped context block for selective injection.

    C (2026-08-06): feeds the Copula rolling-window auto-injection. Uses current node
    versions only (retrieve() defaults to superseded=0). Selective: drops candidates
    below min_score, returns (block, top_score, is_new_topic). is_new_topic is False when
    the query is semantically close to the recent session (a continuation the window
    already carries) — the plugin skips injecting then.
    """
    candidates = retrieve(project_id, recent_text, top_k=top_k, cascade_depth=1)
    candidates = [c for c in candidates if c.get('_score', 0.0) >= min_score]
    top_score = candidates[0]['_score'] if candidates else 0.0
    is_new = _is_new_topic(project_id, session_id, recent_text)
    lines = []
    used = 0
    for c in candidates:
        if c.get('_type') == 'node':
            body = compact.format_node(c)
        else:
            body = compact.format_episode(c)
        toks = estimate_tokens(body) + 1
        if used + toks > budget_tokens and used > 0:
            break
        used += toks
        lines.append(body)
    return '\n\n'.join(lines), top_score, is_new


def _is_new_topic(project_id: str, session_id: str, recent_text: str,
                  recent_n: int = 5, threshold: float = 0.55) -> bool:
    """True if the query is NOT a continuation of the session's recent turns.

    Embeds the query and the last recent_n episodes; low cosine -> new topic.
    Falls back to True (inject-worthy) when embeddings are unavailable.
    """
    if not session_id:
        return True
    from . import embed
    if not embed.is_available():
        return True
    q = embed.encode(recent_text[:1000])
    if q is None:
        return True
    conn = db._get_conn(project_id)
    rows = conn.execute(
        "SELECT content FROM episodes WHERE session_id=? ORDER BY id DESC LIMIT ?",
        (session_id, recent_n)).fetchall()
    if not rows:
        return True
    embs = [embed.encode(r['content'][:1000]) for r in rows]
    embs = [e for e in embs if e]
    if not embs:
        return True
    n, d = len(embs), len(embs[0])
    mean = [sum(embs[i][j] for i in range(n)) / n for j in range(d)]
    qn = math.sqrt(sum(x * x for x in q))
    mn = math.sqrt(sum(x * x for x in mean))
    if qn == 0.0 or mn == 0.0:
        return True
    cos = sum(q[i] * mean[i] for i in range(d)) / (qn * mn)
    return cos < threshold
