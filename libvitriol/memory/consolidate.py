"""
VITRIOL Emulated Memory — Consolidation ("Sleep")

Background thread that summarizes batches of raw episodes into dense
knowledge nodes, mimicking human sleep consolidation.
"""

import threading
import time
import logging
from typing import Optional
from datetime import datetime, timezone

from . import db

logger = logging.getLogger(__name__)

# Consolidation config
CONSOLIDATE_EVERY = int(os.environ.get('MEMORY_CONSOLIDATE_EVERY', '50'))
IDLE_SECONDS = int(os.environ.get('MEMORY_IDLE_SECONDS', '60'))
RETENTION_DAYS = int(os.environ.get('MEMORY_RETENTION_DAYS', '30'))
NODE_DECAY = float(os.environ.get('MEMORY_NODE_DECAY', '0.95'))

import os


def get_active_projects() -> list[str]:
    """Discover all project directories in the memory root."""
    memory_dir = os.environ.get('VITRIOL_MEMORY_DIR',
                                os.path.expanduser('~/.vitriol'))
    if not os.path.isdir(memory_dir):
        return []
    return [d for d in os.listdir(memory_dir)
            if os.path.isdir(os.path.join(memory_dir, d))
            and os.path.exists(os.path.join(memory_dir, d, 'memory.db'))]


def _get_unconsolidated_batch(project_id: str, batch_size: int = 50) -> list[dict]:
    """Find episodes not yet linked by a 'consolidated_from' edge."""
    conn = db._get_conn(project_id)
    cursor = conn.execute("""
        SELECT e.* FROM episodes e
        LEFT JOIN edges ed ON ed.to_type = 'episode'
                          AND ed.to_id = e.id
                          AND ed.relation = 'consolidated_from'
        WHERE ed.id IS NULL
        ORDER BY e.id ASC
        LIMIT ?
    """, (batch_size,))
    return [dict(row) for row in cursor.fetchall()]


def _generate_summary(episodes: list[dict]) -> str:
    """
    Generate a summary from a batch of episodes.

    Phase 1: Simple concatenation with labels.
    Phase 2: Feed to a tiny local model (Qwen 0.5B) for actual summarization.
    """
    if not episodes:
        return ""

    # Extract key information
    roles = set()
    topics = set()
    total_chars = 0

    for ep in episodes:
        roles.add(ep.get('role', ''))
        total_chars += len(ep.get('content', ''))
        # Extract first meaningful line as topic hint
        content = ep.get('content', '').strip()
        if content:
            first_line = content.split('\n')[0][:80]
            topics.add(first_line)

    summary_parts = [
        f"Consolidated {len(episodes)} episodes",
        f"Roles: {', '.join(roles)}",
        f"Total length: ~{total_chars} chars",
        f"Topics: {' | '.join(list(topics)[:5])}",
    ]

    # Phase 2 placeholder: This is where we'd call a local model
    # For now, just concatenate the key exchanges
    key_exchanges = []
    for ep in episodes[-5:]:  # Last 5 episodes
        content = ep.get('content', '').strip()
        if content:
            key_exchanges.append(f"[{ep.get('role', 'user')}]: {content[:200]}")

    if key_exchanges:
        summary_parts.append("---")
        summary_parts.extend(key_exchanges)

    return "\n".join(summary_parts)


def consolidate_project(project_id: str):
    """Run one consolidation pass for a project."""
    batch = _get_unconsolidated_batch(project_id, CONSOLIDATE_EVERY)

    if len(batch) < 10:
        logger.debug(f"[{project_id}] Too few unconsolidated episodes ({len(batch)}), skipping")
        return

    summary = _generate_summary(batch)
    if not summary:
        return

    label = f"consolidated_{batch[0]['id']}_{batch[-1]['id']}"

    # Create knowledge node
    conn = db._get_conn(project_id)
    cursor = conn.execute(
        """INSERT OR IGNORE INTO knowledge_nodes
           (label, summary, source_min, source_max, strength)
           VALUES (?, ?, ?, ?, 1.0)""",
        (label, summary, batch[0]['id'], batch[-1]['id'])
    )
    node_id = cursor.lastrowid

    if node_id is None:
        logger.debug(f"[{project_id}] Node already exists for {label}")
        return

    # Create edges from node to each source episode
    for ep in batch:
        db._ensure_edge(conn, 'node', node_id, 'episode', ep['id'], 'consolidated_from')

    conn.commit()
    logger.info(f"[{project_id}] Consolidated {len(batch)} episodes → node '{label}'")

    # Decay old node strengths
    conn.execute(
        """UPDATE knowledge_nodes
           SET strength = MAX(0.3, strength * ?)
           WHERE created_at < datetime('now', '-7 days')""",
        (NODE_DECAY,)
    )
    conn.commit()

    # Prune old unconsolidated episodes
    cutoff = (datetime.now(timezone.utc).isoformat())
    conn.execute(
        """DELETE FROM episodes
           WHERE created_at < datetime('now', ? || ' days')
           AND id NOT IN (
               SELECT to_id FROM edges WHERE relation = 'consolidated_from'
           )""",
        (str(-RETENTION_DAYS),)
    )
    conn.commit()
    logger.info(f"[{project_id}] Pruned old episodes")


class ConsolidationThread(threading.Thread):
    """Background thread that runs consolidation when the system is idle."""

    def __init__(self):
        super().__init__(daemon=True)
        self._stop_event = threading.Event()
        self.last_request_time = time.time()

    def mark_active(self):
        """Called when a request is received — resets idle timer."""
        self.last_request_time = time.time()

    def run(self):
        while not self._stop_event.is_set():
            time.sleep(30)  # Check every 30 seconds

            idle_time = time.time() - self.last_request_time
            if idle_time < IDLE_SECONDS:
                continue  # Not idle yet

            projects = get_active_projects()
            if not projects:
                continue

            for project_id in projects:
                if self._stop_event.is_set():
                    return
                try:
                    consolidate_project(project_id)
                except Exception as e:
                    logger.error(f"[{project_id}] Consolidation failed: {e}")

    def stop(self):
        self._stop_event.set()


# Singleton
_consolidation_thread: Optional[ConsolidationThread] = None


def start_consolidation():
    """Start the background consolidation thread."""
    global _consolidation_thread
    if _consolidation_thread is None or not _consolidation_thread.is_alive():
        _consolidation_thread = ConsolidationThread()
        _consolidation_thread.start()
        logger.info("Memory consolidation thread started")


def stop_consolidation():
    """Stop the background consolidation thread."""
    global _consolidation_thread
    if _consolidation_thread and _consolidation_thread.is_alive():
        _consolidation_thread.stop()
        _consolidation_thread = None
        logger.info("Memory consolidation thread stopped")
