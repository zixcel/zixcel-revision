// Revision foundation 0.10.0: extracted unchanged mechanics; no domain policy.
use super::store::CommitPoint;
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

fn prepared(id: &str) -> PreparedCommit {
    CommitIntent {
        domain: "example".into(),
        operation_id: id.into(),
        expected_revision: RevisionRef::default(),
        parents: vec![],
        payload: b"owner-validated".to_vec(),
    }
    .prepare()
    .unwrap()
}

fn rollback_matrix<B: Backend>(backend: B) {
    let store = CommitStore::new(backend);
    let p = prepared("retry");
    // The same immutable Prepared survives every rejected publication. It is
    // never a current head, and does not multiply on retry or failed CAS.
    store.stage(&p).unwrap();
    for point in [
        CommitPoint::BeforePrepare,
        CommitPoint::DuringPrepare,
        CommitPoint::AfterPrepare,
        CommitPoint::BeforeHead,
        CommitPoint::DuringHead,
        CommitPoint::AfterHeadBeforeReceiptDelivery,
    ] {
        for _ in 0..100 {
            assert_eq!(
                store.fail_at(&p, point),
                CommitOutcome::Failure(Failure::Storage)
            );
            assert!(store.current("example").unwrap().is_none());
            assert!(store.receipt("example", "retry").unwrap().is_none());
            assert_eq!(
                store.stats().unwrap(),
                StoreStats {
                    prepared: 1,
                    abandoned: 0,
                    committed: 0,
                    content_bytes: p.payload().len()
                }
            );
        }
    }
    assert!(matches!(store.commit(&p), CommitOutcome::Committed(_)));
    assert_eq!(store.stats().unwrap().prepared, 0);
}

#[test]
fn memory_rolls_back_every_publication_boundary() {
    rollback_matrix(MemoryBackend::default());
}

#[cfg(feature = "redb")]
#[test]
fn redb_rolls_back_every_boundary_and_reopens_exact_receipts() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("commit.redb");
    rollback_matrix(RedbBackend::create(&path).unwrap());
    let store = CommitStore::new(RedbBackend::open_existing(&path).unwrap());
    let receipt = store.receipt("example", "retry").unwrap().unwrap();
    assert_eq!(
        store.commit(&prepared("retry")),
        CommitOutcome::NoChange(receipt)
    );
    assert_eq!(
        store.current("example").unwrap().unwrap().payload(),
        b"owner-validated"
    );
    assert!(RedbBackend::open_existing(d.path().join("absent")).is_err());
    assert!(!d.path().join("absent").exists());
}

struct LostDelivery<B> {
    backend: B,
    lose: AtomicBool,
}
impl<B: Backend> Backend for LostDelivery<B> {
    fn read<T>(&self, f: impl FnOnce(&CommitState) -> T) -> Result<T, Failure> {
        self.backend.read(f)
    }
    fn write<T>(
        &self,
        f: impl FnOnce(&mut CommitState) -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        let result = self.backend.write(f)?;
        if self.lose.swap(false, Ordering::SeqCst) {
            Err(Failure::DeliveryUnknown)
        } else {
            Ok(result)
        }
    }
}
fn replay_delivery<B: Backend>(backend: B) {
    let store = CommitStore::new(LostDelivery {
        backend,
        lose: AtomicBool::new(true),
    });
    let p = prepared("lost");
    assert_eq!(
        store.commit(&p),
        CommitOutcome::Failure(Failure::DeliveryUnknown)
    );
    let receipt = store.receipt("example", "lost").unwrap().unwrap();
    assert_eq!(store.commit(&p), CommitOutcome::NoChange(receipt));
    assert_eq!(store.stats().unwrap().committed, 1);
}
#[test]
fn lost_delivery_replays_instead_of_repeating_mutation() {
    replay_delivery(MemoryBackend::default());
    #[cfg(feature = "redb")]
    {
        let d = tempfile::tempdir().unwrap();
        replay_delivery(RedbBackend::create(d.path().join("lost.redb")).unwrap());
    }
}

#[test]
fn prepared_encoding_rejects_tampering_limits_and_owner_rejection_without_storage() {
    let p = prepared("valid");
    let mut forged = p.clone();
    forged.payload.push(0);
    let store = CommitStore::new(MemoryBackend::default());
    assert_eq!(
        store.commit(&forged),
        CommitOutcome::Rejected(Rejection::InvalidIntent)
    );
    assert_eq!(store.stats().unwrap(), StoreStats::default());
    let huge = CommitIntent {
        domain: "example".into(),
        operation_id: "huge".into(),
        expected_revision: RevisionRef::default(),
        parents: vec![],
        payload: vec![0; MAX_PAYLOAD_BYTES + 1],
    };
    assert_eq!(huge.prepare(), Err(Rejection::Capacity));
    let rejected = CommitIntent {
        domain: "example".into(),
        operation_id: "rejected".into(),
        expected_revision: RevisionRef::default(),
        parents: vec![],
        payload: (),
    }
    .prepare_with(|()| Err(Rejection::InvalidIntent));
    assert_eq!(rejected, Err(Rejection::InvalidIntent));
    assert_eq!(store.stats().unwrap(), StoreStats::default());
}

