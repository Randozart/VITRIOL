"""
VITRIOL Emulated Memory — Retrieval with Cascading (Spreading Activation)

Multi-hop retrieval: direct search → edge traversal → score → rank.
"""

import re
from typing import Optional

from . import db
from .scorer import keyword_overlap, recency_score, compute_score


# Default scoring weights — overridable via env or config
DEFAULT_TOP_K = int(os.environ.get('MEMORY_TOP_K', '5'))
DEFAULT_CASCADE_DEPTH = int(os.environ.get('MEMORY_CASCADE_DEPTH', '1'))
DEFAULT_RELEVANCE_WEIGHT = float(os.environ.get('MEMORY_RELEVANCE_WEIGHT', '0.40'))
DEFAULT_RECENCY_WEIGHT = float(os.environ.get('MEMORY_RECENCY_WEIGHT', '0.35'))
DEFAULT_HEBBIAN_WEIGHT = float(os.environ.get('MEMORY_HEBBIAN_WEIGHT', '0.15'))
DEFAULT_STRENGTH_WEIGHT = float(os.environ.get('MEMORY_STRENGTH_WEIGHT', '0.10'))

import os


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
    cascade_depth: int = DEFAULT_CASCADE_DEPTH
) -> list[dict]:
    """
    Main retrieval pipeline.

    1. Hop 1: Direct search over episodes and knowledge nodes
    2. Hop 2+: Edge traversal (spreading activation)
    3. Score and rank
    """
    candidates = []

    # ── Hop 1: Direct Retrieval ──
    episodes = db.search_episodes(project_id, query, limit=top_k * 3)
    for ep in episodes:
        ep['_type'] = 'episode'
        ep['_content'] = ep.get('content', '')
        ep['_source'] = 'hop1_direct'
    _merge_candidates(candidates, episodes)

    nodes = db.search_nodes(project_id, query, limit=top_k * 2)
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
        scored.append(candidate)

    scored.sort(key=lambda c: c['_score'], reverse=True)
    return scored[:top_k]
