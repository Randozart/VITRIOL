"""
VITRIOL Emulated Memory — Token-Budgeted Compaction & Injection

Converts retrieved candidate memories into a compact, formatted system message
that fits within a strict token budget.
"""

from typing import Optional
from datetime import datetime

from .scorer import estimate_tokens


DEFAULT_ACTIVE_BUDGET = int(os.environ.get('MEMORY_ACTIVE_BUDGET', '4000'))
DEFAULT_SESSION_KEEP = int(os.environ.get('MEMORY_SESSION_KEEP', '2'))

import os


def format_episode(episode: dict, max_chars: Optional[int] = None) -> str:
    """Format an episode for injection. Truncates if max_chars set."""
    content = episode.get('content', '')
    if max_chars and len(content) > max_chars:
        content = content[:max_chars] + '…'

    created = episode.get('created_at', '')
    if created:
        try:
            dt = datetime.fromisoformat(created)
            created = dt.strftime('%Y-%m-%d')
        except (ValueError, TypeError):
            pass

    role = episode.get('role', 'user')
    session_label = episode.get('session_label', '')
    prefix = f"[{created}]" if created else ""
    if session_label:
        prefix += f" [{session_label}]"

    return f"{prefix} {role}: {content}"


def format_node(node: dict) -> str:
    """Format a knowledge node for injection."""
    label = node.get('label', 'memory')
    summary = node.get('summary', '')
    strength = node.get('strength', 1.0)
    marker = "●" if strength > 0.7 else "○"
    return f"[Consolidated: {label}] ({marker}) {summary}"


def format_symbol_signature(symbol: dict) -> str:
    """Format a code symbol as a compact signature."""
    sig = symbol.get('signature', '')
    doc = symbol.get('doc_comment', '')
    path = symbol.get('file_path', '')

    parts = [f"// {path}"]
    if sig:
        parts.append(sig)
    if doc:
        parts.append(f"// {doc}")
    return "\n".join(parts)


def format_compact(episode: dict) -> str:
    """Ultra-compact format when budget is tight."""
    content = episode.get('content', '')
    # Take first 120 chars
    compact = content[:120].replace('\n', ' ').strip()
    if len(content) > 120:
        compact += '…'
    created = episode.get('created_at', '')[:10] if episode.get('created_at') else ''
    role = episode.get('role', 'user')
    return f"[{created}] {role}: {compact}"


def compact_context(
    candidates: list[dict],
    project_id: str = "",
    session_id: str = "",
    query: str = "",
    recent_episodes: Optional[list[dict]] = None,
    budget: int = DEFAULT_ACTIVE_BUDGET
) -> list[str]:
    """
    Build a list of formatted context strings within the token budget.

    Recent session context is always injected first. The remaining budget
    is filled by scored candidates in rank order.

    Returns a list of strings to be joined with newlines.
    """
    injected = []
    tokens_used = 0

    # ── Header ──
    header = f"[Memory Context — VITRIOL Emulated Memory]"
    if project_id:
        header += f"\nProject: {project_id}"
    if session_id:
        header += f" | Session: {session_id}"
    header += f"\nQuery: {query}\n"

    injected.append(header)
    tokens_used += estimate_tokens(header)

    # ── Recent session context (always included) ──
    if recent_episodes:
        recent_section = "\n## Recent Context\n"
        for ep in recent_episodes:
            recent_section += format_episode(ep) + "\n"
        recent_tokens = estimate_tokens(recent_section)
        if tokens_used + recent_tokens <= budget:
            injected.append(recent_section)
            tokens_used += recent_tokens

    # ── Relevant past episodes and nodes ──
    if candidates:
        past_section = "\n## Relevant Context\n"
        for candidate in candidates:
            if tokens_used >= budget:
                break

            ctype = candidate.get('_type', 'episode')
            content_text = candidate.get('_content', '')

            if ctype == 'episode':
                formatted = format_episode(candidate)
            elif ctype == 'node':
                formatted = format_node(candidate)
            else:
                formatted = format_episode(candidate)

            formatted += f" (score: {candidate.get('_score', 0):.2f})"

            tokens = estimate_tokens(formatted)
            if tokens_used + tokens <= budget:
                past_section += formatted + "\n"
                tokens_used += tokens
            else:
                # Budget tight — use compact format
                compact = format_compact(candidate)
                if tokens_used + estimate_tokens(compact) <= budget:
                    past_section += compact + "\n"
                    tokens_used += estimate_tokens(compact)

        injected.append(past_section)

    # ── Footer ──
    if tokens_used > 0:
        injected.append("[End Memory Context]\n")

    return injected
