//! A single cooperating engine owns a private cache namespace. FFI callers
//! serialize access; Store also holds an exclusive OS lock across processes.

mod disk;
mod ffi;
mod model;

pub use model::{Config, Grant, Publication};

use disk::Directory;
use model::{Ledger, Pending, Record, Tag};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use vitriol_core::cache_lifetime::{self, Inventory, Owner, Pin, Plan};

const EPOCH_BLOCK: u64 = 1 << 20;

pub struct Store {
    directory: Directory,
    ledger: Ledger,
    config: Config,
    tags: HashMap<u64, Tag>,
    reads: HashMap<u64, String>,
    read_next: u64,
    epoch: u64,
    epoch_limit: u64,
    faulted: bool,
}

impl Store {
    /// Acquire the namespace before recovering a previous engine's incomplete effects.
    pub fn open(path: &Path, config: Config) -> Result<Self, String> {
        config.validate()?;
        let directory = Directory::open(path)?;
        let mut ledger = directory.load()?;
        ledger.cancel_pending();
        directory.recover(&mut ledger)?;
        let epoch = ledger
            .epoch_high_water
            .checked_add(1)
            .ok_or("epoch exhausted")?;
        ledger.epoch_high_water = epoch.checked_add(EPOCH_BLOCK).ok_or("epoch exhausted")?;
        directory.save(&ledger)?;
        let epoch_limit = ledger.epoch_high_water;
        Ok(Self {
            directory,
            ledger,
            config,
            tags: HashMap::new(),
            reads: HashMap::new(),
            read_next: 0,
            epoch,
            epoch_limit,
            faulted: false,
        })
    }

    fn healthy(&self) -> Result<(), String> {
        if self.faulted {
            return Err("managed cache disabled after a storage failure".into());
        }
        Ok(())
    }

    /// Commit metadata before exposing the changed in-memory view.
    fn save(&mut self, ledger: Ledger) -> Result<(), String> {
        if let Err(error) = self.directory.save(&ledger) {
            self.faulted = true;
            return Err(error);
        }
        self.ledger = ledger;
        self.bump()
    }

    fn bump(&mut self) -> Result<(), String> {
        self.epoch = self.epoch.checked_add(1).ok_or("epoch exhausted")?;
        if self.epoch < self.epoch_limit {
            return Ok(());
        }
        let mut next = self.ledger.clone();
        next.epoch_high_water = self
            .epoch
            .checked_add(EPOCH_BLOCK)
            .ok_or("epoch exhausted")?;
        self.directory
            .save(&next)
            .inspect_err(|_| self.faulted = true)?;
        self.epoch_limit = next.epoch_high_water;
        self.ledger = next;
        Ok(())
    }

    /// Reserve a never-reused generation; a request remains pinned until finish.
    pub fn begin(&mut self, session: &str, branch: &str) -> Result<Grant, String> {
        self.healthy()?;
        model::valid_label(session)?;
        model::valid_label(branch)?;
        let mut ledger = self.ledger.clone();
        let generation = ledger.allocate()?;
        self.save(ledger)?;
        let owner = Owner {
            session_id: session.into(),
            branch_id: branch.into(),
            generation,
        };
        self.tags.insert(
            generation,
            Tag {
                owner: owner.clone(),
                finished: None,
            },
        );
        Ok(Grant { generation, owner })
    }

    /// A failed/aborted generation cannot acknowledge a new branch frontier.
    pub fn finish(&mut self, generation: u64, success: bool) -> Result<(), String> {
        self.healthy()?;
        let tag = self
            .tags
            .get_mut(&generation)
            .ok_or("unknown request generation")?;
        if let Some(previous) = tag.finished {
            if previous != success {
                return Err("conflicting request completion".into());
            }
            return Ok(());
        }
        tag.finished = Some(success);
        self.bump()?;
        if success {
            let mut ledger = self.ledger.clone();
            ledger.acknowledge(generation)?;
            self.save(ledger)?;
        }
        self.collect_if_enabled()?;
        Ok(())
    }

    /// C++ shared ownership keeps a generation registered while any prompt can still spill it.
    pub fn release_tag(&mut self, generation: u64) -> Result<(), String> {
        self.healthy()?;
        self.tags.remove(&generation);
        self.bump()
    }

