"""
VITRIOL Emulated Memory — Hebbian Weight Updates

After each LLM response, adjust edge weights based on whether retrieved
memories were actually used. Implements "neurons that fire together, wire together."
"""

import re
from typing import Optional

from . import db


def is_referenced(candidate: dict, response_text: str) -> bool:
    """Check if a candidate memory is referenced in the LLM's response."""
    response_lower = response_text.lower()
    content = candidate.get('_content', '') or candidate.get('content', '') or candidate.get('summary', '')

    # Check if key phrases from the candidate appear in the response
    # Use the first 50 chars as a signature
    signature = content[:50].strip().lower()
    if not signature:
        return False

    # Simple substring match
    words = set(re.findall(r'\w+', signature))
    response_words = set(re.findall(r'\w+', response_lower))
    overlap = words & response_words

    # Heuristic: if 3+ significant words overlap, it's referenced
    significant = {w for w in overlap if len(w) > 3}
    return len(significant) >= 3


def update_weights(project_id: str, response_text: str,
                   retrieved: list[dict]):
    """
    Update Hebbian weights for all retrieved candidates based on
    whether the LLM actually used them in its response.

    - Used candidates: strengthen their outgoing edges (+0.1)
    - Unused candidates: weaken their outgoing edges (-0.05)
    - Co-retrieved pairs that were both used: strengthen (+0.05)
    """
    if not response_text or not retrieved:
        return

    # ── Phase 1: Update each candidate's edges ──
    for candidate in retrieved:
        used = is_referenced(candidate, response_text)
        ctype = candidate.get('_type', 'episode')
        cid = candidate.get('id')

        if ctype is None or cid is None:
            continue

        edges = db.get_outgoing_edges(project_id, ctype, cid)
        for edge in edges:
            if used:
                new_weight = min(3.0, edge['weight'] + 0.1)
            else:
                new_weight = max(0.0, edge['weight'] - 0.05)
            db.update_edge_weight(project_id, edge['id'], new_weight)

    # ── Phase 2: Strengthen co-retrieved pairs ──
    for i, a in enumerate(retrieved):
        for j, b in enumerate(retrieved):
            if i >= j:
                continue

            a_used = is_referenced(a, response_text)
            b_used = is_referenced(b, response_text)

            if a_used and b_used:
                edge = db.get_or_create_edge(
                    project_id,
                    a.get('_type', 'episode'), a.get('id'),
                    b.get('_type', 'episode'), b.get('id'),
                    'co_retrieved',
                    weight=1.0
                )
                new_weight = min(3.0, edge['weight'] + 0.05)
                db.update_edge_weight(project_id, edge['id'], new_weight)
