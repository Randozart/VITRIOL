//! Pinned bounce-slot ring used to stage store reads before the async H2D.
//!
//! Mirrors the C++ ring: 16 cudaHostAllocPortable slots per device, each with
//! a completion event recorded on the LRU stream after its H2D. Acquisition
//! round-robins and blocks on the slot's event, so a slot is never refilled
//! while a previous DMA still reads it. That ordering IS the safety argument;
//! it lives in one place here.

use crate::driver;
use std::ffi::c_void;

pub const SLOTS: usize = 16;
pub const ALIGN: usize = 512;

struct Slot {
    #[allow(dead_code)] // kept so Drop frees the allocation base
    alloc: *mut c_void,
    aligned: *mut u8,
    capacity: usize,
    event: driver::CUevent,
}

pub struct BounceRing {
    slots: Vec<Slot>,
    next: usize,
}

// A slot's memory is DMA-target while owned by an in-flight H2D; the ring
// guards reuse via its own event, never via aliasing pointers.
unsafe impl Send for BounceRing {}

impl BounceRing {
    /// `capacity_bytes` is the per-expert slice size rounded up to ALIGN.
    pub fn new(capacity_bytes: usize) -> Result<BounceRing, String> {
        let capacity = crate::align_up(capacity_bytes, ALIGN);
        let mut slots = Vec::with_capacity(SLOTS);
        for _ in 0..SLOTS {
            // One extra ALIGN so the 512-aligned interior always fits.
            let alloc = unsafe { driver::host_alloc(capacity + ALIGN)? };
            let addr = alloc as usize;
            let aligned = (addr + ALIGN - 1) / ALIGN * ALIGN;
            slots.push(Slot {
                alloc,
                aligned: aligned as *mut u8,
                capacity,
                event: driver::event_create()?,
            });
        }
        Ok(BounceRing { slots, next: 0 })
    }

    /// Block until the chosen slot's previous H2D has completed, then hand
    /// out exactly `len` bytes (len <= capacity, else error).
    pub fn acquire(&mut self, len: usize) -> Result<(&mut [u8], driver::CUevent), String> {
        if self.slots.is_empty() {
            return Err("bounce ring not initialized".to_string());
        }
        let idx = self.next % self.slots.len();
        self.next += 1;
        let slot = &mut self.slots[idx];
        if slot.capacity < len || slot.aligned.is_null() || slot.event.is_null() {
            return Err(format!(
                "bounce slot {idx} too small ({} < {len})",
                slot.capacity
            ));
        }
        driver::event_sync(slot.event)?;
        let slice =
            unsafe { std::slice::from_raw_parts_mut(slot.aligned, slot.capacity) };
        Ok((&mut slice[0..len], slot.event))
    }
}

impl Drop for BounceRing {
    fn drop(&mut self) {
        // Intentionally leak on teardown: CUDA calls after context teardown
        // are undefined. This struct lives for the whole process; the C++
        // implementation had the same lifetime via atexit guards.
        self.slots.clear();
    }
}
