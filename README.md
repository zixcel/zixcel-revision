# Zixcel Revision

Generic bounded immutable content, per-domain head CAS, exact operation replay
and external retention. Callers supply domain-specific payloads and execution
policy through the package interface.

`CommitRef` identifies the content-addressed revision envelope; `RevisionRef`
combines that identity and its ordered stream position. Domain is the stream key.
Owners validate payloads and authority before publication. redb is optional;
backends must atomically publish content, head and receipt or change none.

Reads never initialize or recover. External references do not copy external
payloads and retention alone does not guarantee those bytes exist. Explicit owner
reclamation is fenced with non-reused registration generations and durable permits.
Committed receipts remain bounded roots; this does not promise unlimited history.

## License

Apache-2.0. Copyright 2026 HAT Inc. See [LICENSE](LICENSE) and [NOTICE](NOTICE). External dependencies retain their respective licenses.

## Package integration

The package is an independently consumable unit. Callers reference its documented
interface through a versioned dependency and own application-specific composition
and integration.
