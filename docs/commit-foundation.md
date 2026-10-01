# Generic commit foundation (0.10.0)

This independent package confirms changes reproducibly; it does not decide their meaning.
Callers reference this package as a versioned dependency and provide domain-specific
composition. `CommitIntent<T>` accepts an owner-validated proposal. `prepare_with`
consumes it and its deterministic owner encoder. The sealed representation is
immutable bytes, not a mutable generic object or a semantic shadow store.

## State and identity

| Lifecycle | Storage/read behavior |
| --- | --- |
| Draft | Owner's mutable intent, no mandatory persistence. |
| Prepared | Sealed, optionally staged; never returned by current reads. |
| Committed | Content, canonical head and replay receipt published atomically. |
| Abandoned | Explicitly cancelled Prepared; eligible only after owner grace. |

IDs hash length-framed components using SHA-256. Prepared identity includes
domain, operation ID, exact predecessor sequence **and** commit reference,
sorted unique parent references and payload digest. There is no timestamp,
nonce, address, platform-dependent hash or retry count. The owner must supply
canonical bytes; JSON objects with arbitrary serialization ordering are not
silently interpreted as equal. Same operation + same sealed intent returns
the original receipt, even after later head changes. Changed intent with the
same operation ID conflicts; new operation with stale base conflicts.

Payload content digest and commit reference are different identities. Identical
operation retries occupy one staged/committed entry. Across different operations
payload bytes currently remain in each bounded record; this is not a shared
content-object pool. No uncontrolled external objects are written by the kernel.

## Atomic boundaries

Memory: exclusive write lock → clone bounded image → common lifecycle → validate
image → one image swap. A closure error drops the proposal. Read snapshots never
see a partial head.

redb: exclusive write transaction → decode/validate existing image → **same**
common lifecycle → validate/encode → insert image → database commit. A failure
before durable commit rolls back content, head and receipt together. Identical
staging/replay bytes drop the write transaction without rewriting the image.
Loss of receipt delivery **after** database commit is an uncertain delivery, not
a failed mutation: lookup/replay the same operation. There is no internal retry
loop, distributed transaction, owner callback or provider effect.

Failure injection is compiled only into tests. It covers before/during/after
preparation, before/during head publication, and after head mutation before
receipt delivery. A separate backend decorator simulates lost delivery after
durable commit, including redb. Redb's filesystem/power-loss guarantees remain
those of redb; tests here do not simulate disk controller failure.

## DAG, merge and reclamation

The current head and all valid receipts are roots. Parent references are exact,
same-domain, preexisting and monotonically earlier, preventing cycles and ABA.
An ordinary commit has one predecessor (none for genesis). Multiple parents
describe an explicitly owner-resolved merge. `MergeRequest` / `MergeOutcome`
provide the protocol only: the kernel never guesses an owner merge policy.
`committed(receipt.commit_ref)` returns original result bytes, not current bytes.

Prepared is always protected. An owner explicitly abandons it and supplies a
monotonic reclamation epoch after any retention/grace interval. Repeated abandon
does not shorten grace. GC rechecks within the writer fence; it cannot overtake
an in-flight commit or delete a valid receipt. Backend writes are the in-flight
unit. External work uses explicit durable InFlight/Retained roots; these are
owner-released protections, not timeout leases. Crash before publication
leaves either the old Prepared or no object, never a half-committed current head.
No automatic stale abandonment, semantic retention rule or two-year default.

Maximums: 1 MiB payload, 128-byte domain and operation ID, 8 parents, 256 staged
records, 4,096 committed receipts, 64 MiB payload aggregate, 96 MiB encoded redb
image. Capacity returns a typed error; no canonical record is silently evicted.
These are safety ceilings, not recommended active working-set sizes. Committed
receipt retirement is not implemented; an owner must not claim unlimited storage.
Reclaimed-operation identities remain as bounded tombstones (4,096); replay
cannot resurrect abandoned data. Capacity rejects instead of evicting identities.
External references are limited to 256, with 64 roots/references per object.

## Standalone acceptance

`conformance/registry` is an independent Cargo workspace with exactly one direct
dependency, `zixcel-revision = 0.10.0` from the private registry. No source path is
used. The default executable exercises memory only; feature `durable` adds redb.
Run `memory`, or `create PATH` followed in another process by `read PATH`.
Both durable processes print the identical original receipt and assert the exact
later current payload. Missing paths reject without initializing a store.

Owner unit/integration tests additionally cover 100 competing writers on the
same base, 100 repetitions at each injected failure point, physical redb size
stability, reclamation/grace/in-flight exclusion, tampering, capacity and merge
parent integrity. Graph consumer tests also exercise durable Prepared, 100 writer
contention, restart, explicit physical recovery, and owner-controlled reclamation.

## External ownership (C4)

Reserve immutable owner/object identities before creating external bytes. Stage
and reservation occur under the Graph backend writer when using Graph APIs.
The standalone reservation may leave Prepared after a registration failure; the
owner must not create bytes until reservation succeeds. There is no cross-owner
ACID transaction. Owners publish only after referenced bytes are durable.

Prepared and all committed receipts protect objects. Explicit abandonment plus
elapsed owner-supplied grace permits reclamation only with no retained/in-flight
roots. An atomic claim fences new references; the owner performs idempotent
physical deletion, then acknowledges the exact persisted permit. Foundation
removes its metadata only, never foreign files. Immutable object identities must
not be reused. A crashed claimant resumes the same permit; no clock steals it.

Read-only open uses a read-only redb handle. Explicit recovery acquires the
normal backend writer lock and preserves logical corruption as an error.
Graph data and Foundation head sequence must agree; reads never repair them.

## Historical migration order (not implied by this standalone API)

M3-C2 replaces sem-lang CAS/receipt mechanics while keeping meaning, scope,
cardinality and memory policy there. M3-C3 replaces Hatter control primitives
while keeping authorization/Confirmation/Continuation there. M3-C4 replaces old
Graph CAS and OP70 preparation fence and accounts for previously external
immutable objects. M3-C5 deletes remaining duplicate lifecycles. None of those
consumer cutovers is certified by standalone tests.
