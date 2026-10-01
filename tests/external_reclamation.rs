// Revision foundation 0.10.0: extracted unchanged mechanics; no domain policy.
//! C4: object deletion is performed only by this fixture owner, never Foundation.
use zixcel_revision::*;
fn proposal() -> PreparedCommit {
    CommitIntent {
        domain: "external".into(),
        operation_id: "prepare".into(),
        expected_revision: RevisionRef::default(),
        parents: vec![],
        payload: b"reference-only".to_vec(),
    }
    .prepare()
    .unwrap()
}
fn scenario<B: Backend>(store: &CommitStore<B>) {
    let p = proposal();
    let object = ExternalObjectRef {
        owner: "fixture-repository".into(),
        object: "immutable-object-1".into(),
    };
    store
        .reserve_external(&p, std::slice::from_ref(&object))
        .unwrap();
    assert_eq!(
        store.committed(p.reference()),
        Err(Failure::Rejected(Rejection::PreparedNotCanonical))
    );
    let hold = ExternalRoot {
        kind: ExternalRootKind::InFlight,
        holder: "worker-1".into(),
    };
    store.retain_external(&object, &hold).unwrap();
    let retained = ExternalRoot {
        kind: ExternalRootKind::Retained,
        holder: "owner-retained".into(),
    };
    store.retain_external(&object, &retained).unwrap();
    store.abandon(p.reference(), 10).unwrap();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..16)
            .map(|_| scope.spawn(|| store.claim_external_reclamation(&object, 100)))
            .collect();
        for handle in handles {
            assert!(handle.join().unwrap().unwrap().is_none());
        }
    });
    assert!(
        store
            .claim_external_reclamation(&object, 100)
            .unwrap()
            .is_none()
    );
    assert_eq!(store.reclaim(100).unwrap(), 0);
    store.release_external(&object, &hold).unwrap();
    assert!(!store.external_reclaimable(&object, 100).unwrap());
    store.release_external(&object, &retained).unwrap();
    assert!(
        store
            .claim_external_reclamation(&object, 9)
            .unwrap()
            .is_none()
    );
    let permit = store
        .claim_external_reclamation(&object, 10)
        .unwrap()
        .unwrap();
    assert!(
        store.retain_external(&object, &hold).is_err(),
        "cannot race a physical owner deletion"
    );
    assert_eq!(
        store.claim_external_reclamation(&object, 10).unwrap(),
        Some(permit.clone())
    );
    store.complete_external_reclamation(&permit).unwrap();
    assert_eq!(store.reclaim(10).unwrap(), 1);
    assert!(store.current("external").unwrap().is_none());
    assert_eq!(
        store.commit(&p),
        CommitOutcome::Rejected(Rejection::Abandoned)
    );
    assert_eq!(
        store.stage(&p),
        Err(Failure::Rejected(Rejection::Abandoned))
    );
}
#[test]
fn explicit_external_ownership_grace_and_in_flight_have_backend_parity() {
    scenario(&CommitStore::new(MemoryBackend::default()));
    #[cfg(feature = "redb")]
    {
        let d = tempfile::tempdir().unwrap();
        scenario(&CommitStore::new(
            RedbBackend::create(d.path().join("state.redb")).unwrap(),
        ));
    }
}

fn concurrent_replay<B: Backend>(store: &CommitStore<B>) {
    let p = proposal();
    let object = ExternalObjectRef {
        owner: "fixture".into(),
        object: "canonical".into(),
    };
    store
        .reserve_external(&p, std::slice::from_ref(&object))
        .unwrap();
    let CommitOutcome::Committed(receipt) = store.commit(&p) else {
        panic!("first commit")
    };
    std::thread::scope(|scope| {
        let replay = scope.spawn(|| {
            for _ in 0..100 {
                assert_eq!(store.commit(&p), CommitOutcome::NoChange(receipt.clone()));
            }
        });
        let reclaim = scope.spawn(|| {
            for _ in 0..100 {
                assert!(
                    store
                        .claim_external_reclamation(&object, u64::MAX)
                        .unwrap()
                        .is_none()
                );
                assert_eq!(store.reclaim(u64::MAX).unwrap(), 0);
            }
        });
        replay.join().unwrap();
        reclaim.join().unwrap();
    });
    assert_eq!(store.receipt("external", "prepare").unwrap(), Some(receipt));
    assert_eq!(store.stats().unwrap().committed, 1);
}
#[test]
fn concurrent_replay_and_reclamation_preserve_canonical_external_roots() {
    concurrent_replay(&CommitStore::new(MemoryBackend::default()));
    #[cfg(feature = "redb")]
    {
        let dir = tempfile::tempdir().unwrap();
        concurrent_replay(&CommitStore::new(
            RedbBackend::create(dir.path().join("replay.redb")).unwrap(),
        ));
    }
}

#[test]
fn cancellation_between_standalone_stage_and_reservation_denies_owner_creation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct CancelAfterStage {
        inner: MemoryBackend,
        writes: AtomicUsize,
    }
    impl Backend for &CancelAfterStage {
        fn read<T>(&self, read: impl FnOnce(&CommitState) -> T) -> Result<T, Failure> {
            self.inner.read(read)
        }
        fn write<T>(
            &self,
            change: impl FnOnce(&mut CommitState) -> Result<T, Failure>,
        ) -> Result<T, Failure> {
            let result = self.inner.write(change)?;
            if self.writes.fetch_add(1, Ordering::SeqCst) == 0 {
                CommitStore::new(*self).abandon(proposal().reference(), 0)?;
            }
            Ok(result)
        }
    }
    let backend = CancelAfterStage {
        inner: MemoryBackend::default(),
        writes: AtomicUsize::new(0),
    };
    let store = CommitStore::new(&backend);
    let object = ExternalObjectRef {
        owner: "fixture".into(),
        object: "cancelled".into(),
    };
    assert_eq!(
        store.reserve_external(&proposal(), std::slice::from_ref(&object)),
        Err(Failure::Rejected(Rejection::Abandoned))
    );
    assert!(!store.external_reclaimable(&object, 0).unwrap());
    assert_eq!(store.reclaim(0).unwrap(), 1);
    assert!(store.current("external").unwrap().is_none());
}
