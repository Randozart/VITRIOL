"""
VITRIOL Emulated Memory — Scoring Functions

Composite scoring combining keyword relevance (or semantic similarity),
recency, Hebbian weight, and node strength.
All components normalized to [0, 1].
"""

import os
import re
from datetime import datetime, timezone
from typing import Optional

# Semantic search (sentence-transformers) — optional, lazy-loaded
_SEMANTIC_MODE = os.environ.get('VITRIOL_SEMANTIC_MODE', 'off').lower() == 'on'
_sentence_model = None


def estimate_tokens(text: str) -> int:
    """Rough token estimation (4 chars ≈ 1 token for English)."""
    return max(1, len(text) // 4)


def _load_sentence_model():
    global _sentence_model
    if _sentence_model is None:
        try:
            from sentence_transformers import SentenceTransformer
            _sentence_model = SentenceTransformer('all-MiniLM-L6-v2')
        except ImportError:
            pass


def _encode(text: str):
    _load_sentence_model()
    if _sentence_model is None:
        return None
    return _sentence_model.encode(text)


def cosine_similarity(a, b) -> float:
    """Cosine similarity between two vectors. Returns 0 on error."""
    try:
        import numpy as np
        a_np = np.array(a, dtype='float32')
        b_np = np.array(b, dtype='float32')
        norm = np.linalg.norm(a_np) * np.linalg.norm(b_np)
        if norm == 0:
            return 0.0
        return float(np.dot(a_np, b_np) / norm)
    except Exception:
        return 0.0


def keyword_overlap(query: str, content: str) -> float:
    """Jaccard similarity of word sets between query and content."""
    query_words = set(re.findall(r'\w+', query.lower()))
    content_words = set(re.findall(r'\w+', content.lower()))

    if not query_words or not content_words:
        return 0.0

    intersection = query_words & content_words
    union = query_words | content_words
    return len(intersection) / len(union)


def semantic_similarity(query: str, content: str) -> float:
    """
    Cosine similarity via sentence-transformers embeddings.
    Falls back to keyword_overlap if sentence-transformers unavailable.
    """
    if not _SEMANTIC_MODE:
        return keyword_overlap(query, content)
    q_emb = _encode(query)
    c_emb = _encode(content)
    if q_emb is None or c_emb is None:
        return keyword_overlap(query, content)
    return cosine_similarity(q_emb, c_emb)


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
    """
    Composite score: relevance × rel_w + recency × rec_w + hebbian × heb_w + strength × str_w.
    Relevance uses semantic_similarity when VITRIOL_SEMANTIC_MODE=on.
    """
    rel = semantic_similarity(query, content)
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
