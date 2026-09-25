//! Per-device expert LRU. The safety-critical core.
//!
//! Everything the C++ path kept in file-scope globals lives inside one
//! `Lru` struct per device: pool, slot size/count, the single DMA stream,
//! per-slot completion events, the map AND the recency order AND the
//! pending mask. There is no integer slot fallback anywhere: missing
//! map/order agreement is an error, not slot 0.

use crate::bounce::BounceRing;
use crate::driver::{self, Diag};
use crate::store::Store;
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::sync::OnceLock;

pub const POOL_SIZE_DEFAULT: usize = 2048 * 1024 * 1024;
pub const MAX_SLOTS: usize = 65536;
const ALIGN256: usize = 256;

/// VITRIOL_SYNC_FILL=1: no side stream, no events, no bounce ring. Every
/// fill drains the device, then copies synchronously. Correctness-maximal
/// reference mode for isolating async-protocol bugs.
fn sync_fill() -> bool {
    static S: OnceLock<bool> = OnceLock::new();
    *S.get_or_init(|| std::env::var_os("VITRIOL_SYNC_FILL").is_some())
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    dev: i32,
    base: usize,
    expert: i32,
}

#[derive(Clone, Copy)]
struct SlotHandle(u32);

#[derive(Default)]
pub struct Stats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

pub struct Lru {
    dev: i32,
    pool: driver::CUdeviceptr,
    pool_size: usize,
    slot_size: usize,
    n_slots: usize,
    stream: driver::CUstream,
    events: Vec<driver::CUevent>,
    pending: Vec<bool>,
    map: HashMap<Key, SlotHandle>,
    order: VecDeque<Key>,
    bounce: Option<BounceRing>,
    stats: Stats,
}

fn pool_mb_for(dev: i32) -> usize {
    let dev_env = std::env::var(format!("VITRIOL_LRU_MB_{dev}")).ok();
    let global = std::env::var("VITRIOL_LRU_MB").ok();
    let raw = dev_env.or(global);
    raw.and_then(|v| v.parse::<usize>().ok())
        .filter(|mb| *mb >= 64)
        .map(|mb| mb * 1024 * 1024)
        .unwrap_or(POOL_SIZE_DEFAULT)
}

impl Lru {
    /// `min_expert_size` is the first slice size seen (mirrors C++ init);
    /// `max_expert_bytes` tightens the slot so later larger slices fit.
    /// `store_device` selects whether fills come from the bounce ring.
    pub fn init(
        dev: i32,
        min_expert_size: usize,
        max_expert_bytes: usize,
        store_device: bool,
    ) -> Result<Lru, String> {
        let pool_size = pool_mb_for(dev);

        let mut needed_slot = crate::align_up(min_expert_size, ALIGN256);
        if max_expert_bytes > 0 {
            needed_slot = needed_slot.max(crate::align_up(max_expert_bytes, ALIGN256));
        }
        let mut n_slots = pool_size / needed_slot;
        n_slots = n_slots.min(MAX_SLOTS).max(1);

        let stream = driver::stream_create()?;
        let pool = driver::mem_alloc(pool_size)?;
        let mut events = Vec::with_capacity(n_slots);
        for _ in 0..n_slots {
            events.push(driver::event_create()?);
        }
        let bounce = if store_device {
            Some(BounceRing::new(needed_slot)?)
        } else {
            None
        };

        if driver::diag().verbose {
            println!(
                "VITRIOL: LRU pool {} MB, {n_slots} slots x {needed_slot} bytes on GPU {dev}",
                pool_size / 1024 / 1024
            );
        }

        Ok(Lru {
            dev,
            pool,
            pool_size,
            slot_size: needed_slot,
            n_slots,
            stream,
            events,
            pending: vec![false; n_slots],
            map: HashMap::new(),
            order: VecDeque::new(),
            bounce,
            stats: Stats::default(),
        })
    }

    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    fn ptr_of(&self, slot: SlotHandle) -> driver::CUdeviceptr {
        self.pool + slot.0 as usize * self.slot_size
    }

