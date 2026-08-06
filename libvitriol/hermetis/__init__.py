"""
VITRIOL Emulated Memory — Package Init
"""

from . import db
from . import scorer
from .retrieval import retrieve, classify_intent
from .compact import compact_context, estimate_tokens
from .hebbian import update_weights
from .consolidate import (
    consolidate_project,
    start_consolidation,
    stop_consolidation,
    ConsolidationThread,
)

__all__ = [
    'db',
    'scorer',
    'retrieve',
    'classify_intent',
    'compact_context',
    'estimate_tokens',
    'update_weights',
    'consolidate_project',
    'start_consolidation',
    'stop_consolidation',
    'ConsolidationThread',
]
