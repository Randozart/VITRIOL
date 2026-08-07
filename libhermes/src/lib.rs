//! libhermes — the Hermetis memory subsystem, in Rust.
//!
//! SQLite-backed persistent memory per project, a byte-parity port of
//! `libvitriol/hermetis/db.py`. Each project gets its own
//! `<memory_root>/<project_id>/memory.db`, created on first access, with the
//! exact same DDL, WAL pragmas, write serialization, and versioned supersede
//! semantics so existing data stays readable and the HTTP contract is unchanged.
//!
//! P1 scope: the database layer (schema + WAL + write lock + sessions /
//! episodes / knowledge_nodes / edges / config / embedding cache). Retrieval,
//! scoring, the HTTP server, and consolidation are later phases.
//!
//! Write lock: a single `Mutex` serializes every write, mirroring Python's
//! global `_write_lock` (2026-08-06 rationale: concurrent writers hit SQLite
//! "database is locked" stalls). Reads open a fresh connection (WAL allows
//! concurrent readers).

pub mod compact;
pub mod consolidate;
pub mod db;
pub mod hebbian;
pub mod pymander;
pub mod repomap;
pub mod retrieval;
pub mod scorer;
pub mod server;

/// Convenience re-exports.
pub use db::{EdgeSpec, EpisodeMeta, EpisodeWrite, Hermes, NodeMeta};
pub use retrieval::{retrieve, RetrieveParams};
pub use server::{router, ServerState};
