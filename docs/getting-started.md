# Using zixcel-revision

Retain immutable revisions and detect conflicting updates when an operation is retried.

## Before you start

Revision records cannot prove that an arbitrary external effect happened exactly once.

## First steps

Run from the repository root:

```sh
cargo test --locked
```

## How to assess the result

- Bind updates to an exact revision.
- Inspect retained evidence and conflict results.

A passing source-level check establishes only what that check observes. Keep missing configuration, unavailable services and unverified deployment paths visible.

## Continue reading

[Repository overview](../README.md)
