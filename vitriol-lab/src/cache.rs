// PROVENANCE: inspiration — none; VITRIOL-original bounded whole-snapshot replay.
use crate::model::{reusable, Config, Eviction, Request, Trace};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Cold,
    Slot,
    Ram,
    Disk,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tier {
    Ram,
    Disk,
}

struct Entry {
    tier: Tier,
    bytes: u64,
    protected: bool,
    readers: u64,
    touched: u64,
    work: u64,
}

#[derive(Default, Serialize)]
pub struct Stats {
    pub demoted: u64,
    pub evicted: u64,
    pub skipped: u64,
    pub peak_ram_bytes: u64,
    pub peak_disk_bytes: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct Choice {
    pub source: Source,
    pub cached: usize,
    pub restore: u64,
    pub work: u64,
    pub entry: Option<usize>,
}

impl Choice {
    pub fn new(
        source: Source,
        cached: usize,
        restore: u64,
        entry: Option<usize>,
        request: &Request,
    ) -> Result<Self, String> {
        let work = ((request.prefix_ids.len() - cached) as u64)
            .checked_mul(request.prefill_us_per_token)
            .and_then(|v| v.checked_add(restore))
            .ok_or("cost overflow")?;
        Ok(Self {
            source,
            cached,
            restore,
            work,
            entry,
        })
    }

    pub fn better_than(&self, other: &Self) -> bool {
        (self.work, self.restore, std::cmp::Reverse(self.cached))
            < (other.work, other.restore, std::cmp::Reverse(other.cached))
    }
}

#[derive(Default)]
pub(crate) struct Cache {
    entries: BTreeMap<usize, Entry>,
    pub stats: Stats,
}

impl Cache {
    pub fn choose(
        &self,
        trace: &Trace,
        request: &Request,
        cfg: &Config,
        mut best: Choice,
    ) -> Result<Choice, String> {
        for (id, entry) in &self.entries {
            let cached = reusable(&trace.requests[*id], request, cfg.rewind_tokens);
            if cached == 0 {
                continue;
            }
            let (source, restore) = match entry.tier {
                Tier::Ram => (Source::Ram, request.ram_restore_us),
                Tier::Disk => (Source::Disk, request.disk_restore_us),
            };
            let choice = Choice::new(source, cached, restore, Some(*id), request)?;
            if choice.better_than(&best) {
                best = choice;
            }
        }
        Ok(best)
    }

    pub fn acquire(&mut self, id: Option<usize>, now: u64) {
        if let Some(entry) = id.and_then(|id| self.entries.get_mut(&id)) {
            entry.readers += 1;
            entry.touched = now;
        }
    }

    pub fn release(&mut self, id: Option<usize>) {
        if let Some(entry) = id.and_then(|id| self.entries.get_mut(&id)) {
            entry.readers -= 1;
        }
    }

    fn used(&self, tier: Tier) -> u64 {
        self.entries
            .values()
            .filter(|e| e.tier == tier)
            .map(|e| e.bytes)
            .sum()
    }

    fn victim(&self, tier: Option<Tier>, policy: Eviction) -> Option<usize> {
        self.entries
            .iter()
            .filter(|(_, e)| tier.is_none_or(|t| e.tier == t) && !e.protected && e.readers == 0)
            .min_by(|(a, x), (b, y)| {
                let cost = match policy {
                    Eviction::Lru => std::cmp::Ordering::Equal,
                    Eviction::Value => (u128::from(x.work) * u128::from(y.bytes))
                        .cmp(&(u128::from(y.work) * u128::from(x.bytes))),
                };
                cost.then(x.touched.cmp(&y.touched)).then(a.cmp(b))
            })
            .map(|(id, _)| *id)
    }

    fn disk_room(&mut self, bytes: u64, cfg: &Config) -> bool {
        if bytes > cfg.disk_bytes {
            return false;
        }
        while self.used(Tier::Disk) > cfg.disk_bytes - bytes {
            let Some(id) = self.victim(Some(Tier::Disk), cfg.eviction) else {
                return false;
            };
            self.entries.remove(&id);
            self.stats.evicted += 1;
        }
        true
    }

    fn ram_room(&mut self, bytes: u64, cfg: &Config) -> bool {
        if bytes > cfg.ram_bytes {
            return false;
        }
        while self.used(Tier::Ram) > cfg.ram_bytes - bytes {
            let Some(id) = self.victim(Some(Tier::Ram), cfg.eviction) else {
                return false;
            };
            let mut entry = self.entries.remove(&id).unwrap();
            if self.disk_room(entry.bytes, cfg) {
                entry.tier = Tier::Disk;
                self.entries.insert(id, entry);
                self.stats.demoted += 1;
                self.stats.peak_disk_bytes = self.stats.peak_disk_bytes.max(self.used(Tier::Disk));
            } else {
                self.stats.evicted += 1;
            }
        }
        true
    }

    pub fn admit(
        &mut self,
        id: usize,
        request: &Request,
        now: u64,
        cfg: &Config,
    ) -> Result<(), String> {
        if request.snapshot_bytes == 0 {
            return Ok(());
        }
        if request.snapshot_bytes > cfg.ram_bytes.max(cfg.disk_bytes) {
            self.stats.skipped += 1;
            return Ok(());
        }
        if !self.entry_room(cfg) {
            self.stats.skipped += 1;
            return Ok(());
        }
        let tier = if self.ram_room(request.snapshot_bytes, cfg) {
            Tier::Ram
        } else if self.disk_room(request.snapshot_bytes, cfg) {
            Tier::Disk
        } else {
            self.stats.skipped += 1;
            return Ok(());
        };
        let work = (request.state_prefix_ids.len() as u64)
            .checked_mul(request.prefill_us_per_token)
            .ok_or("snapshot work overflow")?;
        self.entries.insert(
            id,
            Entry {
                tier,
                bytes: request.snapshot_bytes,
                protected: request.protected,
                readers: 0,
                touched: now,
                work,
            },
        );
        self.stats.peak_ram_bytes = self.stats.peak_ram_bytes.max(self.used(Tier::Ram));
        self.stats.peak_disk_bytes = self.stats.peak_disk_bytes.max(self.used(Tier::Disk));
        Ok(())
    }

    fn entry_room(&mut self, cfg: &Config) -> bool {
        if self.entries.len() < cfg.max_entries {
            return true;
        }
        let Some(id) = self.victim(None, cfg.eviction) else {
            return false;
        };
        self.entries.remove(&id);
        self.stats.evicted += 1;
        true
    }
}