    /// Allocate an invisible staging file; pending bytes count against the namespace budget.
    pub fn prepare(
        &mut self,
        generation: u64,
        model: &str,
        layout: &str,
        bytes: u64,
    ) -> Result<Publication, String> {
        self.healthy()?;
        model::valid_label(model)?;
        model::valid_label(layout)?;
        let owner = self
            .tags
            .get(&generation)
            .ok_or("unknown request generation")?
            .owner
            .clone();
        if self.ledger.frontier(&owner, model, layout) > generation {
            return Err("stale publication generation".into());
        }
        if self
            .ledger
            .bytes()?
            .checked_add(bytes)
            .ok_or("cache size overflow")?
            > self.config.max_bytes
        {
            return Err("managed cache budget full".into());
        }
        let mut ledger = self.ledger.clone();
        let ticket = ledger.allocate()?;
        let id = format!("cache-{ticket:016x}.bin");
        ledger.pending.insert(
            ticket,
            Pending::new(id.clone(), owner, model, layout, bytes),
        );
        self.save(ledger)?;
        self.directory
            .reserve_stage(ticket)
            .inspect_err(|_| self.faulted = true)?;
        Ok(Publication {
            ticket,
            id,
            path: self
                .directory
                .stage_path(ticket)
                .to_string_lossy()
                .into_owned(),
        })
    }

    /// fsync+rename and metadata acknowledgment share the same serialized publication boundary.
    pub fn commit(&mut self, ticket: u64) -> Result<String, String> {
        self.healthy()?;
        let pending = self
            .ledger
            .pending
            .get(&ticket)
            .ok_or("unknown publication ticket")?
            .clone();
        if pending.superseded(&self.ledger) {
            self.abort(ticket)?;
            return Err("stale publication fenced".into());
        }
        let identity = self.directory.staged_identity(ticket)?;
        let used = self
            .ledger
            .bytes()?
            .checked_sub(pending.reserved_bytes)
            .ok_or("invalid reservation")?;
        if used
            .checked_add(identity.bytes)
            .ok_or("cache size overflow")?
            > self.config.max_bytes
        {
            self.abort(ticket)?;
            return Err("managed cache budget full".into());
        }
        let mut intent = self.ledger.clone();
        intent.pending.get_mut(&ticket).unwrap().file_identity = Some(identity.clone());
        self.save(intent)?;
        self.directory
            .publish(ticket, &pending.id)
            .inspect_err(|_| self.faulted = true)?;
        let mut ledger = self.ledger.clone();
        ledger.pending.remove(&ticket);
        let generation = pending.owner.generation;
        let id = pending.id.clone();
        ledger
            .records
            .insert(id.clone(), Record::published(pending, ticket, identity));
        if self
            .tags
            .get(&generation)
            .is_some_and(|tag| tag.finished == Some(true))
        {
            ledger.acknowledge(generation)?;
        }
        self.save(ledger)?;
        self.collect_if_enabled()?;
        Ok(id)
    }

    /// Revoke publication rights durably before cleaning up staging bytes.
    pub fn abort(&mut self, ticket: u64) -> Result<(), String> {
        self.healthy()?;
        let mut ledger = self.ledger.clone();
        let pending = ledger
            .pending
            .remove(&ticket)
            .ok_or("unknown publication ticket")?;
        ledger.abandoned.insert(ticket, pending);
        self.save(ledger)?;
        self.directory
            .recover(&mut self.ledger)
            .inspect_err(|_| self.faulted = true)?;
        Ok(())
    }

    /// Pin before the caller opens bytes, and attach the borrowing request as an owner.
    pub fn acquire_read(&mut self, id: &str, generation: Option<u64>) -> Result<u64, String> {
        self.healthy()?;
        let record = self
            .ledger
            .records
            .get(id)
            .ok_or("snapshot no longer published")?;
        self.directory
            .verify(id, &record.identity)
            .inspect_err(|_| self.faulted = true)?;
        if let Some(generation) = generation {
            let owner = self
                .tags
                .get(&generation)
                .ok_or("unknown request generation")?
                .owner
                .clone();
            let mut ledger = self.ledger.clone();
            let owners = ledger
                .records
                .get_mut(id)
                .unwrap()
                .snapshot
                .owners
                .as_mut()
                .ok_or("unknown owners")?;
            if !owners.contains(&owner) {
                owners.push(owner);
                self.save(ledger)?;
            }
        }
        self.read_next = self
            .read_next
            .checked_add(1)
            .ok_or("read lease exhausted")?;
        self.reads.insert(self.read_next, id.into());
        self.bump()?;
        Ok(self.read_next)
    }

    /// RAM restores attach before a writer finishes publishing their backing state.
    pub fn attach(&mut self, id: &str, generation: u64) -> Result<(), String> {
        self.healthy()?;
        let owner = self
            .tags
            .get(&generation)
            .ok_or("unknown request generation")?
            .owner
            .clone();
        let mut ledger = self.ledger.clone();
        let owners = if let Some(record) = ledger.records.get_mut(id) {
            self.directory
                .verify(id, &record.identity)
                .inspect_err(|_| self.faulted = true)?;
            record.snapshot.owners.as_mut().ok_or("unknown owners")?
        } else {
            let digits = id
                .strip_prefix("cache-")
                .and_then(|s| s.strip_suffix(".bin"))
                .ok_or("invalid managed id")?;
            let ticket = u64::from_str_radix(digits, 16).map_err(|_| "invalid managed id")?;
            let pending = ledger
                .pending
                .get_mut(&ticket)
                .filter(|pending| pending.id == id)
                .ok_or("snapshot no longer available")?;
            if pending.owner == owner {
                return Ok(());
            }
            &mut pending.borrowers
        };
        if owners.contains(&owner) {
            return Ok(());
        }
        owners.push(owner);
        self.save(ledger)
    }

