// PROVENANCE: inspiration - SGLang/RadixAttention (Apache-2.0, arXiv:2312.07104)
// prefix-tree reuse shape and vLLM (Apache-2.0) chained content-hash block
// pool; re-derived from the papers, not copied.
//
// Radix trie over token sequences with caller-owned value handles.
//
// Design contract (stack-pull S5): this crate is the lookup/eviction POLICY
// brain for the VITRIOL prompt cache. It owns no KV bytes - the caller maps
// each `value` handle (an opaque u64) to its own state. The tree indexes
// token sequences so that
//
//   insert(&[t0..tN], v)   registers v as the state for that exact sequence
//   match_prefix(&[u0..])  returns the deepest registered ancestor state
//
// which is exactly the question the C++ side asks per request: "which
// cached state shares the longest prefix with this prompt?" Branching
// conversations (subagents, retries, session forks) then hit the shared
// ancestor instead of re-prefilling from zero.
//
// Matching walks the trie token-by-token via per-node first-token child
// maps: O(depth) hash lookups, no scanning. Nodes track last_hit for the
// eviction walk; pinned nodes (live slots) are never evicted.

use std::collections::HashMap;

#[derive(Debug, Default)]
struct Node {
    // tokens leading TO this node from the parent (radix edge label)
    edge: Vec<u64>,
    parent: Option<usize>,
    children: HashMap<u64, usize>,
    // registered state handle; 0 = structural node only (u64::MAX reserved)
    value: u64,
    last_hit: u64, // logical clock at last match/insert/touch
    pinned: bool,
}

pub struct RadixTree {
    nodes: Vec<Node>,
    // free-list of tombstoned node slots
    free: Vec<usize>,
    clock: u64,
    len: usize,
}

/// Sentinel for "no value" - callers must never register this handle.
pub const NO_VALUE: u64 = u64::MAX;

