//! The Hermetis database layer (P1) — byte-parity port of `hermetis/db.py`.
//!
//! Every DDL string, pragma, query, and row shape mirrors the Python original so
//! the schema can be byte-compared and existing `~/.vitriol/<project>/memory.db`
//! files stay readable. Write functions serialize on a single mutex (the Python
//! `_write_lock`); WAL journaling + `busy_timeout` keep concurrent readers safe.

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Result type used across the module.
pub type Result<T> = std::result::Result<T, rusqlite::Error>;

/// Bundle an edge identity + weight (Python `EdgeSpec`).
#[derive(Debug, Clone)]
pub struct EdgeSpec {
    pub from_type: String,
    pub from_id: i64,
    pub to_type: String,
    pub to_id: i64,
    pub relation: String,
    pub weight: f64,
}

impl EdgeSpec {
    /// Build a spec with the default weight.
    pub fn new(from_type: &str, from_id: i64, to_type: &str, to_id: i64, relation: &str) -> Self {
        Self {
            from_type: from_type.into(),
            from_id,
            to_type: to_type.into(),
            to_id,
            relation: relation.into(),
            weight: 1.0,
        }
    }
}

/// Episode write metadata (Python `meta` dict).
#[derive(Debug, Default, Clone)]
pub struct EpisodeMeta {
    pub token_count: i64,
    pub turn_index: Option<i64>,
}

/// One episode write (bundled to keep `store_episode` under the param gate).
#[derive(Debug, Clone)]
pub struct EpisodeWrite {
    pub role: String,
    pub content: String,
    pub meta: EpisodeMeta,
}

/// Node write metadata (Python `meta` dict).
#[derive(Debug, Default, Clone)]
pub struct NodeMeta {
    pub strength: f64,
    pub source_min: Option<i64>,
    pub source_max: Option<i64>,
    pub git_rev: String,
}

/// The exact DDL from Python `_init_db` — byte-identical so `.schema` diffs clean.
pub const SCHEMA_DDL: &str = r#"
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
            label        TEXT NOT NULL,
            summary      TEXT NOT NULL,
            source_min   INTEGER,
            source_max   INTEGER,
            strength     REAL DEFAULT 1.0,
            git_rev      TEXT DEFAULT '',
            superseded   INTEGER DEFAULT 0,
            superseded_by INTEGER,
            created_at   TEXT DEFAULT (datetime('now')),
            UNIQUE(label, git_rev)
        );
        CREATE INDEX IF NOT EXISTS idx_nodes_label_cur
            ON knowledge_nodes(label, superseded);

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
    "#;

/// The Hermetis memory store over a memory root (default `~/.vitriol`).
pub struct Hermes {
    /// Memory root directory (Python `MEMORY_DIR`).
    root: PathBuf,
    /// Global write mutex (Python `_write_lock`).
    write_lock: Mutex<()>,
}

