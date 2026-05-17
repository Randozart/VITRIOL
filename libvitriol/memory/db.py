"""
VITRIOL Emulated Memory — Database Layer

SQLite-backed persistent memory per project. Each project gets its own
.vitriol/memory.db file, automatically created on first access.
Optionally caches sentence-transformer embeddings for semantic search.
"""

import hashlib
import sqlite3
import os
import threading
from typing import Optional
from datetime import datetime

# Default state root directory
MEMORY_DIR = os.environ.get('VITRIOL_MEMORY_DIR', os.path.expanduser('~/.vitriol'))

# Thread-local DB connections
_local = threading.local()

# Global write mutex — serializes all writes to prevent "database is locked"
_write_lock = threading.Lock()


def _get_db_path(project_id: str) -> str:
    """Get the path to a project's memory database."""
    project_dir = os.path.join(MEMORY_DIR, project_id)
    os.makedirs(project_dir, exist_ok=True)
    return os.path.join(project_dir, 'memory.db')


def _get_conn(project_id: str) -> sqlite3.Connection:
    """Get a thread-local connection for the given project."""
    cache_key = f"{project_id}:{threading.get_ident()}"
    if not hasattr(_local, 'conns'):
        _local.conns = {}
    if cache_key not in _local.conns:
        db_path = _get_db_path(project_id)
        conn = sqlite3.connect(db_path)
        conn.row_factory = sqlite3.Row
        conn.execute("PRAGMA journal_mode=WAL")
        conn.execute("PRAGMA synchronous=NORMAL")
        _local.conns[cache_key] = conn
        _init_db(conn)
    conn = _local.conns[cache_key]
    conn.execute("PRAGMA busy_timeout=10000")
    return conn


def _wal_checkpoint(project_id: str):
    """Trigger a WAL checkpoint to keep the WAL file small."""
    try:
        conn = _get_conn(project_id)
        conn.execute("PRAGMA wal_checkpoint(PASSIVE)")
    except Exception:
        pass


def _init_db(conn: sqlite3.Connection):
    """Create tables if they don't exist."""
    conn.executescript("""
        CREATE TABLE IF NOT EXISTS episodes (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id   TEXT NOT NULL,
            turn_index   INTEGER NOT NULL,
            role         TEXT NOT NULL,
            content      TEXT NOT NULL,
            token_count  INTEGER DEFAULT 0,
            created_at   TEXT DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_episodes_session
            ON episodes(session_id, turn_index);
        CREATE INDEX IF NOT EXISTS idx_episodes_created
            ON episodes(created_at);

        CREATE TABLE IF NOT EXISTS knowledge_nodes (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            label        TEXT NOT NULL UNIQUE,
            summary      TEXT NOT NULL,
            source_min   INTEGER,
            source_max   INTEGER,
            strength     REAL DEFAULT 1.0,
            created_at   TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS edges (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            from_type    TEXT NOT NULL,
            from_id      INTEGER NOT NULL,
            to_type      TEXT NOT NULL,
            to_id        INTEGER NOT NULL,
            relation     TEXT NOT NULL,
            weight       REAL DEFAULT 1.0,
            updated_at   TEXT DEFAULT (datetime('now')),
            UNIQUE(from_type, from_id, to_type, to_id, relation)
        );

        CREATE TABLE IF NOT EXISTS sessions (
            session_id   TEXT PRIMARY KEY,
            label        TEXT,
            turn_count   INTEGER DEFAULT 0,
            created_at   TEXT DEFAULT (datetime('now')),
            updated_at   TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS config (
            key   TEXT PRIMARY KEY,
            value TEXT
        );

        CREATE TABLE IF NOT EXISTS embeddings (
            content_hash TEXT PRIMARY KEY,
            content_type TEXT NOT NULL,
            vector BLOB NOT NULL,
            created_at   TEXT DEFAULT (datetime('now'))
        );
    """)
    conn.commit()


# ── Embedding Cache (for semantic search) ─────────────────────────

_SEMANTIC_MODE = os.environ.get('VITRIOL_SEMANTIC_MODE', 'off').lower() == 'on'