#[test]
fn merge_is_owner_decided_but_parent_identity_and_dag_are_enforced() {
    let store = CommitStore::new(MemoryBackend::default());
    let CommitOutcome::Committed(a) = store.commit(&prepared("a")) else {
        panic!("a")
    };
    let b = CommitIntent {
        domain: "example".into(),
        operation_id: "b".into(),
        expected_revision: a.committed_revision.clone(),
        parents: vec![a.commit_ref.clone()],
        payload: b"second".to_vec(),
    }
    .prepare()
    .unwrap();
    let CommitOutcome::Committed(b) = store.commit(&b) else {
        panic!("b")
    };
    let request = MergeRequest {
        base: a.commit_ref.clone(),
        left: a.commit_ref.clone(),
        right: b.commit_ref.clone(),
    };
    let owner: MergeOutcome<Vec<u8>, _> =
        MergeOutcome::NeedsOwnerResolution("single slot conflict");
    assert!(matches!(owner, MergeOutcome::NeedsOwnerResolution(_)));
    assert_eq!(store.stats().unwrap().committed, 2);
    let merged = CommitIntent {
        domain: "example".into(),
        operation_id: "owner-merge".into(),
        expected_revision: b.committed_revision,
        parents: vec![request.left, request.right],
        payload: b"explicit-owner-resolution".to_vec(),
    }
    .prepare()
    .unwrap();
    let CommitOutcome::Committed(receipt) = store.commit(&merged) else {
        panic!("merge")
    };
    assert_eq!(receipt.parent_refs.len(), 2);
    assert_eq!(
        store.reachable_from(&[receipt.commit_ref]).unwrap().len(),
        3
    );
    assert_eq!(
        store.commit(&prepared("stale")).retry_class(),
        RetryClass::Revalidate
    );
}

#[test]
fn prepared_capacity_and_abandonment_do_not_evict_live_or_committed_objects() {
    let store = CommitStore::new(MemoryBackend::default());
    for n in 0..MAX_PREPARED {
        store.stage(&prepared(&format!("p{n}"))).unwrap();
    }
    assert_eq!(store.stage(&prepared("overflow")), Err(Failure::Capacity));
    assert_eq!(store.stats().unwrap().prepared, MAX_PREPARED);
    assert_eq!(store.reclaim(u64::MAX).unwrap(), 0);
    let p = prepared("p0");
    store.abandon(p.reference(), 10).unwrap();
    store.abandon(p.reference(), 0).unwrap(); // A retry cannot shorten grace.
    assert_eq!(store.reclaim(0).unwrap(), 0);
    assert_eq!(
        store.commit(&p),
        CommitOutcome::Rejected(Rejection::Abandoned)
    );
    assert_eq!(store.reclaim(10).unwrap(), 1);
    assert_eq!(store.stats().unwrap().prepared, MAX_PREPARED - 1);
}

fn in_flight<B: Backend + 'static>(backend: B) {
    use std::sync::{Arc, Barrier, mpsc};
    let store = Arc::new(CommitStore::new(backend));
    let p = prepared("in-flight");
    store.stage(&p).unwrap();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let writer = {
        let store = store.clone();
        let entered = entered.clone();
        let release = release.clone();
        std::thread::spawn(move || store.pause_before_head(&p, &entered, &release))
    };
    entered.wait();
    let (send, receive) = mpsc::channel();
    let collector = {
        let store = store.clone();
        std::thread::spawn(move || send.send(store.reclaim(u64::MAX)).unwrap())
    };
    let early = receive.recv_timeout(std::time::Duration::from_millis(20));
    // Always release and join our own threads, even when the assertion will fail.
    release.wait();
    let result = writer.join().unwrap();
    collector.join().unwrap();
    assert!(early.is_err());
    assert_eq!(receive.recv().unwrap().unwrap(), 0);
    assert!(matches!(result, CommitOutcome::Committed(_)));
    assert_eq!(
        store.current("example").unwrap().unwrap().payload(),
        b"owner-validated"
    );
}
#[test]
fn reclamation_cannot_overtake_an_in_flight_publication() {
    in_flight(MemoryBackend::default());
    #[cfg(feature = "redb")]
    {
        let d = tempfile::tempdir().unwrap();
        in_flight(RedbBackend::create(d.path().join("flight.redb")).unwrap());
    }
}

#[cfg(feature = "redb")]
#[test]
fn repeated_failure_and_replay_do_not_grow_the_physical_image() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("bounded.redb");
    let store = CommitStore::new(RedbBackend::create(&path).unwrap());
    let p = prepared("physical");
    store.stage(&p).unwrap();
    let prepared_size = std::fs::metadata(&path).unwrap().len();
    for _ in 0..100 {
        assert!(matches!(
            store.fail_at(&p, CommitPoint::AfterPrepare),
            CommitOutcome::Failure(_)
        ));
        store.stage(&p).unwrap();
    }
    assert_eq!(std::fs::metadata(&path).unwrap().len(), prepared_size);
    let CommitOutcome::Committed(receipt) = store.commit(&p) else {
        panic!("committed")
    };
    let committed_size = std::fs::metadata(&path).unwrap().len();
    for _ in 0..100 {
        assert_eq!(store.commit(&p), CommitOutcome::NoChange(receipt.clone()));
    }
    assert_eq!(std::fs::metadata(&path).unwrap().len(), committed_size);
}
