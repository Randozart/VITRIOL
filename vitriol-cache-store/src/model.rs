use crate::disk::FileIdentity;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use vitriol_core::cache_lifetime::{Owner, Retirement, Snapshot};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub max_bytes: u64,
    pub reclaim: bool,
    pub max_gc_files: usize,
    pub max_gc_bytes: u64,
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_bytes == 0 || self.max_gc_files == 0 || self.max_gc_bytes == 0 {
            return Err("cache and reclamation bounds must be positive".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    pub generation: u64,
    pub owner: Owner,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Publication {
    pub ticket: u64,
    pub id: String,
    pub path: String,
}

pub struct Tag {
    pub owner: Owner,
    pub finished: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pending {
    pub id: String,
    pub owner: Owner,
    pub model_key: String,
    pub layout_key: String,
    pub reserved_bytes: u64,
    pub file_identity: Option<FileIdentity>,
    #[serde(default)]
    pub borrowers: Vec<Owner>,
}

impl Pending {
    pub fn new(id: String, owner: Owner, model: &str, layout: &str, bytes: u64) -> Self {
        Self {
            id,
            owner,
            model_key: model.into(),
            layout_key: layout.into(),
            reserved_bytes: bytes,
            file_identity: None,
            borrowers: Vec::new(),
        }
    }

    fn validate(&self, ticket: u64, next_id: u64) -> Result<(), String> {
        if ticket == 0
            || self.id != file_id(ticket)
            || ticket > next_id
            || self.owner.generation > next_id
        {
            return Err("invalid pending identity".into());
        }
        valid_owner(&self.owner, next_id)?;
        let mut owners = HashSet::from([&self.owner]);
        for owner in &self.borrowers {
            valid_owner(owner, next_id)?;
            if !owners.insert(owner) {
                return Err("invalid pending borrower".into());
            }
        }
        valid_label(&self.model_key)?;
        valid_label(&self.layout_key)
    }

    /// A shared pending publication stays valid until every attached owner moves on.
    pub fn superseded(&self, ledger: &Ledger) -> bool {
        std::iter::once(&self.owner)
            .chain(&self.borrowers)
            .all(|owner| {
                ledger.frontier(owner, &self.model_key, &self.layout_key) > owner.generation
            })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Record {
    pub snapshot: Snapshot,
    pub producer: Owner,
    pub identity: FileIdentity,
}

impl Record {
    pub fn published(pending: Pending, incarnation: u64, identity: FileIdentity) -> Self {
        let owners = std::iter::once(pending.owner.clone())
            .chain(pending.borrowers)
            .collect();
        Self {
            producer: pending.owner.clone(),
            snapshot: Snapshot {
                id: pending.id,
                incarnation,
                model_key: pending.model_key,
                layout_key: pending.layout_key,
                bytes: identity.bytes,
                durable: true,
                owners: Some(owners),
                pins: vec![],
                readers: 0,
                writers: 0,
            },
            identity,
        }
    }

    fn validate(&self, id: &str, next_id: u64) -> Result<(), String> {
        self.validate_identity(id, next_id)?;
        let mut owners = HashSet::new();
        for owner in self.snapshot.owners.iter().flatten() {
            valid_owner(owner, next_id)?;
            if !owners.insert(owner) {
                return Err("invalid managed owner reference".into());
            }
        }
        if !owners.contains(&self.producer) {
            return Err("managed producer reference missing".into());
        }
        valid_label(&self.snapshot.model_key)?;
        valid_label(&self.snapshot.layout_key)
    }

    fn validate_identity(&self, id: &str, next_id: u64) -> Result<(), String> {
        if id != file_id(self.snapshot.incarnation)
            || id != self.snapshot.id
            || self.snapshot.incarnation == 0
            || self.snapshot.incarnation > next_id
            || self.snapshot.bytes != self.identity.bytes
            || !self.snapshot.durable
        {
            return Err("invalid managed snapshot identity".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Ledger {
    pub version: u32,
    pub epoch_high_water: u64,
    pub next_id: u64,
    pub records: BTreeMap<String, Record>,
    pub pending: BTreeMap<u64, Pending>,
    pub heads: BTreeMap<String, Vec<String>>,
    pub abandoned: BTreeMap<u64, Pending>,
    pub tombstones: BTreeMap<String, Record>,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            version: 1,
            epoch_high_water: 0,
            next_id: 0,
            records: BTreeMap::new(),
            pending: BTreeMap::new(),
            heads: BTreeMap::new(),
            abandoned: BTreeMap::new(),
            tombstones: BTreeMap::new(),
        }
    }
}

pub fn valid_label(label: &str) -> Result<(), String> {
    if label.is_empty() || label.len() > 256 || label.chars().any(char::is_control) {
        return Err("ownership and identity labels must be nonempty, bounded and contain no control characters".into());
    }
    Ok(())
}

fn valid_owner(owner: &Owner, next_id: u64) -> Result<(), String> {
    valid_label(&owner.session_id)?;
    valid_label(&owner.branch_id)?;
    if owner.generation == 0 || owner.generation > next_id {
        return Err("invalid managed owner generation".into());
    }
    Ok(())
}

fn owner_key(owner: &Owner, model: &str, layout: &str) -> String {
    // Tuple encoding cannot collide when labels contain separators or Unicode.
    serde_json::to_string(&(&owner.session_id, &owner.branch_id, model, layout))
        .expect("string tuple serialization")
}

pub fn file_id(incarnation: u64) -> String {
    format!("cache-{incarnation:016x}.bin")
}

impl Ledger {
    pub fn allocate(&mut self) -> Result<u64, String> {
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or("cache identifier exhausted")?;
        Ok(self.next_id)
    }

    pub fn bytes(&self) -> Result<u64, String> {
        self.records
            .values()
            .map(|r| r.snapshot.bytes)
            .chain(self.pending.values().map(|p| p.reserved_bytes))
            .try_fold(0u64, |total, bytes| {
                total
                    .checked_add(bytes)
                    .ok_or_else(|| "cache size overflow".into())
            })
    }

    pub fn frontier(&self, owner: &Owner, model: &str, layout: &str) -> u64 {
        self.heads
            .get(&owner_key(owner, model, layout))
            .and_then(|ids| ids.first())
            .and_then(|id| self.records.get(id))
            .map(|record| record.producer.generation)
            .unwrap_or(0)
    }

    /// Keep two distinct acknowledged generations per branch as recurrent rollback anchors.
    pub fn acknowledge(&mut self, generation: u64) -> Result<(), String> {
        let Some(record) = self
            .records
            .values()
            .filter(|r| r.producer.generation == generation)
            .max_by_key(|r| r.snapshot.incarnation)
        else {
            return Ok(());
        };
        let owner = record.producer.clone();
        if self.frontier(
            &owner,
            &record.snapshot.model_key,
            &record.snapshot.layout_key,
        ) > generation
        {
            return Ok(());
        }
        let id = record.snapshot.id.clone();
        let key = owner_key(
            &owner,
            &record.snapshot.model_key,
            &record.snapshot.layout_key,
        );
        let heads = self.heads.entry(key).or_default();
        heads.retain(|id| self.records[id].producer.generation != generation);
        heads.insert(0, id);
        heads.truncate(2);
        Ok(())
    }

    pub fn pinned_heads(&self) -> HashSet<&str> {
        self.heads.values().flatten().map(String::as_str).collect()
    }

    pub fn retirements(&self) -> Vec<Retirement> {
        self.records
            .values()
            .flat_map(|record| {
                record.snapshot.owners.iter().flatten().filter_map(|owner| {
                    let next = self
                        .heads
                        .get(&owner_key(
                            owner,
                            &record.snapshot.model_key,
                            &record.snapshot.layout_key,
                        ))?
                        .first()?;
                    let successor = self.records.get(next)?;
                    (successor.producer.generation > owner.generation).then(|| Retirement {
                        snapshot_id: record.snapshot.id.clone(),
                        owner: owner.clone(),
                        successor_id: next.clone(),
                    })
                })
            })
            .collect()
    }

    /// Validate all managed file identifiers before they can reach filesystem operations.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("unsupported managed ledger version".into());
        }
        for (id, record) in self.records.iter().chain(self.tombstones.iter()) {
            record.validate(id, self.next_id)?;
        }
        for (ticket, pending) in self.pending.iter().chain(self.abandoned.iter()) {
            pending.validate(*ticket, self.next_id)?;
            if self.records.contains_key(&pending.id) {
                return Err("pending publication overlaps visible snapshot".into());
            }
        }
        self.validate_heads()?;
        if self
            .tombstones
            .keys()
            .any(|id| self.records.contains_key(id))
        {
            return Err("published tombstone overlap".into());
        }
        self.bytes()?;
        Ok(())
    }

    fn validate_heads(&self) -> Result<(), String> {
        for (key, ids) in &self.heads {
            if ids.len() > 2 || ids.iter().any(|id| !self.records.contains_key(id)) {
                return Err("invalid branch frontier".into());
            }
            if ids.iter().any(|id| {
                owner_key(
                    &self.records[id].producer,
                    &self.records[id].snapshot.model_key,
                    &self.records[id].snapshot.layout_key,
                ) != *key
            }) {
                return Err("branch frontier owner mismatch".into());
            }
        }
        Ok(())
    }

    /// An exclusive new process owner proves that prior in-process writer threads are gone.
    pub fn cancel_pending(&mut self) {
        for (ticket, pending) in std::mem::take(&mut self.pending) {
            self.abandoned.insert(ticket, pending);
        }
    }
}