impl Hermes {
    /// Open a store rooted at `root` (e.g. `~/.vitriol`).
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            write_lock: Mutex::new(()),
        }
    }

    /// Open a store rooted at `~/.vitriol` (honoring `VITRIOL_MEMORY_DIR`).
    pub fn default_root() -> PathBuf {
        std::env::var("VITRIOL_MEMORY_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(|h| PathBuf::from(h).join(".vitriol"))
                    .unwrap_or_else(|_| PathBuf::from(".vitriol"))
            })
    }

    /// The per-project database path.
    fn db_path(&self, project_id: &str) -> PathBuf {
        self.root.join(project_id).join("memory.db")
    }

    /// Open a connection with the Python pragmas + schema, creating the dir.
    pub fn conn(&self, project_id: &str) -> Result<Connection> {
        let path = self.db_path(project_id);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        }
        let conn = Connection::open(&path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "busy_timeout", 30_000)?;
        conn.execute_batch(SCHEMA_DDL)?;
        Ok(conn)
    }

    /// Trigger a passive WAL checkpoint to keep the WAL small (best effort).
    pub fn wal_checkpoint(&self, project_id: &str) {
        if let Ok(conn) = self.conn(project_id) {
            let _ = conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()));
        }
    }

    /// Get or create a session row; returns `(session_id, turn_count)`.
    pub fn get_or_create_session(
        &self,
        project_id: &str,
        session_id: &str,
    ) -> Result<(String, i64)> {
        let _g = self.write_lock.lock().unwrap();
        let conn = self.conn(project_id)?;
        let row = conn.query_row(
            "SELECT session_id, turn_count FROM sessions WHERE session_id = ?1",
            [session_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        );
        match row {
            Ok((sid, turns)) => {
                conn.execute(
                    "UPDATE sessions SET updated_at = datetime('now') WHERE session_id = ?1",
                    [&sid],
                )?;
                Ok((sid, turns))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                conn.execute(
                    "INSERT INTO sessions (session_id) VALUES (?1)",
                    [session_id],
                )?;
                Ok((session_id.to_string(), 0))
            }
            Err(e) => Err(e),
        }
    }

    /// Store a conversation turn; auto-assigns turn_index when absent, bumps the
    /// session turn_count, and links a `follows` edge to the previous episode.
    /// Returns the episode id.
    pub fn store_episode(
        &self,
        project_id: &str,
        session_id: &str,
        write: &EpisodeWrite,
    ) -> Result<i64> {
        let _g = self.write_lock.lock().unwrap();
        let conn = self.conn(project_id)?;
        let meta = &write.meta;
        let turn_index = match meta.turn_index {
            Some(t) => t,
            None => conn.query_row(
                "SELECT COALESCE(MAX(turn_index), -1) + 1 FROM episodes WHERE session_id = ?1",
                [session_id],
                |r| r.get::<_, i64>(0),
            )?,
        };
        conn.execute(
            "INSERT INTO episodes (session_id, turn_index, role, content, token_count)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session_id,
                turn_index,
                write.role,
                write.content,
                meta.token_count
            ],
        )?;
        let episode_id = conn.last_insert_rowid();
        conn.execute(
            "UPDATE sessions SET turn_count = turn_count + 1, updated_at = datetime('now')
             WHERE session_id = ?1",
            [session_id],
        )?;
        if turn_index > 0 {
            let prev = conn.query_row(
                "SELECT id FROM episodes WHERE session_id = ?1 AND turn_index = ?2",
                params![session_id, turn_index - 1],
                |r| r.get::<_, i64>(0),
            );
            if let Ok(prev_id) = prev {
                self.ensure_edge(
                    &conn,
                    &EdgeSpec::new("episode", prev_id, "episode", episode_id, "follows"),
                )?;
            }
        }
        Ok(episode_id)
    }

    /// Store a knowledge node, versioned by git_rev (Python P3.1 semantics):
    /// same git_rev refreshes the row; a new git_rev supersedes the current row
    /// (never hard-discard). Returns the node id.
    pub fn store_node(
        &self,
        project_id: &str,
        label: &str,
        summary: &str,
        meta: &NodeMeta,
    ) -> Result<i64> {
        let _g = self.write_lock.lock().unwrap();
        let conn = self.conn(project_id)?;
        let existing = conn.query_row(
            "SELECT id FROM knowledge_nodes WHERE label = ?1 AND git_rev = ?2",
            params![label, meta.git_rev],
            |r| r.get::<_, i64>(0),
        );
        if let Ok(id) = existing {
            conn.execute(
                "UPDATE knowledge_nodes SET summary = ?1, strength = ?2,
                 source_min = ?3, source_max = ?4 WHERE id = ?5",
                params![summary, meta.strength, meta.source_min, meta.source_max, id],
            )?;
            return Ok(id);
        }
        let current = conn.query_row(
            "SELECT id FROM knowledge_nodes WHERE label = ?1 AND superseded = 0",
            [label],
            |r| r.get::<_, i64>(0),
        );
        conn.execute(
            "INSERT INTO knowledge_nodes
             (label, summary, strength, source_min, source_max, git_rev, superseded)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            params![
                label,
                summary,
                meta.strength,
                meta.source_min,
                meta.source_max,
                meta.git_rev
            ],
        )?;
        let new_id = conn.last_insert_rowid();
        if let Ok(cur_id) = current {
            conn.execute(
                "UPDATE knowledge_nodes SET superseded = 1, superseded_by = ?1 WHERE id = ?2",
                params![new_id, cur_id],
            )?;
        }
        Ok(new_id)
    }

    /// Recent episodes for scoring (Python `search_episodes` fetch).
    pub fn fetch_episodes(&self, project_id: &str, limit: i64) -> Result<Vec<serde_json::Value>> {
        let conn = self.conn(project_id)?;
        let mut stmt = conn.prepare(
            "SELECT e.*, s.label as session_label
             FROM episodes e
             LEFT JOIN sessions s ON s.session_id = e.session_id
             ORDER BY e.id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit * 10], |r| {
            Ok(row_to_value(r, r.as_ref().column_count()))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Current knowledge nodes for scoring (superseded=0 unless history).
    pub fn fetch_nodes(
        &self,
        project_id: &str,
        limit: i64,
        include_history: bool,
    ) -> Result<Vec<serde_json::Value>> {
        let conn = self.conn(project_id)?;
        let sql = if include_history {
            "SELECT * FROM knowledge_nodes ORDER BY strength DESC, created_at DESC LIMIT ?1"
        } else {
            "SELECT * FROM knowledge_nodes WHERE superseded = 0
             ORDER BY strength DESC, created_at DESC LIMIT ?1"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([limit * 3], |r| {
            Ok(row_to_value(r, r.as_ref().column_count()))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Recent episodes from one session (Python `get_recent_episodes`).
    pub fn recent_episodes(
        &self,
        project_id: &str,
        session_id: &str,
        limit: i64,
    ) -> Result<Vec<serde_json::Value>> {
        let conn = self.conn(project_id)?;
        let mut stmt = conn.prepare(
            "SELECT * FROM episodes WHERE session_id = ?1 ORDER BY turn_index DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit], |r| {
            Ok(row_to_value(r, r.as_ref().column_count()))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Outgoing edges from a node (Python `get_outgoing_edges`).
    pub fn outgoing_edges(
        &self,
        project_id: &str,
        from_type: &str,
        from_id: i64,
    ) -> Result<Vec<serde_json::Value>> {
        let conn = self.conn(project_id)?;
        let mut stmt = conn.prepare("SELECT * FROM edges WHERE from_type = ?1 AND from_id = ?2")?;
        let rows = stmt.query_map(params![from_type, from_id], |r| {
            Ok(row_to_value(r, r.as_ref().column_count()))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Target nodes of all outgoing edges (Python `get_edge_targets`, explicit
    /// column lists after P3.1 split the UNION sides).
    pub fn edge_targets(
        &self,
        project_id: &str,
        from_type: &str,
        from_id: i64,
    ) -> Result<Vec<serde_json::Value>> {
        let conn = self.conn(project_id)?;
        let mut stmt = conn.prepare(
            "SELECT 'episode' AS _type, e.id, e.created_at, e.content AS content, NULL AS strength
             FROM edges ed
             JOIN episodes e ON ed.to_type = 'episode' AND e.id = ed.to_id
             WHERE ed.from_type = ?1 AND ed.from_id = ?2
             UNION
             SELECT 'node' AS _type, n.id, n.created_at, n.summary AS content, n.strength
             FROM edges ed
             JOIN knowledge_nodes n ON ed.to_type = 'node' AND n.id = ed.to_id
             WHERE ed.from_type = ?1 AND ed.from_id = ?2",
        )?;
        let rows = stmt.query_map(params![from_type, from_id], |r| {
            Ok(row_to_value(r, r.as_ref().column_count()))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Get or create an edge; returns its full row.
    pub fn get_or_create_edge(
        &self,
        project_id: &str,
        spec: &EdgeSpec,
    ) -> Result<serde_json::Value> {
        let _g = self.write_lock.lock().unwrap();
        let conn = self.conn(project_id)?;
        self.ensure_edge(&conn, spec)?;
        let mut stmt = conn.prepare(
            "SELECT * FROM edges WHERE from_type=?1 AND from_id=?2 AND to_type=?3 AND to_id=?4 AND relation=?5",
        )?;
        let row = stmt.query_row(
            params![
                spec.from_type,
                spec.from_id,
                spec.to_type,
                spec.to_id,
                spec.relation
            ],
            |r| Ok(row_to_value(r, r.as_ref().column_count())),
        )?;
        Ok(row)
    }

    /// Update an edge's weight + timestamp.
    pub fn update_edge_weight(&self, project_id: &str, edge_id: i64, weight: f64) -> Result<()> {
        let _g = self.write_lock.lock().unwrap();
        let conn = self.conn(project_id)?;
        conn.execute(
            "UPDATE edges SET weight = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![weight, edge_id],
        )?;
        Ok(())
    }

    /// Get a config value.
    pub fn get_config(&self, project_id: &str, key: &str) -> Result<Option<String>> {
        let conn = self.conn(project_id)?;
        conn.query_row("SELECT value FROM config WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
    }

    /// Set a config value.
    pub fn set_config(&self, project_id: &str, key: &str, value: &str) -> Result<()> {
        let _g = self.write_lock.lock().unwrap();
        let conn = self.conn(project_id)?;
        conn.execute(
            "INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    /// SHA-256 hex of content (embedding cache key, Python `_content_hash`).
    pub fn content_hash(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Retrieve a cached embedding blob by content hash.
    pub fn get_cached_embedding(
        &self,
        project_id: &str,
        content_hash: &str,
    ) -> Result<Option<Vec<u8>>> {
        let conn = self.conn(project_id)?;
        conn.query_row(
            "SELECT vector FROM embeddings WHERE content_hash = ?1",
            [content_hash],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
    }

    /// Store or replace a cached embedding blob.
    pub fn store_cached_embedding(
        &self,
        project_id: &str,
        content_hash: &str,
        content_type: &str,
        vector: &[u8],
    ) -> Result<()> {
        let _g = self.write_lock.lock().unwrap();
        let conn = self.conn(project_id)?;
        conn.execute(
            "INSERT OR REPLACE INTO embeddings (content_hash, content_type, vector)
             VALUES (?1, ?2, ?3)",
            params![content_hash, content_type, vector],
        )?;
        Ok(())
    }

    /// Internal upsert of an edge (Python `_ensure_edge`).
    fn ensure_edge(&self, conn: &Connection, spec: &EdgeSpec) -> Result<()> {
        conn.execute(
            "INSERT OR IGNORE INTO edges (from_type, from_id, to_type, to_id, relation, weight)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                spec.from_type,
                spec.from_id,
                spec.to_type,
                spec.to_id,
                spec.relation,
                spec.weight
            ],
        )?;
        Ok(())
    }
}

/// Convert a query row to a JSON object with string keys (mirrors Python's
/// `dict(sqlite3.Row)` — all values JSON-serialized).
fn row_to_value(row: &rusqlite::Row<'_>, ncols: usize) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for i in 0..ncols {
        let name = row.as_ref().column_name(i).unwrap_or("").to_string();
        match row.get_ref(i) {
            Ok(rusqlite::types::ValueRef::Integer(v)) => {
                map.insert(name, serde_json::json!(v));
            }
            Ok(rusqlite::types::ValueRef::Real(v)) => {
                map.insert(name, serde_json::json!(v));
            }
            Ok(rusqlite::types::ValueRef::Text(v)) => {
                map.insert(name, serde_json::json!(String::from_utf8_lossy(v)));
            }
            Ok(rusqlite::types::ValueRef::Blob(v)) => {
                map.insert(name, serde_json::json!(v.len()));
            }
            Ok(rusqlite::types::ValueRef::Null) => {
                map.insert(name, serde_json::Value::Null);
            }
            Err(_) => {}
        }
    }
    serde_json::Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store(name: &str) -> (PathBuf, Hermes) {
        let root = std::env::temp_dir().join(format!("hermes_db_test_{name}"));
        let _ = std::fs::remove_dir_all(&root);
        (root.clone(), Hermes::new(&root))
    }

    #[test]
    fn schema_matches_python_fixture() {
        // The fixture is captured from Python's _init_db (see scripts); the DDL
        // must be byte-identical so `sqlite3 .schema` diffs clean.
        let (root, h) = tmp_store("schema");
        let conn = h.conn("proj").unwrap();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','index') AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(
            tables,
            vec![
                "config",
                "edges",
                "embeddings",
                "episodes",
                "idx_episodes_created",
                "idx_episodes_session",
                "idx_nodes_label_cur",
                "knowledge_nodes",
                "sessions"
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sessions_get_or_create() {
        let (root, h) = tmp_store("session");
        let (sid, turns) = h.get_or_create_session("p", "sess1").unwrap();
        assert_eq!(sid, "sess1");
        assert_eq!(turns, 0);
        let (_, turns2) = h.get_or_create_session("p", "sess1").unwrap();
        assert_eq!(turns2, 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn store_episode_auto_turn_and_edges() {
        let (root, h) = tmp_store("episode");
        h.get_or_create_session("p", "s").unwrap();
        let id0 = h
            .store_episode(
                "p",
                "s",
                &EpisodeWrite {
                    role: "user".into(),
                    content: "hello".into(),
                    meta: EpisodeMeta::default(),
                },
            )
            .unwrap();
        let id1 = h
            .store_episode(
                "p",
                "s",
                &EpisodeWrite {
                    role: "assistant".into(),
                    content: "hi".into(),
                    meta: EpisodeMeta::default(),
                },
            )
            .unwrap();
        assert!(id0 < id1);
        // turn indices auto-assigned 0 and 1
        let eps = h.recent_episodes("p", "s", 10).unwrap();
        assert_eq!(eps.len(), 2);
        // a 'follows' edge links episode 0 -> 1
        let edges = h.outgoing_edges("p", "episode", id0).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0]["relation"], "follows");
        assert_eq!(edges[0]["to_id"], id1);
        // session turn_count bumped
        let (_, turns) = h.get_or_create_session("p", "s").unwrap();
        assert_eq!(turns, 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn store_node_versioned_supersede() {
        let (root, h) = tmp_store("node");
        let id0 = h
            .store_node(
                "p",
                "widget",
                "v1",
                &NodeMeta {
                    git_rev: "abc".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        // same git_rev refreshes in place
        let id1 = h
            .store_node(
                "p",
                "widget",
                "v1b",
                &NodeMeta {
                    git_rev: "abc".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(id0, id1);
        // new git_rev supersedes (never discards)
        let id2 = h
            .store_node(
                "p",
                "widget",
                "v2",
                &NodeMeta {
                    git_rev: "def".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_ne!(id2, id0);
        let current = h.fetch_nodes("p", 5, false).unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0]["id"], id2);
        let all = h.fetch_nodes("p", 5, true).unwrap();
        assert_eq!(all.len(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn edges_and_config_and_embeddings() {
        let (root, h) = tmp_store("misc");
        h.get_or_create_session("p", "s").unwrap();
        let id0 = h
            .store_episode(
                "p",
                "s",
                &EpisodeWrite {
                    role: "user".into(),
                    content: "a".into(),
                    meta: EpisodeMeta::default(),
                },
            )
            .unwrap();
        let id1 = h
            .store_episode(
                "p",
                "s",
                &EpisodeWrite {
                    role: "assistant".into(),
                    content: "b".into(),
                    meta: EpisodeMeta::default(),
                },
            )
            .unwrap();
        let edge = h
            .get_or_create_edge(
                "p",
                &EdgeSpec::new("episode", id0, "episode", id1, "follows"),
            )
            .unwrap();
        h.update_edge_weight("p", edge["id"].as_i64().unwrap(), 2.5)
            .unwrap();
        let targets = h.edge_targets("p", "episode", id0).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0]["_type"], "episode");

        assert_eq!(h.get_config("p", "k").unwrap(), None);
        h.set_config("p", "k", "v").unwrap();
        assert_eq!(h.get_config("p", "k").unwrap().as_deref(), Some("v"));

        let ch = Hermes::content_hash("content");
        assert_eq!(ch.len(), 64);
        h.store_cached_embedding("p", &ch, "episode", &[1.0f32.to_le_bytes()[0]])
            .unwrap();
        assert!(h.get_cached_embedding("p", &ch).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&root);
    }
}
