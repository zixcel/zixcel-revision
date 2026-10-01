//! Separate-workspace/process archive consumer, with no owner dependencies.
use zixcel_revision::{
    Backend, CommitIntent, CommitOutcome, CommitStore, MemoryBackend, RevisionRef,
};

fn proposal() -> zixcel_revision::PreparedCommit {
    CommitIntent {
        domain: "standalone".into(),
        operation_id: "first".into(),
        expected_revision: RevisionRef::default(),
        parents: vec![],
        payload: b"canonical payload".to_vec(),
    }
    .prepare()
    .unwrap()
}
fn verify<B: Backend>(backend: B, create: bool) {
    let store = CommitStore::new(backend);
    let first = proposal();
    if create {
        assert!(store.current("standalone").unwrap().is_none());
        store.stage(&first).unwrap();
        assert!(store.current("standalone").unwrap().is_none());
        let CommitOutcome::Committed(receipt) = store.commit(&first) else {
            panic!("first commit")
        };
        let next = CommitIntent {
            domain: "standalone".into(),
            operation_id: "second".into(),
            expected_revision: receipt.committed_revision,
            parents: vec![receipt.commit_ref],
            payload: b"second payload".to_vec(),
        }
        .prepare()
        .unwrap();
        assert!(matches!(store.commit(&next), CommitOutcome::Committed(_)));
    }
    let receipt = store.receipt("standalone", "first").unwrap().unwrap();
    assert_eq!(
        store.commit(&first),
        CommitOutcome::NoChange(receipt.clone())
    );
    assert_eq!(
        store.current("standalone").unwrap().unwrap().payload(),
        b"second payload"
    );
    assert_eq!(store.stats().unwrap().committed, 2);
    println!(
        "{}:{}:{}:second payload",
        receipt.prepared_ref, receipt.commit_ref, receipt.committed_revision.sequence
    );
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("memory") if args.len() == 1 => verify(MemoryBackend::default(), true),
        #[cfg(feature = "durable")]
        Some("create") if args.len() == 2 => verify(
            zixcel_revision::RedbBackend::create(&args[1]).unwrap(),
            true,
        ),
        #[cfg(feature = "durable")]
        Some("read") if args.len() == 2 => verify(
            zixcel_revision::RedbBackend::open_existing(&args[1]).unwrap(),
            false,
        ),
        _ => panic!("usage: memory | create PATH | read PATH"),
    }
}
