// Revision foundation 0.10.0: extracted unchanged mechanics; no domain policy.
//! Generic commit acceptance: payload meaning belongs exclusively to callers.
use std::sync::{Arc, Barrier};
use zixcel_revision::{
    Backend, CommitIntent, CommitOutcome, CommitStore, Lifecycle, MemoryBackend, RevisionRef,
};

fn intent(
    operation: &str,
    revision: RevisionRef,
    parents: Vec<String>,
    payload: &[u8],
) -> zixcel_revision::PreparedCommit {
    CommitIntent {
        domain: "example".into(),
        operation_id: operation.into(),
        expected_revision: revision,
        parents,
        payload: payload.to_vec(),
    }
    .prepare()
    .unwrap()
}

#[test]
fn immutable_prepare_replay_stale_and_reclamation_are_one_lifecycle() {
    lifecycle(MemoryBackend::default());
    #[cfg(feature = "redb")]
    {
        let d = tempfile::tempdir().unwrap();
        lifecycle(zixcel_revision::RedbBackend::create(d.path().join("lifecycle.redb")).unwrap());
    }
}

fn lifecycle<B: Backend>(backend: B) {
    let store = CommitStore::new(backend);
    let a = intent("a", RevisionRef::default(), vec![], b"alpha");
    assert_eq!(a, intent("a", RevisionRef::default(), vec![], b"alpha"));
    store.stage(&a).unwrap();
    assert_eq!(
        store.lifecycle(a.reference()).unwrap(),
        Some(Lifecycle::Prepared)
    );
    assert!(store.current("example").unwrap().is_none());
    assert!(store.reclaim_candidates(100).unwrap().is_empty());
    let CommitOutcome::Committed(first) = store.commit(&a) else {
        panic!("first commit")
    };
    let b = intent(
        "b",
        first.committed_revision.clone(),
        vec![first.commit_ref.clone()],
        b"beta",
    );
    let CommitOutcome::Committed(second) = store.commit(&b) else {
        panic!("next commit")
    };
    assert_eq!(store.commit(&a), CommitOutcome::NoChange(first.clone()));
    assert_eq!(
        store.current("example").unwrap().unwrap().payload(),
        b"beta"
    );
    assert_eq!(store.receipt("example", "a").unwrap(), Some(first.clone()));
    assert_eq!(
        store
            .committed(&first.commit_ref)
            .unwrap()
            .unwrap()
            .payload(),
        b"alpha"
    );
    let stale = intent("stale", RevisionRef::default(), vec![], b"unused");
    assert!(matches!(store.commit(&stale), CommitOutcome::Conflict(_)));
    let conflicting_id = intent("a", RevisionRef::default(), vec![], b"altered");
    assert!(matches!(
        store.commit(&conflicting_id),
        CommitOutcome::Conflict(_)
    ));
    let garbage = intent(
        "garbage",
        second.committed_revision.clone(),
        vec![second.commit_ref],
        b"discard",
    );
    for _ in 0..100 {
        store.stage(&garbage).unwrap();
    }
    assert_eq!(store.stats().unwrap().prepared, 1);
    store.abandon(garbage.reference(), 9).unwrap();
    assert_eq!(
        store.lifecycle(garbage.reference()).unwrap(),
        Some(Lifecycle::Abandoned)
    );
    assert!(store.reclaim_candidates(8).unwrap().is_empty());
    assert_eq!(
        store.reclaim_candidates(9).unwrap(),
        vec![garbage.reference().to_owned()]
    );
    assert_eq!(store.reclaim(9).unwrap(), 1);
    assert_eq!(store.stats().unwrap().prepared, 0);
    assert_eq!(store.receipt("example", "a").unwrap(), Some(first));
    assert_eq!(
        store.reachable_from(&store.roots().unwrap()).unwrap().len(),
        2
    );
}

#[test]
fn one_hundred_same_base_writers_publish_exactly_one_result() {
    concurrent(MemoryBackend::default());
    #[cfg(feature = "redb")]
    {
        let d = tempfile::tempdir().unwrap();
        concurrent(zixcel_revision::RedbBackend::create(d.path().join("writers.redb")).unwrap());
    }
}

fn concurrent<B: Backend + 'static>(backend: B) {
    let store = Arc::new(CommitStore::new(backend));
    let barrier = Arc::new(Barrier::new(100));
    let workers: Vec<_> = (0..100)
        .map(|n| {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::Builder::new()
                .stack_size(128 * 1024)
                .spawn(move || {
                    let p = intent(
                        &format!("op{n}"),
                        RevisionRef::default(),
                        vec![],
                        b"proposal",
                    );
                    barrier.wait();
                    store.commit(&p)
                })
                .unwrap()
        })
        .collect();
    let results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, CommitOutcome::Committed(_)))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, CommitOutcome::Conflict(_)))
            .count(),
        99
    );
    let stats = store.stats().unwrap();
    assert_eq!(stats.committed, 1);
    assert_eq!(stats.prepared, 0);
}
