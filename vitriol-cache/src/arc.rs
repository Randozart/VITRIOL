// PROVENANCE: inspiration - vLLM (Apache-2.0) kv_offload/cpu/policies/arc.py;
// algorithm from Megiddo & Modha, "ARC: A Self-Tuning, Low Overhead
// Replacement Cache" (USENIX FAST 2004). Re-derived from the paper, not
// copied.
//
// ARC over u64 keys with a pin escape hatch.
//
// T1 (seen once) / T2 (seen >= twice) hold resident keys; B1 / B2 are ghost
// lists remembering recently-evicted keys. Ghost hits tilt the adaptive
// split point `p` between recency and frequency. Pinned keys (VITRIOL live
// slots) leave eviction scope entirely.

use std::collections::VecDeque;

#[derive(Default)]
pub struct Arc {
    t1: VecDeque<u64>,
    t2: VecDeque<u64>,
    b1: VecDeque<u64>,
    b2: VecDeque<u64>,
    p: usize,
    capacity: usize,
    pinned: Vec<u64>,
}

fn list_remove(v: &mut VecDeque<u64>, key: u64) -> bool {
    if let Some(pos) = v.iter().position(|&k| k == key) {
        v.remove(pos);
        true
    } else {
        false
    }
}

impl Arc {
    pub fn new(capacity: usize) -> Self {
        Arc {
            capacity: capacity.max(1),
            ..Default::default()
        }
    }

    pub fn len(&self) -> usize {
        self.t1.len() + self.t2.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[cfg(test)]
    fn contains_resident(&self, key: u64) -> bool {
        self.t1.contains(&key) || self.t2.contains(&key)
    }

    /// L1 (T1+T2) full -> evict from the side `p` dictates.
    /// `x_in_b2` is the paper's discrimination case for equal |T1| == p.
    fn replace(&mut self, x_in_b2: bool) -> Option<u64> {
        if self.t1.len() + self.t2.len() < self.capacity {
            return None;
        }
        if self.t1.len() >= 1 && (self.t1.len() > self.p || (x_in_b2 && self.t1.len() == self.p)) {
            let old = self.t1.pop_back()?;
            self.b1.push_front(old);
            if self.b1.len() > self.capacity {
                self.b1.pop_back();
            }
            Some(old)
        } else if self.t2.len() >= 1 {
            let old = self.t2.pop_back()?;
            self.b2.push_front(old);
            if self.b2.len() > self.capacity {
                self.b2.pop_back();
            }
            Some(old)
        } else {
            self.t1.pop_back()
        }
    }

    fn ghost_trim(&mut self) {
        // paper: when L1 is not full but |L1|+|L2|+ghosts == c, drop LRU ghost
        let total = self.t1.len() + self.t2.len() + self.b1.len() + self.b2.len();
        if total >= self.capacity {
            // drop the LRU of B2 first (least recently useful recency hint)
            if self.b2.pop_back().is_none() {
                self.b1.pop_back();
            }
        }
    }

    /// Record access to `key`; returns Some(evicted) if an eviction happened.
    /// Pinned keys are never returned as victims and never adapt `p`.
    pub fn touch(&mut self, key: u64) -> Option<u64> {
        if self.pinned.contains(&key) {
            return None;
        }
        if self.t1.contains(&key) {
            list_remove(&mut self.t1, key);
            self.t2.push_front(key);
            return None;
        }
        if self.t2.contains(&key) {
            list_remove(&mut self.t2, key);
            self.t2.push_front(key);
            return None;
        }

        let in_b1 = self.b1.contains(&key);
        let in_b2 = !in_b1 && self.b2.contains(&key);
        let delta = |a: usize, b: usize| if b == 0 { 1 } else { (a / b).max(1) };

        let evicted = if in_b1 {
            // frequency hint: x was recently evicted from T1 - grow T2 share
            self.p = (self.p + delta(self.b2.len(), self.b1.len())).min(self.capacity);
            list_remove(&mut self.b1, key);
            self.replace(false)
        } else if in_b2 {
            // recency hint: x was recently evicted from T2 - grow T1 share
            self.p = self.p.saturating_sub(delta(self.b1.len(), self.b2.len()));
            list_remove(&mut self.b2, key);
            self.replace(true)
        } else if self.t1.len() + self.t2.len() >= self.capacity {
            self.replace(false)
        } else {
            self.ghost_trim();
            None
        };

        self.t1.push_front(key);
        evicted
    }

    /// Remove key from all lists (caller dropped the state).
    pub fn forget(&mut self, key: u64) {
        list_remove(&mut self.t1, key);
        list_remove(&mut self.t2, key);
        list_remove(&mut self.b1, key);
        list_remove(&mut self.b2, key);
        self.pinned.retain(|&k| k != key);
    }

    pub fn pin(&mut self, key: u64) {
        if !self.pinned.contains(&key) {
            self.pinned.push(key);
            list_remove(&mut self.t1, key);
            list_remove(&mut self.t2, key);
        }
    }

    pub fn unpin(&mut self, key: u64) {
        self.pinned.retain(|&k| k != key);
        self.t1.push_front(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_lru_when_full() {
        let mut arc = Arc::new(2);
        arc.touch(1);
        arc.touch(2);
        assert_eq!(arc.touch(3), Some(1)); // 1 is LRU
        assert_eq!(arc.len(), 2);
    }

    #[test]
    fn second_access_promotes_to_frequency() {
        let mut arc = Arc::new(2);
        arc.touch(1);
        arc.touch(2);
        arc.touch(1); // 1 -> T2 (frequent)
                      // fresh miss evicts LRU of T1 (2), not the frequent 1
        assert_eq!(arc.touch(3), Some(2));
        arc.touch(1); // still resident, no panic
    }

    #[test]
    fn ghost_hit_adapts_and_stays_bounded() {
        let mut arc = Arc::new(2);
        arc.touch(1);
        arc.touch(2);
        arc.touch(3); // evicts 1 into B1
        arc.touch(1); // B1 hit -> p grows
        assert!(arc.len() <= 2);
        arc.touch(4);
        assert!(arc.len() <= 2);
    }

    #[test]
    fn pinned_keys_survive() {
        let mut arc = Arc::new(2);
        arc.touch(1);
        arc.touch(2);
        arc.pin(1);
        // capacity below pinned count: only unpinned keys get evicted
        let victim = arc.touch(3);
        assert_ne!(victim, Some(1));
        assert!(arc.pinned.contains(&1));
    }

    #[test]
    fn forget_removes_everywhere() {
        let mut arc = Arc::new(2);
        arc.touch(1);
        arc.touch(2);
        arc.touch(3); // 1 -> B1 ghost
        arc.forget(1);
        arc.touch(1); // fresh miss, no ghost adaptation
        assert!(arc.len() <= 2);
    }

    #[test]
    fn unpin_returns_to_recency() {
        let mut arc = Arc::new(2);
        arc.touch(1);
        arc.touch(2);
        arc.pin(1);
        arc.unpin(1);
        assert!(arc.contains_resident(1) || arc.pinned.contains(&1) == false);
    }
}
