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

## Native audit and RPC

Build the binaries from the repository root:

```sh
cargo build --release --locked --offline --manifest-path vitriol-core/Cargo.toml --bins
```

`vitriol cache-lifetime` selects the installed native audit binary or the local
release build. `VITRIOL_CACHE_LIFETIME_BIN` explicitly overrides that selection.
The command requires one source and accepts `--json` as the final argument:

```sh
vitriol cache-lifetime --disk-dir /path/to/engine/disk-mirror
vitriol cache-lifetime --inventory vitriol-core/testdata/cache-lifetime-inventory.json --json
```

The disk mode counts regular `cache-*.bin` files, excluding symlinks, directories
and temporary writes. It supplies unknown ownership, incomplete reference
coverage and unverified durability to the policy, so every existing anonymous
entry is retained. It does not read multi-GiB KV payloads, infer references from
mtime, or mutate files. Filesystem races can invalidate accounting; the output
is an audit rather than an atomic inventory. A disappearing/unreadable entry
fails the scan rather than silently provide a complete reference graph.

The inventory mode reads the strict schema illustrated by the fixture above.
Eligibility remains conditional on the caller's facts; a hand-written JSON
file is not an engine lifecycle acknowledgment. IDs are opaque identifiers,
not filesystem deletion paths. The command has no `--delete` mode.

`vitriol-core-rpc` accepts the same inventory as parameters:

```json
{"id":1,"method":"cache_retirement_plan","params":{"version":1,"epoch":0,"referenceCoverageComplete":false,"snapshots":[],"retirements":[]}}
```

The result is the same plan as the library/CLI. Invalid inventories return an
RPC error with no plan; existing `admit`, `probe_plan`, `parse_error` and `ping`
contracts remain intact. The sidecar remains stateless and per client process;
this change does not introduce a shared daemon or an authoritative engine
reference collector.

Run the Rust tests and lints:

```sh
cargo test --locked --offline --manifest-path vitriol-core/Cargo.toml
cargo fmt --manifest-path vitriol-core/Cargo.toml --check
cargo clippy --locked --offline --manifest-path vitriol-core/Cargo.toml --all-targets -- -D warnings
```