    pub fn release_read(&mut self, lease: u64) -> Result<(), String> {
        self.healthy()?;
        if self.reads.remove(&lease).is_none() {
            return Err("unknown read lease".into());
        }
        self.bump()?;
        Ok(())
    }

    /// Derive policy facts exclusively from the owner's ledger and current process leases.
    pub fn inventory(&self) -> Result<Inventory, String> {
        self.healthy()?;
        let mut snapshots: Vec<_> = self
            .ledger
            .records
            .values()
            .map(|r| r.snapshot.clone())
            .collect();
        let mut reads = HashMap::<&str, u64>::new();
        for id in self.reads.values() {
            *reads.entry(id).or_default() += 1;
        }
        let pinned = self.ledger.pinned_heads();
        for snapshot in &mut snapshots {
            snapshot.readers = reads.get(snapshot.id.as_str()).copied().unwrap_or(0);
            if pinned.contains(snapshot.id.as_str()) {
                snapshot.pins.push(Pin::RollbackAnchor);
            }
            let active = snapshot.owners.iter().flatten().any(|owner| {
                self.tags
                    .get(&owner.generation)
                    .is_some_and(|tag| tag.finished.is_none())
            });
            if active {
                snapshot.pins.push(Pin::Branch);
            }
        }
        let retirements = self.ledger.retirements();
        Ok(Inventory {
            version: 1,
            epoch: self.epoch,
            reference_coverage_complete: true,
            snapshots,
            retirements,
        })
    }

    pub fn audit(&self) -> Result<Plan, String> {
        cache_lifetime::plan(&self.inventory()?)
    }

    pub fn pin(&mut self, id: &str, pin: Pin, enabled: bool) -> Result<(), String> {
        self.healthy()?;
        let mut ledger = self.ledger.clone();
        let pins = &mut ledger
            .records
            .get_mut(id)
            .ok_or("snapshot no longer published")?
            .snapshot
            .pins;
        pins.retain(|existing| *existing != pin);
        if enabled {
            pins.push(pin);
        }
        self.save(ledger)
    }

    /// Recheck under the owner's lock; tombstone before unlink so restart cannot expose retired bytes.
    pub fn collect(&mut self, expected_epoch: u64, dry_run: bool) -> Result<Value, String> {
        self.healthy()?;
        if self.epoch != expected_epoch {
            return Err("stale inventory epoch".into());
        }
        let plan = self.audit()?;
        if dry_run || !self.config.reclaim {
            return Ok(json!({"removed":0,"bytes":0,"plan":plan}));
        }
        let mut ledger = self.ledger.clone();
        let mut bytes = 0u64;
        let mut count = 0usize;
        for decision in plan.decisions.iter().filter(|d| d.eligible) {
            if count >= self.config.max_gc_files {
                break;
            }
            let record = ledger
                .records
                .get(&decision.id)
                .ok_or("snapshot disappeared")?;
            if record.snapshot.incarnation != decision.incarnation {
                return Err("snapshot incarnation changed".into());
            }
            let Some(next_bytes) = bytes
                .checked_add(record.snapshot.bytes)
                .filter(|n| *n <= self.config.max_gc_bytes)
            else {
                continue;
            };
            self.directory
                .verify(&decision.id, &record.identity)
                .inspect_err(|_| self.faulted = true)?;
            let record = ledger.records.remove(&decision.id).unwrap();
            ledger.tombstones.insert(decision.id.clone(), record);
            bytes = next_bytes;
            count += 1;
        }
        if count == 0 {
            return Ok(json!({"removed":0,"bytes":0}));
        }
        self.save(ledger)?;
        self.directory
            .recover(&mut self.ledger)
            .inspect_err(|_| self.faulted = true)?;
        Ok(json!({"removed":count,"bytes":bytes}))
    }

    fn collect_if_enabled(&mut self) -> Result<(), String> {
        if self.config.reclaim {
            self.collect(self.epoch, false)?;
        }
        Ok(())
    }

    pub fn list(&self) -> Result<Value, String> {
        let audit = self.audit()?;
        Ok(
            json!({"epoch":self.epoch,"directory":self.directory.path(),"audit":audit,
            "reclamationEnabled":self.config.reclaim,"limits":self.config,
            "snapshots":self.ledger.records.values().map(|r| &r.snapshot).collect::<Vec<_>>() }),
        )
    }
}
