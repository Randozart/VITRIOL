"""Round-2 compile-gate problems: 6 hard single-file Rust, std-only,
self-testing via main() asserts. Green = rustc + run + clippy clean."""

COMMON = (
    "Requirements:\n"
    "- ONE complete single-file Rust program, exactly one ```rust code block, no prose after it.\n"
    "- No external crates; std only; edition 2021; must compile with rustc and run cleanly.\n"
    "- fn main() must run self-tests with asserts that actually verify correctness (not trivial).\n"
    "- If unsafe is used, keep it minimal and justify each block in a comment.\n"
)

PROBLEMS = [
    ("treiber-stack",
     "Implement a lock-free Treiber stack<T: Send + 'static> using std::sync::atomic::AtomicPtr "
     "with push and pop. Then in main: 4 threads each push 10,000 items concurrently (with occasional "
     "yield_now to force contention), then join and pop everything, asserting the total count is 40,000 "
     "and no item is lost. Explain in a comment what the ABA hazard is for this structure and which "
     "aspect of the Node allocation strategy mitigates it here."),
    ("spsc-ring",
     "Implement a bounded single-producer single-consumer ring buffer of u64 with power-of-two capacity, "
     "using std::sync::atomic with correct acquire/release memory orderings (head owned by producer, tail "
     "owned by consumer; relaxed loads of own index). Use cache-line padding between indices. In main: "
     "spawn a producer pushing 1,000,000 sequential values and a consumer popping them, assert all values "
     "arrive in exact order and no value is duplicated or dropped."),
    ("handle-arena",
     "Implement a typed-handle arena: Arena<T> that hands out Handle<T> = {index: u32, generation: u32}; "
     "freeing a slot bumps its generation so a stale handle's get() returns None (no use-after-free). "
     "Support alloc, free, get(&self, h) -> Option<&T>, get_mut. No unsafe allowed. In main: allocate 1000 "
     "values, free every 3rd, assert stale handles from before the free return None after, and that "
     "re-allocated slots invalidate the old generation's handles. Also support iteration over live values."),
    ("tlv-parser",
     "Implement a zero-copy TLV binary format parser over &[u8] with lifetime-borrowed output. Format: "
     "each node = tag:u8, length: varint (LEB128, up to 5 bytes), value: [u8; length]. Tag 0x01 = primitive "
     "(value is payload bytes), tag 0x02 = container whose value is itself a sequence of TLV nodes (nesting "
     "allowed, depth >= 3 in tests). Parse into a borrowed tree (Node<'a>). Must REJECT: truncated input, "
     "length exceeding remaining bytes, varints longer than 5 bytes or non-minimal. In main: build a nested "
     "message, encode to bytes, parse and assert equality of all leaves; then feed 6+ malformed inputs and "
     "assert each is rejected with an error (no panics)."),
    ("rbtree",
     "Implement a Red-Black tree (keys: u64, values: String) with insert and delete, iterative or recursive, "
     "with a parent-pointer or explicit-stack approach that handles all rebalancing cases. In main: insert "
     "10,000 pseudo-random keys (deterministic LCG, no rand crate), interleaved with 4,000 deletes; verify "
     "against a BTreeMap oracle that lookups match; then verify tree invariants directly: BST order via "
     "in-order walk, no red-red parent/child, equal black height on all root-to-leaf paths, root is black. "
     "Assert all invariants hold after every 2,000 operations."),
    ("mini-executor",
     "Implement a single-threaded mini-executor for std::future::Future without tokio or block_on: a "
     "Spawner/Executor pair where tasks are BoxFuture-like (Pin<Box<dyn Future<Output = ()> + 'static>>), "
     "a manual Waker built from RawWaker/Arc-based wakeups that push the task into a ready VecDeque, and an "
     "run-until-idle loop that polls ready tasks. Include at least 3 distinct task types (e.g. a yield-based "
     "loop that completes after N wakeups, a cascading task that wakes another task, a no-op immediate) "
     "running concurrently in main; assert all complete, assert the cascade fired, and print a completion "
     "count. No unsafe except the RawWaker vtable plumbing."),
]
