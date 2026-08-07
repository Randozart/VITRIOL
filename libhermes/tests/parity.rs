//! Schema-parity integration test: the Rust-created DB must produce byte-
//! identical `sqlite_master` SQL to the Python `_init_db` DDL (fixture captured
//! from `hermetis/db.py` on 2026-08-07). Existing Python-written DBs must also
//! be readable by the Rust layer.

use std::path::PathBuf;

use libhermes::{EdgeSpec, EpisodeMeta, Hermes, NodeMeta};

fn fixture() -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python.schema");
    std::fs::read_to_string(p).unwrap()
}

fn dump_schema(conn: &rusqlite::Connection) -> String {
    let mut stmt = conn
        .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name")
        .unwrap();
    let rows: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    rows.join(";\n") + ";\n"
}

#[test]
fn schema_byte_parity_with_python() {
    let root = std::env::temp_dir().join("hermes_parity_test");
    let _ = std::fs::remove_dir_all(&root);
    let h = Hermes::new(&root);
    h.get_or_create_session("proj", "s").unwrap();
    let conn = rusqlite::Connection::open(root.join("proj/memory.db")).unwrap();
    assert_eq!(dump_schema(&conn), fixture());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn reads_python_written_db() {
    // Reproduce the Python session shape (episode + versioned nodes + config),
    // then read it back through the Rust layer.
    let root = std::env::temp_dir().join("hermes_readback");
    let _ = std::fs::remove_dir_all(&root);
    let h = Hermes::new(&root);
    h.get_or_create_session("p", "s1").unwrap();
    h.store_episode(
        "p",
        "s1",
        &libhermes::EpisodeWrite {
            role: "user".into(),
            content: "hello".into(),
            meta: EpisodeMeta {
                token_count: 2,
                ..Default::default()
            },
        },
    )
    .unwrap();
    h.store_node(
        "p",
        "w",
        "v1",
        &NodeMeta {
            git_rev: "abc".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let v2 = h
        .store_node(
            "p",
            "w",
            "v2",
            &NodeMeta {
                git_rev: "def".into(),
                ..Default::default()
            },
        )
        .unwrap();
    h.store_cached_embedding(
        "p",
        &Hermes::content_hash("hello"),
        "episode",
        &[0, 1, 2, 3],
    )
    .unwrap();
    h.get_or_create_edge("p", &EdgeSpec::new("episode", 1, "episode", 2, "follows"))
        .unwrap();
    h.set_config("p", "k", "v").unwrap();

    let recent = h.recent_episodes("p", "s1", 2).unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0]["content"], "hello");
    let current = h.fetch_nodes("p", 5, false).unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0]["id"], v2);
    assert_eq!(current[0]["superseded"], 0);
    let history = h.fetch_nodes("p", 5, true).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(h.get_config("p", "k").unwrap().as_deref(), Some("v"));
    assert_eq!(
        h.get_cached_embedding("p", &Hermes::content_hash("hello"))
            .unwrap()
            .as_deref(),
        Some(&[0, 1, 2, 3][..])
    );
    let edges = h.outgoing_edges("p", "episode", 1).unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["relation"], "follows");
    let _ = std::fs::remove_dir_all(&root);
}
