# VITRIOL coordination core

Rust policy shared with Officina through the `vitriol-core-rpc` stdio sidecar.
Existing admission/probe decisions retain their TypeScript parity vectors.

## Cache lifetime

`cache_lifetime::plan` consumes a versioned inventory with an epoch, immutable
snapshot IDs/incarnations, model/layout identity, owner generations, explicit
retirements and protection facts. It returns per-snapshot reasons and byte
accounting. It performs no filesystem or network I/O.

A later session head alone never retires a snapshot. Every owner of a shared
snapshot must explicitly release its exact session/branch/generation reference
to a durable newer snapshot for that same branch and engine identity. Active
readers, pending writes, branch/rewind pins and recurrent rollback anchors retain
the old snapshot. Replacement targets are retained within the same plan.

Unknown ownership and incomplete reference coverage retain data. Missing
protection fields, unknown schema versions, duplicate identities, dangling
retirements and accounting overflow reject the inventory rather than infer safety.
Owner-reference indexes avoid scanning every shared owner for every retirement.

The result always has `deletionAuthorized: false`: eligibility is conditional on
the supplied lifecycle facts. A future deletion executor must obtain those facts
from an authoritative owner, revalidate epoch/incarnation under synchronization,
and prevent late writers from republishing retired generations. The current
engine's anonymous disk mirror does not yet provide that ownership protocol.
This library does not claim to prove an entry can never match a future prompt.

Run the Rust tests and lints:

```sh
cargo test --locked --offline --manifest-path vitriol-core/Cargo.toml
cargo fmt --manifest-path vitriol-core/Cargo.toml --check
cargo clippy --locked --offline --manifest-path vitriol-core/Cargo.toml --all-targets -- -D warnings
```