def _content_hash(content: str) -> str:
    """SHA-256 hex digest of content for embedding cache key."""
    return hashlib.sha256(content.encode('utf-8')).hexdigest()


def _get_cached_embedding(conn, content_hash: str) -> Optional[bytes]:
    """Retrieve cached embedding blob by content hash."""
    cursor = conn.execute(
        "SELECT vector FROM embeddings WHERE content_hash = ?",
        (content_hash,)
    )
    row = cursor.fetchone()
    return row['vector'] if row else None


def _store_cached_embedding(conn, content_hash: str,
                            content_type: str, vector: bytes):
    """Store or update an embedding blob."""
    conn.execute(
        "INSERT OR REPLACE INTO embeddings (content_hash, content_type, vector) VALUES (?, ?, ?)",
        (content_hash, content_type, vector)
    )


def _compute_and_cache(conn, content: str, content_type: str = 'episode') -> Optional[bytes]:
    """
    Compute embedding, cache it, return serialised bytes.
    Returns None if semantic mode is off or sentence-transformers unavailable.
    """
    if not _SEMANTIC_MODE:
        return None
    try:
        from .scorer import _encode
        emb = _encode(content)
        if emb is None:
            return None
        import numpy as np
        blob = np.array(emb, dtype='float32').tobytes()
        ch = _content_hash(content)
        _store_cached_embedding(conn, ch, content_type, blob)
        return blob
    except Exception:
        return None


def get_embedding_for_text(content: str) -> Optional[list]:
    """
    Public helper: retrieve or compute an embedding for arbitrary text.
    Returns a list of floats, or None if unavailable.
    """
    if not _SEMANTIC_MODE:
        return None
    from .scorer import _encode
    emb = _encode(content)
    if emb is None:
        return None
    return list(emb)


def get_or_create_session(project_id: str, session_id: str) -> dict:
    """Get or create a session row. Returns session dict."""
    with _write_lock:
        conn = _get_conn(project_id)
        cursor = conn.execute(
            "SELECT * FROM sessions WHERE session_id = ?", (session_id,)
        )
        row = cursor.fetchone()
        if row:
            conn.execute(
                "UPDATE sessions SET updated_at = datetime('now') WHERE session_id = ?",
                (session_id,)
            )
            conn.commit()
            return dict(row)

        conn.execute(
            "INSERT INTO sessions (session_id) VALUES (?)", (session_id,)
        )
        conn.commit()
        return {'session_id': session_id, 'turn_count': 0}


def store_episode(project_id: str, session_id: str, role: str,
                  content: str, token_count: int = 0,
                  turn_index: Optional[int] = None) -> int:
    """Store a conversation turn. Returns the episode ID."""
    with _write_lock:
        conn = _get_conn(project_id)

    if turn_index is None:
        cursor = conn.execute(
            "SELECT COALESCE(MAX(turn_index), -1) + 1 FROM episodes WHERE session_id = ?",
            (session_id,)
        )
        turn_index = cursor.fetchone()[0]

    cursor = conn.execute(
        """INSERT INTO episodes (session_id, turn_index, role, content, token_count)
           VALUES (?, ?, ?, ?, ?)""",
        (session_id, turn_index, role, content, token_count)
    )
    episode_id = cursor.lastrowid

    conn.execute(
        "UPDATE sessions SET turn_count = turn_count + 1, updated_at = datetime('now') WHERE session_id = ?",
        (session_id,)
    )
    conn.commit()

    # Link to previous episode in session
    if turn_index > 0:
        cursor = conn.execute(
            "SELECT id FROM episodes WHERE session_id = ? AND turn_index = ?",
            (session_id, turn_index - 1)
        )
        prev = cursor.fetchone()
        if prev:
            _ensure_edge(conn, 'episode', prev['id'],
                         'episode', episode_id, 'follows')

    return episode_id


def search_episodes(project_id: str, query: str, limit: int = 10) -> list[dict]:
    """Search episodes by keyword overlap (Jaccard scoring in scorer.py).
    Returns all episodes for post-filter scoring."""
    conn = _get_conn(project_id)
    cursor = conn.execute(
        """SELECT e.*, s.label as session_label
           FROM episodes e
           LEFT JOIN sessions s ON s.session_id = e.session_id
           ORDER BY e.id DESC
           LIMIT ?""",
        (limit * 10,)  # fetch more for scoring
    )
    return [dict(row) for row in cursor.fetchall()]


