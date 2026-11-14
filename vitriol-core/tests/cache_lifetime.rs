use vitriol_core::cache_lifetime::{plan, Inventory, Owner, Pin, Reason, Retirement, Snapshot};

fn owner(session: &str, generation: u64) -> Owner {
    Owner {
        session_id: session.into(),
        branch_id: "main".into(),
        generation,
    }
}

fn snapshot(id: &str, generation: u64) -> Snapshot {
    Snapshot {
        id: id.into(),
        incarnation: 7,
        model_key: "model".into(),
        layout_key: "layout".into(),
        bytes: 100,
        durable: true,
        owners: Some(vec![owner("one", generation)]),
        pins: vec![],
        readers: 0,
        writers: 0,
    }
}

fn inventory() -> Inventory {
    Inventory {
        version: 1,
        epoch: 42,
        reference_coverage_complete: true,
        snapshots: vec![snapshot("old", 1), snapshot("new", 2)],
        retirements: vec![Retirement {
            snapshot_id: "old".into(),
            owner: owner("one", 1),
            successor_id: "new".into(),
        }],
    }
}

fn old_reason(input: &Inventory) -> Reason {
    plan(input)
        .unwrap()
        .decisions
        .into_iter()
        .find(|d| d.id == "old")
        .unwrap()
        .reason
}

#[test]
fn explicit_retirement_is_auditable_but_never_an_unlink_permit() {
    let output = plan(&inventory()).unwrap();
    assert_eq!(output.inventory_epoch, 42);
    assert_eq!(output.total_bytes, 200);
    assert_eq!(output.eligible_bytes, 100);
    assert!(!output.deletion_authorized);
    assert_eq!(output.decisions[0].incarnation, 7);
    assert_eq!(output.decisions[0].reason, Reason::Retired);
    assert_eq!(output.decisions[1].reason, Reason::RequiredReplacement);
}

#[test]
fn moving_on_does_not_retire_an_ancestor_or_a_recurrent_rollback_point() {
    let mut input = inventory();
    input.retirements.clear();
    assert_eq!(old_reason(&input), Reason::OwnerNotRetired);
    for pin in [Pin::Branch, Pin::Rewind, Pin::RollbackAnchor] {
        input = inventory();
        input.snapshots[0].pins = vec![pin];
        assert_eq!(old_reason(&input), Reason::Pinned);
    }
}

#[test]
fn unknown_ownership_and_incomplete_reference_coverage_retain_everything() {
    for owners in [None, Some(vec![])] {
        let mut input = inventory();
        input.retirements.clear();
        input.snapshots[0].owners = owners;
        assert_eq!(old_reason(&input), Reason::UnknownOwnership);
    }
    let mut input = inventory();
    input.reference_coverage_complete = false;
    assert_eq!(old_reason(&input), Reason::IncompleteInventory);
    assert_eq!(plan(&input).unwrap().eligible_bytes, 0);
}

#[test]
fn every_shared_owner_must_release_its_exact_generation() {
    let mut input = inventory();
    input.snapshots[0]
        .owners
        .as_mut()
        .unwrap()
        .push(owner("two", 4));
    assert_eq!(old_reason(&input), Reason::OwnerNotRetired);
    input.retirements.push(Retirement {
        snapshot_id: "old".into(),
        owner: owner("two", 4),
        successor_id: "new".into(),
    });
    assert_eq!(old_reason(&input), Reason::UnsafeSuccessor);
    input.snapshots[1]
        .owners
        .as_mut()
        .unwrap()
        .push(owner("two", 5));
    assert_eq!(old_reason(&input), Reason::Retired);
}

#[test]
fn publication_and_readers_protect_the_physical_snapshot() {
    let mut input = inventory();
    input.snapshots[0].durable = false;
    assert_eq!(old_reason(&input), Reason::PendingWrite);
    input.snapshots[0].durable = true;
    input.snapshots[0].writers = 1;
    assert_eq!(old_reason(&input), Reason::PendingWrite);
    input.snapshots[0].writers = 0;
    input.snapshots[0].readers = 1;
    input.epoch += 1;
    assert_eq!(old_reason(&input), Reason::ActiveReader);
    assert_eq!(plan(&input).unwrap().inventory_epoch, 43);
}

