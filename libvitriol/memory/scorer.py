"""
VITRIOL Emulated Memory — Scoring Functions

Composite scoring combining keyword relevance, recency, Hebbian weight, and node strength.
All components normalized to [0, 1].
"""

import re
from datetime import datetime, timezone
from typing import Optional


def estimate_tokens(text: str) -> int:
    """Rough token estimation (4 chars ≈ 1 token for English)."""
    return max(1, len(text) // 4)


def keyword_overlap(query: str, content: str) -> float:
    """Jaccard similarity of word sets between query and content."""
    query_words = set(re.findall(r'\w+', query.lower()))
    content_words = set(re.findall(r'\w+', content.lower()))

    if not query_words or not content_words:
        return 0.0

    intersection = query_words & content_words
    union = query_words | content_words
    return len(intersection) / len(union)


def recency_score(created_at: Optional[str], max_days: float = 30.0) -> float:
    """Linear recency decay over max_days. Clamped to [0, 1]."""
    if not created_at:
        return 0.5  # neutral for unknown dates

    try:
        created = datetime.fromisoformat(created_at)
        if created.tzinfo is None:
            created = created.replace(tzinfo=timezone.utc)
        now = datetime.now(timezone.utc)
        days_old = (now - created).total_seconds() / 86400.0
        return max(0.0, min(1.0, 1.0 - (days_old / max_days)))
    except (ValueError, TypeError):
        return 0.5


def compute_score(
    query: str,
    content: str,
    created_at: Optional[str] = None,
    hebbian_weight: float = 0.5,
    node_strength: float = 1.0,
    relevance_weight: float = 0.40,
    recency_weight: float = 0.35,
    hebbian_coeff: float = 0.15,
    strength_coeff: float = 0.10
) -> float:
    """Composite score: relevance × rel_w + recency × rec_w + hebbian × heb_w + strength × str_w."""
    rel = keyword_overlap(query, content)
    rec = recency_score(created_at)
    heb = max(0.0, min(1.0, hebbian_weight))  # already normalized
    strn = max(0.0, min(1.0, node_strength))

    score = (
        rel * relevance_weight +
        rec * recency_weight +
        heb * hebbian_coeff +
        strn * strength_coeff
    )
    return score
