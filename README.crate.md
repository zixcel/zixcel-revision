# zixcel-revision

Bounded, content-addressed revision storage with compare-and-swap conflict checks, retry receipts and explicit retention. Use this crate when a service needs deterministic commit mechanics while keeping payload interpretation and authorization in the owning application.

## Install

```toml
[dependencies]
zixcel-revision = "0.10.0"
```

This dependency uses crates.io; no private registry is required.

## Capabilities

- Prepare immutable commits from caller-validated canonical payloads.
- Atomically publish content, heads and receipts through a backend.
- Retain, retire and reclaim revision objects under explicit bounds.

## Example

```rust
use zixcel_revision::{CommitStore, MemoryBackend};

let store = CommitStore::new(MemoryBackend::default());
assert!(store.current("example-domain")?.is_none());
# Ok::<(), zixcel_revision::Failure>(())
```

## Features and requirements

Default features use the in-memory backend. Enable `redb` for durable storage and `attestation` for signed evidence.

## Boundaries

Revision receipts describe this store. They cannot prove that an arbitrary external side effect occurred exactly once. The owner defines canonical payload encoding and semantic validation.

## Development

```sh
cargo fmt --all -- --check
cargo test --locked --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
```

## Documentation and license

[API documentation](https://docs.rs/zixcel-revision) · [Source](https://github.com/zixcel/zixcel-revision) · [Usage guide](https://github.com/zixcel/zixcel-revision/blob/main/docs/getting-started.md)

Apache-2.0. Retain the package LICENSE and NOTICE; see the source repository for security reporting and contribution guidelines.

## Durable backend cache settings

With `redb`, `durable::Handle::open_with_cache`, `create_with_cache` and `recover_with_cache` take an explicit cache target in bytes. The handle preserves this target across read/write access changes and retry after lock refusal. `cache_bytes()` reports the configured target; other constructors retain backend defaults. A zero cache target is accepted; internal buffering and other allocations still exist. These methods configure redb caching only; they do not limit transaction staging, allocator overhead, RSS, storage size or caller-owned data.

Explicit creation concerns physical storage only. Consumers own logical tables, schema and initialization. Existing open never creates or repairs a missing database. `tests/durable_cache.rs` checks retained values, foreign-reader writer refusal, retry and zero/finite targets. This source API is not available from an older published artifact merely because its manifest version is the same. Consumers must select the accepted matching source or wait for a reviewed release.

Run the isolated cache/reopen playground with `cargo run --locked --features redb --example durable-cache`. It creates only a temporary synthetic database.