impl RadixTree {
    pub fn new() -> Self {
        let root = Node {
            edge: Vec::new(),
            parent: None,
            children: HashMap::new(),
            value: NO_VALUE,
            last_hit: 0,
            pinned: false,
        };
        RadixTree {
            nodes: vec![root],
            free: Vec::new(),
            clock: 0,
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn alloc_node(&mut self, edge: Vec<u64>, parent: Option<usize>) -> usize {
        let idx = match self.free.pop() {
            Some(i) => {
                self.nodes[i] = Node {
                    edge,
                    parent,
                    children: HashMap::new(),
                    value: NO_VALUE,
                    last_hit: self.clock,
                    pinned: false,
                };
                i
            }
            None => {
                self.nodes.push(Node {
                    edge,
                    parent,
                    children: HashMap::new(),
                    value: NO_VALUE,
                    last_hit: self.clock,
                    pinned: false,
                });
                self.nodes.len() - 1
            }
        };
        idx
    }

    fn unlink(&mut self, idx: usize) {
        // remove node from parent; recurse upward through valueless,
        // childless, unpinned ancestors
        let mut cur = idx;
        loop {
            let (parent, first_token) = {
                let node = &self.nodes[cur];
                match node.parent {
                    Some(p) => (p, *node.edge.first().unwrap_or(&0)),
                    None => return, // root
                }
            };
            let is_empty = self.nodes[cur].children.is_empty() && self.nodes[cur].value == NO_VALUE;
            if is_empty && !self.nodes[cur].pinned {
                self.nodes[parent].children.remove(&first_token);
                self.free.push(cur);
                cur = parent;
            } else {
                return;
            }
        }
    }

    /// Register `value` for the exact token sequence. Replaces any previous
    /// registration of the same sequence (returns the old value).
    pub fn insert(&mut self, tokens: &[u64], value: u64) -> Option<u64> {
        assert_ne!(value, NO_VALUE, "NO_VALUE is reserved");
        if tokens.is_empty() {
            return None;
        }
        self.clock = self.clock.wrapping_add(1);

        let mut cur = 0usize; // root
        let mut i = 0usize;

        while i < tokens.len() {
            let tok = tokens[i];
            let child = self.nodes[cur].children.get(&tok).copied();
            match child {
                None => {
                    // new edge with the remaining tokens
                    let node = self.alloc_node(tokens[i..].to_vec(), Some(cur));
                    self.nodes[cur].children.insert(tok, node);
                    self.nodes[node].value = value;
                    self.nodes[node].last_hit = self.clock;
                    self.len += 1;
                    return None;
                }
                Some(c) => {
                    let edge = self.nodes[c].edge.clone();
                    let common = edge
                        .iter()
                        .zip(&tokens[i..])
                        .take_while(|(a, b)| a == b)
                        .count();

                    if common < edge.len() {
                        // split edge at `common`: intermediate structural node
                        let tail: Vec<u64> = edge[common..].to_vec();
                        let split = self.alloc_node(edge[..common].to_vec(), Some(cur));
                        let old_first = edge[common];
                        // re-parent the existing child under the split node
                        self.nodes[c].edge = tail;
                        self.nodes[c].parent = Some(split);
                        self.nodes[split].children.insert(old_first, c);
                        self.nodes[cur].children.insert(tok, split);
                        cur = split;
                        i += common;
                        continue;
                    }

                    cur = c;
                    i += common;

                    if i >= tokens.len() {
                        // sequence ends exactly at this node
                        let old = if self.nodes[cur].value == NO_VALUE {
                            self.len += 1;
                            None
                        } else {
                            Some(self.nodes[cur].value)
                        };
                        self.nodes[cur].value = value;
                        self.nodes[cur].last_hit = self.clock;
                        return old;
                    }
                }
            }
        }
        None
    }

    /// Deepest registered ancestor: returns (value, matched_token_depth).
    pub fn match_prefix(&mut self, tokens: &[u64]) -> Option<(u64, usize)> {
        self.clock = self.clock.wrapping_add(1);
        let mut cur = 0usize;
        let mut depth = 0usize;
        let mut best: Option<(u64, usize)> = None;

        let mut i = 0usize;
        while i < tokens.len() {
            let tok = tokens[i];
            let child = match self.nodes[cur].children.get(&tok).copied() {
                Some(c) => c,
                None => break,
            };
            let edge = self.nodes[child].edge.as_slice();
            let common = edge
                .iter()
                .zip(&tokens[i..])
                .take_while(|(a, b)| a == b)
                .count();
            // a partial edge match means the sequence diverges mid-edge
            if common < edge.len() {
                break;
            }
            cur = child;
            depth += edge.len();
            i += edge.len();
            if self.nodes[cur].value != NO_VALUE {
                self.nodes[cur].last_hit = self.clock;
                best = Some((self.nodes[cur].value, depth));
            }
        }
        best
    }

    /// Refresh recency for a value without a token walk.
    pub fn touch(&mut self, value: u64) {
        self.clock = self.clock.wrapping_add(1);
        let clock = self.clock;
        if let Some(node) = self.nodes.iter_mut().find(|n| n.value == value) {
            node.last_hit = clock;
        }
    }

    pub fn pin(&mut self, value: u64) -> bool {
        match self.nodes.iter_mut().find(|n| n.value == value) {
            Some(n) => {
                n.pinned = true;
                true
            }
            None => false,
        }
    }

    pub fn unpin(&mut self, value: u64) {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.value == value) {
            n.pinned = false;
        }
    }

    /// Evict least-recently-hit unpinned registrations until at most
    /// `max_entries` remain. Returns the evicted value handles (callers
    /// must drop their own state bytes; the tree forgets the sequence).
    pub fn evict_lru(&mut self, max_entries: usize) -> Vec<u64> {
        let mut evicted = Vec::new();
        while self.len > max_entries {
            // weakest = smallest last_hit among value-carrying unpinned nodes
            let victim = self
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.value != NO_VALUE && !n.pinned)
                .min_by_key(|(_, n)| n.last_hit)
                .map(|(i, _)| i);

            let v = match victim {
                Some(i) => i,
                None => break, // everything pinned
            };

            let value = self.nodes[v].value;
            self.nodes[v].value = NO_VALUE;
            self.len -= 1;
            evicted.push(value);
            self.unlink(v);
        }
        evicted
    }

    /// Remove a specific registration (e.g. caller dropped the state).
    pub fn remove(&mut self, value: u64) -> bool {
        let idx = self
            .nodes
            .iter()
            .position(|n| n.value == value);
        match idx {
            Some(i) => {
                self.nodes[i].value = NO_VALUE;
                self.len -= 1;
                self.unlink(i);
                true
            }
            None => false,
        }
    }
}