def search_nodes(project_id: str, query: str, limit: int = 5) -> list[dict]:
    """Search knowledge nodes by keyword overlap."""
    conn = _get_conn(project_id)
    cursor = conn.execute(
        "SELECT * FROM knowledge_nodes ORDER BY strength DESC, created_at DESC LIMIT ?",
        (limit * 3,)
    )
    return [dict(row) for row in cursor.fetchall()]


def get_recent_episodes(project_id: str, session_id: str,
                        limit: int = 2) -> list[dict]:
    """Get the most recent N episodes from a session."""
    conn = _get_conn(project_id)
    cursor = conn.execute(
        """SELECT * FROM episodes
           WHERE session_id = ?
           ORDER BY turn_index DESC
           LIMIT ?""",
        (session_id, limit)
    )
    return [dict(row) for row in cursor.fetchall()]


def get_outgoing_edges(project_id: str,
                       from_type: str, from_id: int) -> list[dict]:
    """Get all outgoing edges from a node."""
    conn = _get_conn(project_id)
    cursor = conn.execute(
        "SELECT * FROM edges WHERE from_type = ? AND from_id = ?",
        (from_type, from_id)
    )
    return [dict(row) for row in cursor.fetchall()]


def get_edge_targets(project_id: str,
                     from_type: str, from_id: int) -> list[dict]:
    """Get the target nodes of all outgoing edges from a node."""
    conn = _get_conn(project_id)
    cursor = conn.execute(
        """SELECT e.*, ed.relation, ed.weight as edge_weight
           FROM edges ed
           JOIN episodes e ON ed.to_type = 'episode' AND e.id = ed.to_id
           WHERE ed.from_type = ? AND ed.from_id = ?
           UNION
           SELECT n.*, ed.relation, ed.weight as edge_weight
           FROM edges ed
           JOIN knowledge_nodes n ON ed.to_type = 'node' AND n.id = ed.to_id
           WHERE ed.from_type = ? AND ed.from_id = ?""",
        (from_type, from_id, from_type, from_id)
    )
    return [dict(row) for row in cursor.fetchall()]


def get_or_create_edge(project_id: str,
                       from_type: str, from_id: int,
                       to_type: str, to_id: int,
                       relation: str, weight: float = 1.0) -> dict:
    """Get or create an edge between two nodes."""
    conn = _get_conn(project_id)
    _ensure_edge(conn, from_type, from_id, to_type, to_id, relation, weight)
    cursor = conn.execute(
        "SELECT * FROM edges WHERE from_type=? AND from_id=? AND to_type=? AND to_id=? AND relation=?",
        (from_type, from_id, to_type, to_id, relation)
    )
    return dict(cursor.fetchone())


def _ensure_edge(conn, from_type, from_id, to_type, to_id, relation, weight=1.0):
    """Internal: upsert an edge."""
    conn.execute(
        """INSERT OR IGNORE INTO edges (from_type, from_id, to_type, to_id, relation, weight)
           VALUES (?, ?, ?, ?, ?, ?)""",
        (from_type, from_id, to_type, to_id, relation, weight)
    )


def update_edge_weight(project_id: str, edge_id: int, new_weight: float):
    """Update an edge's weight and timestamp."""
    with _write_lock:
        conn = _get_conn(project_id)
        conn.execute(
            "UPDATE edges SET weight = ?, updated_at = datetime('now') WHERE id = ?",
            (new_weight, edge_id)
        )
        conn.commit()


def get_config(project_id: str, key: str, default: str = '') -> str:
    """Get a config value."""
    conn = _get_conn(project_id)
    cursor = conn.execute(
        "SELECT value FROM config WHERE key = ?", (key,)
    )
    row = cursor.fetchone()
    return row['value'] if row else default


def set_config(project_id: str, key: str, value: str):
    """Set a config value."""
    with _write_lock:
        conn = _get_conn(project_id)
        conn.execute(
            "INSERT OR REPLACE INTO config (key, value) VALUES (?, ?)",
            (key, value)
        )
        conn.commit()