#[test]
fn replacement_must_be_durable_later_same_branch_and_same_engine_identity() {
    let mutations: [fn(&mut Snapshot); 9] = [
        |s| s.durable = false,
        |s| s.writers = 1,
        |s| s.model_key = "other-model".into(),
        |s| s.layout_key = "other-layout".into(),
        |s| s.owners = None,
        |s| s.owners = Some(vec![]),
        |s| s.owners = Some(vec![owner("other-session", 2)]),
        |s| s.owners.as_mut().unwrap()[0].branch_id = "fork".into(),
        |s| s.owners.as_mut().unwrap()[0].generation = 1,
    ];
    for (case, mutate) in mutations.iter().enumerate() {
        let mut input = inventory();
        mutate(&mut input.snapshots[1]);
        assert_eq!(old_reason(&input), Reason::UnsafeSuccessor, "case {case}");
    }
    let mut input = inventory();
    input.snapshots[1].readers = 2;
    assert_eq!(
        old_reason(&input),
        Reason::Retired,
        "a live durable successor is a valid witness"
    );
}

#[test]
fn a_retirement_batch_keeps_its_witnesses_and_cannot_eat_a_cycle() {
    let mut input = inventory();
    input.snapshots.push(snapshot("newest", 3));
    input.retirements.push(Retirement {
        snapshot_id: "new".into(),
        owner: owner("one", 2),
        successor_id: "newest".into(),
    });
    let output = plan(&input).unwrap();
    assert_eq!(output.eligible_bytes, 100);
    assert!(!output.decisions[1].eligible);
    input.retirements[1].successor_id = "old".into();
    assert_eq!(plan(&input).unwrap().eligible_bytes, 0);
    input = inventory();
    input.retirements[0].successor_id = "old".into();
    assert_eq!(plan(&input).unwrap().eligible_bytes, 0);
}

#[test]
fn inventory_order_does_not_change_policy() {
    let input = inventory();
    let mut reordered = input.clone();
    reordered.snapshots.reverse();
    let mut left = plan(&input).unwrap().decisions;
    let mut right = plan(&reordered).unwrap().decisions;
    left.sort_by(|a, b| a.id.cmp(&b.id));
    right.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(left, right);
}

#[test]
fn invalid_or_ambiguous_inventory_fails_closed() {
    let mutations: [fn(&mut Inventory); 10] = [
        |i| i.version = 2,
        |i| i.snapshots.push(i.snapshots[0].clone()),
        |i| {
            i.snapshots[0]
                .owners
                .as_mut()
                .unwrap()
                .push(owner("one", 1))
        },
        |i| i.retirements.push(i.retirements[0].clone()),
        |i| i.retirements[0].snapshot_id = "missing".into(),
        |i| i.retirements[0].successor_id = "missing".into(),
        |i| i.retirements[0].owner = owner("not-an-owner", 1),
        |i| i.snapshots[0].model_key.clear(),
        |i| i.snapshots[0].owners.as_mut().unwrap()[0].branch_id.clear(),
        |i| i.snapshots[0].bytes = u64::MAX,
    ];
    for (case, mutate) in mutations.iter().enumerate() {
        let mut input = inventory();
        mutate(&mut input);
        assert!(plan(&input).is_err(), "case {case}");
    }
}

#[test]
fn wire_schema_does_not_default_missing_protection_facts_to_safe() {
    let valid = serde_json::to_value(inventory()).unwrap();
    for field in ["readers", "writers", "pins", "durable", "incarnation"] {
        let mut broken = valid.clone();
        broken["snapshots"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            serde_json::from_value::<Inventory>(broken).is_err(),
            "field {field}"
        );
    }
    let mut unknown = valid.clone();
    unknown["snapshots"][0]["pins"] = serde_json::json!(["future-pin"]);
    assert!(serde_json::from_value::<Inventory>(unknown).is_err());
    unknown = valid;
    unknown["retireAll"] = serde_json::json!(true);
    assert!(serde_json::from_value::<Inventory>(unknown).is_err());
}