impl Default for RadixTree {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(v: &[u64]) -> Vec<u64> {
        v.to_vec()
    }

    #[test]
    fn insert_and_exact_match() {
        let mut t = RadixTree::new();
        assert_eq!(t.insert(&toks(&[1, 2, 3, 4]), 100), None);
        assert_eq!(t.match_prefix(&toks(&[1, 2, 3, 4])), Some((100, 4)));
        assert_eq!(t.match_prefix(&toks(&[1, 2, 3])), None); // structural only
        assert_eq!(t.match_prefix(&toks(&[9, 9])), None);
    }

    #[test]
    fn deepest_ancestor_wins() {
        let mut t = RadixTree::new();
        t.insert(&toks(&[1, 2, 3, 4, 5, 6]), 1);
        t.insert(&toks(&[1, 2, 3]), 2);
        assert_eq!(t.match_prefix(&toks(&[1, 2, 3, 4, 5, 6, 7])), Some((1, 6)));
        // shortening the branch flips the best match
        t.remove(1);
        assert_eq!(t.match_prefix(&toks(&[1, 2, 3, 4, 5, 6, 7])), None);
    }

    #[test]
    fn edge_split() {
        let mut t = RadixTree::new();
        t.insert(&toks(&[1, 2, 3, 4, 5]), 1);
        // branch at token 3
        t.insert(&toks(&[1, 2, 3, 9, 9]), 2);
        assert_eq!(t.match_prefix(&toks(&[1, 2, 3, 4, 5])), Some((1, 5)));
        assert_eq!(t.match_prefix(&toks(&[1, 2, 3, 9, 9])), Some((2, 5)));
        assert_eq!(t.match_prefix(&toks(&[1, 2, 3])), None);
        // both still reachable after the split
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn insert_replaces_same_sequence() {
        let mut t = RadixTree::new();
        t.insert(&toks(&[1, 2, 3]), 10);
        assert_eq!(t.insert(&toks(&[1, 2, 3]), 11), Some(10));
        assert_eq!(t.len(), 1);
        assert_eq!(t.match_prefix(&toks(&[1, 2, 3])), Some((11, 3)));
    }

    #[test]
    fn evict_respects_pinned_and_recency() {
        let mut t = RadixTree::new();
        t.insert(&toks(&[1]), 1);
        t.insert(&toks(&[2]), 2);
        t.insert(&toks(&[3]), 3);
        t.pin(2);
        t.match_prefix(&toks(&[3])); // touch 3
        // cap 2: must evict 1 (never hit), keep 2 (pinned) and 3 (recent)
        let evicted = t.evict_lru(2);
        assert_eq!(evicted, vec![1]);
        assert_eq!(t.match_prefix(&toks(&[2])), Some((2, 1)));
        assert_eq!(t.match_prefix(&toks(&[3])), Some((3, 1)));
        // cap below pinned count: evict stops
        let evicted = t.evict_lru(0);
        assert_eq!(evicted, vec![3]); // 3 is unpinned
        assert_eq!(t.evict_lru(0).is_empty(), true); // 2 pinned, refuses
    }

    #[test]
    fn remove_prunes_dead_branches() {
        let mut t = RadixTree::new();
        t.insert(&toks(&[1, 2, 3, 4]), 1);
        t.remove(1);
        assert_eq!(t.len(), 0);
        assert_eq!(t.match_prefix(&toks(&[1, 2, 3, 4])), None);
        // structural nodes are reclaimed
        assert!(t.nodes.len() < 8);
    }

    #[test]
    fn shared_prefix_branch_fork() {
        // subagent fork: long shared system prefix, two branches
        let mut t = RadixTree::new();
        let sys: Vec<u64> = (10..100).collect();
        let mut a = sys.clone();
        a.extend([7, 7, 7]);
        let mut b = sys.clone();
        b.extend([9, 9]);
        t.insert(&a, 100);
        t.insert(&b, 200);
        // a new request continuing branch b finds it fully
        assert_eq!(t.match_prefix(&b), Some((200, b.len() as _)));
        // a request that only shares the system prefix finds nothing yet,
        // but a later insert of the shared prefix wins deepest-ancestor
        let mut c = sys.clone();
        c.extend([5]);
        t.insert(&sys, 300);
        assert_eq!(t.match_prefix(&c), Some((300, sys.len() as _)));
    }
}
