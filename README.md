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

[OSS responsibility and source completion](docs/oss-source-contract.md) defines the supported package boundary and required acceptance. It does not declare unexecuted source, distribution or production checks complete.

Dependency updates: [repository policy](docs/dependency-updates.md).
Registry releases: [version tags and GitHub OIDC](docs/registry-release.md).
