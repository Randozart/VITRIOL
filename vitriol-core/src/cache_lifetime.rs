//! Read-only cache retirement policy. Eligibility requires authoritative lifecycle
//! facts; it is never permission to unlink a file from an unsynchronized scan.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Owner {
    pub session_id: String,
    pub branch_id: String,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Pin {
    Branch,
    Rewind,
    RollbackAnchor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Snapshot {
    pub id: String,
    pub incarnation: u64,
    pub model_key: String,
    pub layout_key: String,
    pub bytes: u64,
    pub durable: bool,
    /// None/empty is legacy or unknown, never evidence of zero references.
    pub owners: Option<Vec<Owner>>,
    pub pins: Vec<Pin>,
    pub readers: u64,
    pub writers: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Retirement {
    pub snapshot_id: String,
    pub owner: Owner,
    pub successor_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Inventory {
    pub version: u32,
    pub epoch: u64,
    /// All owners, pins and in-flight readers/writers are accounted for.
    pub reference_coverage_complete: bool,
    pub snapshots: Vec<Snapshot>,
    pub retirements: Vec<Retirement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    UnknownOwnership,
    IncompleteInventory,
    PendingWrite,
    ActiveReader,
    Pinned,
    RequiredReplacement,
    OwnerNotRetired,
    UnsafeSuccessor,
    Retired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Decision {
    pub id: String,
    pub incarnation: u64,
    pub eligible: bool,
    pub reason: Reason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Plan {
    pub version: u32,
    pub inventory_epoch: u64,
    pub deletion_authorized: bool,
    pub total_bytes: u64,
    pub eligible_bytes: u64,
    pub decisions: Vec<Decision>,
}

#[derive(Default)]
struct View<'a> {
    snapshots: HashMap<&'a str, &'a Snapshot>,
    retirements: HashMap<(&'a str, &'a Owner), &'a str>,
    replacements: HashSet<&'a str>,
    owner_refs: HashSet<(&'a str, &'a Owner)>,
    generations: HashMap<(&'a str, &'a str, &'a str), u64>,
    complete: bool,
}

/// Reject ambiguous identity or reference input before making any decisions.
fn validate_snapshot(snapshot: &Snapshot) -> Result<(), String> {
    if snapshot.id.is_empty() || snapshot.model_key.is_empty() || snapshot.layout_key.is_empty() {
        return Err("snapshot identity must not be empty".into());
    }
    let mut owners = HashSet::new();
    for owner in snapshot.owners.iter().flatten() {
        if owner.session_id.is_empty() || owner.branch_id.is_empty() || !owners.insert(owner) {
            return Err(
                "snapshot owners must be unique nonempty session/branch generations".into(),
            );
        }
    }
    Ok(())
}

/// Validate and index one explicit owner release without mutating external state.
fn add_retirement<'a>(view: &mut View<'a>, retirement: &'a Retirement) -> Result<(), String> {
    if !view.snapshots.contains_key(retirement.snapshot_id.as_str()) {
        return Err("retired snapshot missing".into());
    }
    if !view
        .owner_refs
        .contains(&(retirement.snapshot_id.as_str(), &retirement.owner))
    {
        return Err("retirement owner does not own snapshot".into());
    }
    if !view
        .snapshots
        .contains_key(retirement.successor_id.as_str())
    {
        return Err("successor snapshot missing".into());
    }
    let key = (retirement.snapshot_id.as_str(), &retirement.owner);
    if view
        .retirements
        .insert(key, &retirement.successor_id)
        .is_some()
    {
        return Err("duplicate owner retirement".into());
    }
    view.replacements.insert(&retirement.successor_id);
    Ok(())
}

/// Require explicit retirement records to refer to real owners and real replacements.
fn build_view(inventory: &Inventory) -> Result<View<'_>, String> {
    let mut view = View {
        complete: inventory.reference_coverage_complete,
        ..View::default()
    };
    for snapshot in &inventory.snapshots {
        validate_snapshot(snapshot)?;
        if view.snapshots.insert(&snapshot.id, snapshot).is_some() {
            return Err("duplicate snapshot id".into());
        }
    }
    // One pass over total references, independent of how many sessions share each prefix.
    let references = inventory.snapshots.iter().flat_map(|snapshot| {
        snapshot
            .owners
            .iter()
            .flatten()
            .map(move |owner| (snapshot.id.as_str(), owner))
    });
    for (id, owner) in references {
        view.owner_refs.insert((id, owner));
        let key = (id, owner.session_id.as_str(), owner.branch_id.as_str());
        let generation = view.generations.entry(key).or_insert(0);
        *generation = (*generation).max(owner.generation);
    }
    for retirement in &inventory.retirements {
        add_retirement(&mut view, retirement)?;
    }
    Ok(view)
}

/// A durable later state is a witness only for the same owner and engine layout.
fn safe_successor(
    snapshot: &Snapshot,
    owner: &Owner,
    successor: &Snapshot,
    view: &View<'_>,
) -> bool {
    successor.id != snapshot.id
        && successor.durable
        && successor.writers == 0
        && successor.model_key == snapshot.model_key
        && successor.layout_key == snapshot.layout_key
        && view
            .generations
            .get(&(
                successor.id.as_str(),
                owner.session_id.as_str(),
                owner.branch_id.as_str(),
            ))
            .is_some_and(|generation| *generation > owner.generation)
}

/// Retain every protected or unaccounted-for state; longer prefixes alone prove nothing.
fn reason(snapshot: &Snapshot, view: &View<'_>) -> Reason {
    let Some(owners) = snapshot.owners.as_ref().filter(|owners| !owners.is_empty()) else {
        return Reason::UnknownOwnership;
    };
    if !view.complete {
        return Reason::IncompleteInventory;
    }
    if !snapshot.durable || snapshot.writers > 0 {
        return Reason::PendingWrite;
    }
    if snapshot.readers > 0 {
        return Reason::ActiveReader;
    }
    if !snapshot.pins.is_empty() {
        return Reason::Pinned;
    }
    if view.replacements.contains(snapshot.id.as_str()) {
        return Reason::RequiredReplacement;
    }
    for owner in owners {
        let Some(next_id) = view.retirements.get(&(snapshot.id.as_str(), owner)) else {
            return Reason::OwnerNotRetired;
        };
        if !safe_successor(snapshot, owner, view.snapshots[next_id], view) {
            return Reason::UnsafeSuccessor;
        }
    }
    Reason::Retired
}

/// Produce an epoch-bound audit. Physical reclamation requires a separate synchronized executor.
pub fn plan(inventory: &Inventory) -> Result<Plan, String> {
    if inventory.version != 1 {
        return Err("unsupported cache inventory version".into());
    }
    let view = build_view(inventory)?;
    let mut result = Plan {
        version: 1,
        inventory_epoch: inventory.epoch,
        deletion_authorized: false,
        total_bytes: 0,
        eligible_bytes: 0,
        decisions: Vec::new(),
    };
    for snapshot in &inventory.snapshots {
        result.total_bytes = result
            .total_bytes
            .checked_add(snapshot.bytes)
            .ok_or("inventory byte count overflow")?;
        let reason = reason(snapshot, &view);
        let eligible = reason == Reason::Retired;
        if eligible {
            result.eligible_bytes = result
                .eligible_bytes
                .checked_add(snapshot.bytes)
                .ok_or("eligible byte count overflow")?;
        }
        result.decisions.push(Decision {
            id: snapshot.id.clone(),
            incarnation: snapshot.incarnation,
            eligible,
            reason,
        });
    }
    Ok(result)
}
