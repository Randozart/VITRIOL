# Engine-owned KV store

This crate adds POSIX filesystem effects around the pure `vitriol-core` cache
lifetime policy. It owns a private directory, an exclusive OS lock and an
atomic/fsynced metadata ledger. The C ABI serializes access through one mutex;
Rust callers use exclusive mutable access. No KV bytes pass through JSON/FFI.

`begin` grants a persisted monotonic generation for one session/branch. `prepare`
reserves budget and a unique staging directory. The engine writes opaque state
there, then `commit` fsyncs the bytes, publishes a non-overwriting hard link and
durably records its identity. Successful `finish` acknowledges a branch frontier;
failed requests do not. Two distinct durable generations per branch remain
rollback anchors. Shared readers add their own ownership references, and active
requests/file leases/explicit pins prevent reclamation.

Old writers are rejected once a newer acknowledged snapshot supersedes their
generation. Aborting a ticket revokes publication rights before removing its
staging directory, so a delayed writer cannot recreate its path. Identifiers
are never reused. Persisted epoch ranges prevent an old process's audit epoch
from being mistaken for a new owner's epoch.

Reclamation defaults to dry-run and requires explicit positive file/byte bounds.
Under the owner's lock it rechecks epoch and physical identity, persists
tombstones, unlinks files and durably clears tombstones. Restart obtains the OS
lock before recovering abandoned tickets/tombstones. It cannot delete unknown
incarnations. Storage failures disable managed effects; they do not authorize a
new empty ledger. Existing anonymous mirror files are outside this namespace.

The protocol assumes a cooperating engine, exclusive directory ownership and
local POSIX fsync/rename/link semantics. It does not defend against a privileged
actor replacing the directory/lock underneath the running process or claim that
any retired bytes could never match an arbitrary future prompt. Eligibility is
about explicit current ownership and the documented retention contract.

```sh
cargo test --locked --offline --manifest-path vitriol-cache-store/Cargo.toml
cargo clippy --locked --offline --manifest-path vitriol-cache-store/Cargo.toml --all-targets -- -D warnings
```

`include/vitriol-cache-store.h` declares opaque handle/string ownership. Open
receives `{ "path": "...", "config": { "maxBytes": 1024, "reclaim": false,
"maxGcFiles": 1, "maxGcBytes": 1024 } }`. Calls use an `op`-tagged JSON object;
responses are `{ "ok": true, "result": ... }` or `{ "ok": false, "error": ... }`.
Callers must keep the handle alive through all calls and free returned strings
with the matching library function. Invalid commands return errors; panics cannot
unwind across C. A poisoned mutex rejects further operations.

The finite P model covers a serialized owner, a reader, a newer durable publisher
and a delayed old writer. `tcSerialized` checks lease/deletion and late-publisher
safety; `tcMissingFence` is a deliberately faulty negative control. Sampling is
not exhaustive proof. Filesystem crash recovery, multiple owners/branches,
durability failures and FFI conformance are covered separately by implementation
tests or remain explicit platform assumptions.