    /// Ensure the expert slice is in VRAM; returns the device pointer or
    /// `Ok(0)` for "not cached, caller falls back" (uncached only in the
    /// non-store path when the slice exceeds the slot - same as C++).
    #[allow(clippy::too_many_arguments)]
    pub fn ensure(
        &mut self,
        tensor_base: *const c_void,
        expert_idx: i32,
        expert_data: *const c_void,
        expert_size: usize,
        compute_stream: driver::CUstream,
        store: Option<(&Store, u32)>, // (store, tid) when the fill reads the store
        store_device: bool,
    ) -> Result<driver::CUdeviceptr, String> {
        let d: Diag = driver::diag();
        let base = tensor_base as usize;
        if !store_device && (expert_data.is_null() || expert_size == 0) && base == 0 {
            return Ok(0);
        }
        if store_device && (base == 0 || expert_size == 0) {
            return Err("store H2D called with invalid geometry".to_string());
        }

        let dev = driver::ctx_device()?;
        if self.dev != dev {
            return Err(format!("ensure on dev {dev} but Lru is dev {}", self.dev));
        }
        if expert_size > self.slot_size {
            if store_device {
                return Err("store expert exceeds LRU slot".to_string());
            }
            return Ok(0);
        }

        // Synchronous reference mode: drain everything, then a plain
        // blocking copy. No second stream, no events, no eviction race.
        if sync_fill() {
            let key = Key {
                dev,
                base,
                expert: expert_idx,
            };
            if let Some(slot) = self.map.get(&key).copied() {
                self.stats.hits += 1;
                return Ok(self.ptr_of(slot));
            }
            self.stats.misses += 1;
            driver::stream_sync(compute_stream)?;
            let slot = if self.map.len() < self.n_slots {
                SlotHandle(self.map.len() as u32)
            } else {
                let evict = *self
                    .order
                    .back()
                    .ok_or("LRU order/map desync: empty order")?;
                self.order.retain(|k| *k != evict);
                let Some(s) = self.map.remove(&evict) else {
                    return Err("LRU map/order desync: victim not in map".to_string());
                };
                self.stats.evictions += 1;
                s
            };
            self.map.insert(key, slot);
            self.order.push_front(key);
            let dst = self.ptr_of(slot);
            match store {
                Some((store, tid)) => {
                    let ring = self.bounce.as_mut().ok_or("bounce ring missing")?;
                    let slice_bytes = crate::align_up(expert_size, 512);
                    let (buf, _) = ring.acquire(slice_bytes)?;
                    store.read_expert_padded(tid, expert_idx as u32, buf)?;
                    driver::memcpy_sync_h_to_d(dst, buf.as_ptr().cast(), expert_size)?;
                }
                None => {
                    driver::memcpy_sync_h_to_d(
                        dst,
                        expert_data,
                        expert_size,
                    )?;
                }
            }
            return Ok(dst);
        }

        let key = Key {
            dev,
            base,
            expert: expert_idx,
        };
        let cstream = compute_stream;

        // Cache hit.
        if let Some(slot) = self.map.get(&key).copied() {
            self.order.retain(|k| *k != key);
            self.order.push_front(key);
            self.stats.hits += 1;
            driver::stream_wait_event(cstream, self.events[slot.0 as usize])?;
            self.pending[slot.0 as usize] = true;
            return Ok(self.ptr_of(slot));
        }

        // Cache miss.
        self.stats.misses += 1;
        let slot: SlotHandle = if self.map.len() < self.n_slots {
            SlotHandle(self.map.len() as u32)
        } else {
            // Three-state victim selection (kimi-k3-in-c inspiration): skip
            // slots whose consumer matmul has not completed yet (pending),
            // then prefer slots whose fill event has completed.
            let mut evict = *self.order.back().ok_or("LRU order/map desync: empty order")?;
            for k in self.order.iter().rev().skip(1) {
                let Some(s) = self.map.get(k) else { continue };
                if s.0 as usize >= self.n_slots || self.pending[s.0 as usize] {
                    continue;
                }
                if driver::event_query_done(self.events[s.0 as usize]) {
                    evict = *k;
                    break;
                }
            }
            self.order.retain(|k| *k != evict);
            let Some(s) = self.map.remove(&evict) else {
                return Err("LRU map/order desync: victim not in map".to_string());
            };
            self.stats.evictions += 1;
            s
        };
        self.map.insert(key, slot);
        self.order.push_front(key);
        self.pending[slot.0 as usize] = true;

        let dst = self.ptr_of(slot);

        // Stage the fill.
        let mut bounce_event: Option<driver::CUevent> = None;
        let copy_source: *const c_void = if store_device {
            let (store, tid) = store.ok_or("store fill without binding")?;
            let ring = self.bounce.as_mut().ok_or("bounce ring missing")?;
            let slice_bytes = crate::align_up(expert_size, 512);
            let (buf, event) = ring.acquire(slice_bytes)?;
            store.read_expert_padded(tid, expert_idx as u32, buf)?;
            bounce_event = Some(event);
            buf.as_ptr().cast()
        } else {
            expert_data
        };

        // Overwrite ordering: prior consumer of this slot must be done.
        driver::stream_wait_event(self.stream, self.events[slot.0 as usize])?;
        driver::memcpy_h_to_d_async(dst, copy_source, expert_size, self.stream)?;
        if let Some(be) = bounce_event {
            driver::event_record(be, self.stream)?;
        }
        driver::event_record(self.events[slot.0 as usize], self.stream)?;

        if d.vram_hash {
            // Verify the DMA landed exactly the source bytes.
            driver::stream_sync(self.stream)?;
            let mut vbuf = vec![0u8; expert_size];
            driver::memcpy_sync_d_to_h(vbuf.as_mut_ptr().cast(), dst, expert_size)?;
            let sh = driver::fnv1a(unsafe {
                std::slice::from_raw_parts(copy_source.cast::<u8>(), expert_size)
            });
            let vh = driver::fnv1a(&vbuf);
            eprintln!(
                "VITRIOL_LRU_VRAM_HASH dev={dev} tensor={:p} expert={expert_idx} size={expert_size} src={sh:016x} vram={vh:016x} {}",
                tensor_base,
                if sh == vh { "OK" } else { "MISMATCH" }
            );
        }

        driver::stream_wait_event(cstream, self.events[slot.0 as usize])?;

        if d.strict {
            driver::stream_sync(self.stream)?;
        }

        Ok(dst)
    }

    /// Record compute completion for the slot behind `vram_ptr`.
    pub fn mark_compute_done(
        &mut self,
        vram_ptr: driver::CUdeviceptr,
        stream: driver::CUstream,
    ) -> Result<(), String> {
        let offset = vram_ptr.wrapping_sub(self.pool);
        if offset >= self.pool_size {
            return Ok(());
        }
        let slot = offset / self.slot_size;
        if offset % self.slot_size != 0 || slot >= self.n_slots {
            return Ok(());
        }
        driver::event_record(self.events[slot], stream)?;
        self.pending[slot] = false;
        Ok(())
    }
}

/// Process-lifetime flags resolved once (mirrors the C++ config parse).
#[derive(Clone, Copy)]
pub struct Config {
    pub store_device: bool,
}

pub fn config() -> Config {
    static C: OnceLock<Config> = OnceLock::new();
    *C.get_or_init(|| Config {
        store_device: std::env::var("VITRIOL_STORE_SOURCE")
            .map(|v| v == "store")
            .unwrap_or(true), // C++ default is device/store source
    })
}
