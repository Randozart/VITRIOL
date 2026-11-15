use serde_json::Value;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use vitriol_cache_store::{Config, Store};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "vitriol-managed-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self { path }
    }
    fn owned(&self) -> PathBuf {
        self.path.join("owned")
    }
    fn open(&self, reclaim: bool) -> Store {
        Store::open(&self.owned(), config(reclaim)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn config(reclaim: bool) -> Config {
    Config {
        max_bytes: 1024,
        reclaim,
        max_gc_files: 1,
        max_gc_bytes: 4,
    }
}

fn publish(store: &mut Store, generation: u64) -> String {
    let publication = store.prepare(generation, "model", "layout", 4).unwrap();
    fs::write(&publication.path, [7u8; 4]).unwrap();
    store.commit(publication.ticket).unwrap()
}

fn acknowledged(store: &mut Store, session: &str, branch: &str) -> String {
    let grant = store.begin(session, branch).unwrap();
    store.finish(grant.generation, true).unwrap();
    let id = publish(store, grant.generation);
    store.release_tag(grant.generation).unwrap();
    id
}

#[test]
fn publication_requires_success_ack_and_keeps_two_branch_rollback_states() {
    let fixture = Fixture::new();
    let mut store = fixture.open(false);
    let first = acknowledged(&mut store, "s", "main");
    let second = acknowledged(&mut store, "s", "main");
    let third = acknowledged(&mut store, "s", "main");
    let audit = store.audit().unwrap();
    assert_eq!(audit.eligible_bytes, 4);
    assert!(
        audit
            .decisions
            .iter()
            .find(|d| d.id == first)
            .unwrap()
            .eligible
    );
    assert!(
        !audit
            .decisions
            .iter()
            .find(|d| d.id == second)
            .unwrap()
            .eligible
    );
    assert!(
        !audit
            .decisions
            .iter()
            .find(|d| d.id == third)
            .unwrap()
            .eligible
    );
    assert!(!audit.deletion_authorized);
    assert!(
        fixture.owned().join(first).exists(),
        "dry-run default retains physical files"
    );
}

#[test]
fn bounded_reclamation_uses_store_authority_and_persistent_incarnations() {
    let fixture = Fixture::new();
    let mut store = fixture.open(false);
    for _ in 0..4 {
        acknowledged(&mut store, "s", "main");
    }
    let previous_epoch = store.audit().unwrap().inventory_epoch;
    assert_eq!(store.collect(previous_epoch, false).unwrap()["removed"], 0);
    drop(store);
    let mut store = fixture.open(true);
    let epoch = store.audit().unwrap().inventory_epoch;
    assert!(epoch > previous_epoch);
    assert!(store.collect(previous_epoch, false).is_err());
    let result = store.collect(epoch, false).unwrap();
    assert_eq!(result["removed"], 1);
    assert_eq!(result["bytes"], 4);
    assert_eq!(store.audit().unwrap().eligible_bytes, 4);
    let epoch = store.audit().unwrap().inventory_epoch;
    assert_eq!(store.collect(epoch, false).unwrap()["removed"], 1);
    assert_eq!(store.audit().unwrap().eligible_bytes, 0);
}

#[test]
fn active_read_lease_prevents_unlink_until_release() {
    let fixture = Fixture::new();
    let mut store = fixture.open(true);
    let first = acknowledged(&mut store, "s", "main");
    let lease = store.acquire_read(&first, None).unwrap();
    acknowledged(&mut store, "s", "main");
    acknowledged(&mut store, "s", "main");
    assert!(fixture.owned().join(&first).exists());
    store.release_read(lease).unwrap();
    let epoch = store.audit().unwrap().inventory_epoch;
    store.collect(epoch, false).unwrap();
    assert!(!fixture.owned().join(first).exists());
}

#[test]
fn a_borrowing_session_must_also_advance_before_the_shared_state_can_retire() {
    let fixture = Fixture::new();
    let mut store = fixture.open(true);
    let first = acknowledged(&mut store, "producer", "main");
    let borrow = store.begin("borrower", "fork").unwrap();
    let lease = store.acquire_read(&first, Some(borrow.generation)).unwrap();
    store.release_read(lease).unwrap();
    store.finish(borrow.generation, true).unwrap();
    store.release_tag(borrow.generation).unwrap();
    acknowledged(&mut store, "producer", "main");
    acknowledged(&mut store, "producer", "main");
    assert!(fixture.owned().join(&first).exists());
    acknowledged(&mut store, "borrower", "fork");
    assert!(!fixture.owned().join(first).exists());
}

#[test]
fn late_detached_writer_is_fenced_and_cannot_recreate_a_retired_file() {
    let fixture = Fixture::new();
    let mut store = fixture.open(true);
    let grant = store.begin("s", "main").unwrap();
    store.finish(grant.generation, true).unwrap();
    let pending = store
        .prepare(grant.generation, "model", "layout", 4)
        .unwrap();
    let first = publish(&mut store, grant.generation);
    acknowledged(&mut store, "s", "main");
    acknowledged(&mut store, "s", "main");
    assert!(!fixture.owned().join(first).exists());
    fs::write(&pending.path, [9u8; 4]).unwrap();
    assert!(store
        .commit(pending.ticket)
        .unwrap_err()
        .contains("stale publication fenced"));
    assert!(!fixture.owned().join(pending.id).exists());
    assert!(
        fs::write(pending.path, [1u8; 4]).is_err(),
        "revocation removes the staging directory"
    );
}

#[test]
fn failed_and_unfinished_generations_cannot_advance_the_durable_frontier() {
    let fixture = Fixture::new();
    let mut store = fixture.open(true);
    let first = acknowledged(&mut store, "s", "main");
    let grant = store.begin("s", "main").unwrap();
    let failed = publish(&mut store, grant.generation);
    assert!(
        store
            .audit()
            .unwrap()
            .decisions
            .iter()
            .find(|d| d.id == failed)
            .unwrap()
            .reason
            == vitriol_core::cache_lifetime::Reason::Pinned
    );
    store.finish(grant.generation, false).unwrap();
    assert!(store.finish(grant.generation, true).is_err());
    acknowledged(&mut store, "s", "main");
    assert!(
        fixture.owned().join(first).exists(),
        "failed publication did not consume a rollback anchor"
    );
}

#[test]
fn branch_heads_and_explicit_rewind_pins_are_independent() {
    use vitriol_core::cache_lifetime::Pin;
    let fixture = Fixture::new();
    let mut store = fixture.open(true);
    let main = acknowledged(&mut store, "s", "main");
    let fork = acknowledged(&mut store, "s", "fork");
    store.pin(&main, Pin::Rewind, true).unwrap();
    acknowledged(&mut store, "s", "main");
    acknowledged(&mut store, "s", "main");
    assert!(fixture.owned().join(&main).exists());
    assert!(fixture.owned().join(fork).exists());
    store.pin(&main, Pin::Rewind, false).unwrap();
    let epoch = store.audit().unwrap().inventory_epoch;
    store.collect(epoch, false).unwrap();
    assert!(!fixture.owned().join(main).exists());
}

#[test]
fn an_exclusive_namespace_lock_and_unknown_files_survive_restart() {
    let fixture = Fixture::new();
    let mut store = fixture.open(false);
    assert!(Store::open(&fixture.owned(), config(false)).is_err());
    let first = acknowledged(&mut store, "s", "main");
    fs::write(fixture.owned().join("cache-unknown.bin"), b"unowned").unwrap();
    let grant = store.begin("s", "main").unwrap();
    let pending = store
        .prepare(grant.generation, "model", "layout", 4)
        .unwrap();
    fs::write(&pending.path, [3u8; 4]).unwrap();
    drop(store);
    let store = fixture.open(false);
    assert!(fixture.owned().join(first).exists());
    assert!(fixture.owned().join("cache-unknown.bin").exists());
    assert!(!PathBuf::from(pending.path).exists());
    assert_eq!(store.audit().unwrap().total_bytes, 4);
}

#[test]
fn recovery_after_file_publication_before_metadata_keeps_the_previous_head() {
    let fixture = Fixture::new();
    let mut store = fixture.open(false);
    let first = acknowledged(&mut store, "s", "main");
    let grant = store.begin("s", "main").unwrap();
    let pending = store
        .prepare(grant.generation, "model", "layout", 4)
        .unwrap();
    fs::write(&pending.path, [8u8; 4]).unwrap();
    let metadata = fs::metadata(&pending.path).unwrap();
    let path = fixture.owned().join("ledger.json");
    let mut ledger: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    ledger["pending"][pending.ticket.to_string()]["fileIdentity"] =
        serde_json::json!({"dev":metadata.dev(),"ino":metadata.ino(),"bytes":metadata.len()});
    fs::write(&path, serde_json::to_vec(&ledger).unwrap()).unwrap();
    fs::hard_link(&pending.path, fixture.owned().join(&pending.id)).unwrap();
    drop(store);
    let store = fixture.open(false);
    assert!(fixture.owned().join(first).exists());
    assert!(!fixture.owned().join(pending.id).exists());
    assert_eq!(store.audit().unwrap().total_bytes, 4);
}

#[test]
fn tombstones_recover_after_unlink_and_unrecognized_incarnations_are_retained() {
    let fixture = Fixture::new();
    let mut store = fixture.open(false);
    let first = acknowledged(&mut store, "s", "main");
    acknowledged(&mut store, "s", "main");
    acknowledged(&mut store, "s", "main");
    let path = fixture.owned().join("ledger.json");
    let mut ledger: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let record = ledger["records"]
        .as_object_mut()
        .unwrap()
        .remove(&first)
        .unwrap();
    ledger["tombstones"][&first] = record;
    fs::write(path, serde_json::to_vec(&ledger).unwrap()).unwrap();
    fs::remove_file(fixture.owned().join(&first)).unwrap();
    drop(store);
    let mut store = fixture.open(true);
    let record = store.list().unwrap()["snapshots"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let lease = store.acquire_read(&record, None).unwrap();
    fs::remove_file(fixture.owned().join(&record)).unwrap();
    fs::write(fixture.owned().join(&record), b"different").unwrap();
    assert!(store.acquire_read(&record, None).is_err());
    assert_eq!(
        fs::read(fixture.owned().join(record)).unwrap(),
        b"different"
    );
    let _ = store.release_read(lease);
}

#[test]
fn pending_reservations_and_actual_file_size_are_bounded() {
    let fixture = Fixture::new();
    let mut cfg = config(false);
    cfg.max_bytes = 8;
    let mut store = Store::open(&fixture.owned(), cfg).unwrap();
    let grant = store.begin("s", "main").unwrap();
    let first = store.prepare(grant.generation, "m", "l", 6).unwrap();
    assert!(store.prepare(grant.generation, "m", "l", 6).is_err());
    fs::write(&first.path, [0u8; 9]).unwrap();
    assert!(store.commit(first.ticket).is_err());
    assert!(!PathBuf::from(first.path).exists());
    assert_eq!(store.audit().unwrap().total_bytes, 0);
}

#[test]
fn untrusted_namespace_or_metadata_is_not_reinitialized_as_empty() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.owned()).unwrap();
    fs::set_permissions(fixture.owned(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Store::open(&fixture.owned(), config(false)).is_err());
    fs::set_permissions(fixture.owned(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(fixture.owned().join("ledger.json"), b"corrupt").unwrap();
    assert!(Store::open(&fixture.owned(), config(false)).is_err());
    assert_eq!(
        fs::read(fixture.owned().join("ledger.json")).unwrap(),
        b"corrupt"
    );
}

#[test]
fn failed_publication_collision_never_removes_an_unknown_existing_file() {
    let fixture = Fixture::new();
    let mut store = fixture.open(false);
    let grant = store.begin("s", "main").unwrap();
    let pending = store
        .prepare(grant.generation, "model", "layout", 4)
        .unwrap();
    fs::write(&pending.path, [1u8; 4]).unwrap();
    let foreign = fixture.owned().join(&pending.id);
    fs::write(&foreign, b"unowned-existing-file").unwrap();
    assert!(store.commit(pending.ticket).is_err());
    assert!(
        store.begin("other", "branch").is_err(),
        "publication failure disables managed effects"
    );
    drop(store);
    assert!(Store::open(&fixture.owned(), config(false)).is_err());
    assert_eq!(fs::read(foreign).unwrap(), b"unowned-existing-file");
}

#[test]
fn a_pending_ram_borrower_keeps_publication_live_after_the_producer_advances() {
    let fixture = Fixture::new();
    let mut store = fixture.open(true);
    let producer = store.begin("producer", "main").unwrap();
    store.finish(producer.generation, true).unwrap();
    let publication = store
        .prepare(producer.generation, "model", "layout", 4)
        .unwrap();
    fs::write(&publication.path, [4u8; 4]).unwrap();
    let borrower = store.begin("borrower", "fork").unwrap();
    store.attach(&publication.id, borrower.generation).unwrap();
    store.finish(borrower.generation, true).unwrap();
    acknowledged(&mut store, "producer", "main");
    acknowledged(&mut store, "producer", "main");
    let id = store.commit(publication.ticket).unwrap();
    assert!(fixture.owned().join(&id).exists());
    acknowledged(&mut store, "borrower", "fork");
    assert!(!fixture.owned().join(id).exists());
}

#[test]
fn rollback_anchors_and_retirement_are_partitioned_by_model_and_layout() {
    let fixture = Fixture::new();
    let mut store = fixture.open(false);
    for (model, layout) in [
        ("model-a", "layout-a"),
        ("model-b", "layout-a"),
        ("model-a", "layout-b"),
    ]
    .repeat(3)
    {
        let grant = store.begin("s", "main").unwrap();
        store.finish(grant.generation, true).unwrap();
        let publication = store.prepare(grant.generation, model, layout, 4).unwrap();
        fs::write(&publication.path, [4u8; 4]).unwrap();
        store.commit(publication.ticket).unwrap();
    }
    let audit = store.audit().unwrap();
    assert_eq!(audit.eligible_bytes, 12);
    assert_eq!(audit.decisions.iter().filter(|d| !d.eligible).count(), 6);
}
