# zixcel-revision

Retain immutable revisions and detect conflicting updates when an operation is retried.

## What you can do

- Bind updates to an exact revision.
- Inspect retained evidence and conflict results.

## Current scope

Revision records cannot prove that an arbitrary external effect happened exactly once.

This is the reviewed first-publication candidate for `zixcel-revision` 0.10.0. Published availability must be verified separately; this documentation does not claim the release is complete.

## Getting started

Install Rust 1.97 or newer and make the declared dependencies available. The reviewed crate uses crates.io dependencies. Run from this repository:

```sh
cargo test --locked
```

## Documentation and source

[Usage guide](docs/getting-started.md)

[Detailed documentation](docs) · [Implementation and public interfaces](src) · [Verification cases](tests) · [Contributing](CONTRIBUTING.md) · [Security reporting](SECURITY.md) · [License](LICENSE) · [Attribution notices](NOTICE)
