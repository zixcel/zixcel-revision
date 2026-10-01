// Revision foundation 0.10.0: extracted unchanged mechanics; no domain policy.
//! ZG1: receipt identity, external registration and reader holds are independent.
use zixcel_revision::*;

fn prepare(operation: &str, base: RevisionRef) -> PreparedCommit {
    CommitIntent {
        domain: "owner".into(),
        operation_id: operation.into(),
        parents: base.commit.iter().cloned().collect(),
        expected_revision: base,
        payload: operation.as_bytes().to_vec(),
    }
    .prepare()
    .unwrap()
}
fn published(outcome: CommitOutcome) -> CommitReceipt {
    let CommitOutcome::Committed(receipt) = outcome else {
        panic!("{outcome:?}")
    };
    receipt
}
fn object() -> ExternalObjectRef {
    ExternalObjectRef {
        owner: "owner".into(),
        object: "shared-exact-object".into(),
    }
}
fn hold() -> ExternalRoot {
    ExternalRoot {
        kind: ExternalRootKind::InFlight,
        holder: "reader".into(),
    }
}
fn lifecycle<B: Backend>(store: &CommitStore<B>) {
    let e = object();
    let p = prepare("first", RevisionRef::default());
    assert_eq!(
        store.retain_external(&e, &hold()),
        Err(Failure::Rejected(Rejection::NotPrepared))
    );
    store
        .reserve_external(&p, std::slice::from_ref(&e))
        .unwrap();
    let a = store
        .external_registration(&e, p.reference())
        .unwrap()
        .unwrap();
    let receipt = published(store.commit(&p));
    store.retain_registration(&a, &hold()).unwrap();
    let next = prepare("second", receipt.committed_revision.clone());
    let second = published(store.commit(&next));
    let retirement = store.retire_external_registration(&a).unwrap();
    assert_eq!(retirement.registration, a);
    assert_eq!(
        store.registration_status(&a).unwrap(),
        ExternalRegistrationStatus::Retiring
    );
    assert!(store.retain_registration(&a, &hold()).is_err());
    assert!(store.retain_external(&e, &hold()).is_err());
    assert!(!store.external_reclaimable(&e, u64::MAX).unwrap());
    store.release_registration(&a, &hold()).unwrap();
    store.release_registration(&a, &hold()).unwrap();
    assert_eq!(
        store.registration_status(&a).unwrap(),
        ExternalRegistrationStatus::Retired
    );
    assert!(store.external_reclaimable(&e, u64::MAX).unwrap());
    // Explicit new generation, original retirement replay never retires B.
    let b = store.reregister_external(&a).unwrap();
    assert_ne!(a.generation, b.generation);
    assert_eq!(store.reregister_external(&a).unwrap(), b);
    store.retain_registration(&b, &hold()).unwrap();
    assert_eq!(store.retire_external_registration(&a).unwrap(), retirement);
    assert_eq!(
        store.registration_status(&b).unwrap(),
        ExternalRegistrationStatus::Active
    );
    assert!(store.release_registration(&a, &hold()).is_err());
    assert!(!store.external_reclaimable(&e, u64::MAX).unwrap());
    // A separate canonical registration protects the same object independently.
    store
        .reserve_external(&next, std::slice::from_ref(&e))
        .unwrap();
    let shared = store
        .external_registration(&e, next.reference())
        .unwrap()
        .unwrap();
    store.retire_external_registration(&b).unwrap();
    store.release_registration(&b, &hold()).unwrap();
    assert!(!store.external_reclaimable(&e, u64::MAX).unwrap());
    store.retain_registration(&shared, &hold()).unwrap();
    store.retire_external_registration(&shared).unwrap();
    assert!(!store.external_reclaimable(&e, u64::MAX).unwrap());
    store.release_registration(&shared, &hold()).unwrap();
    let permit = store
        .claim_external_reclamation(&e, u64::MAX)
        .unwrap()
        .unwrap();
    assert!(store.reregister_external(&shared).is_err());
    assert!(
        store
            .reserve_external(&next, std::slice::from_ref(&e))
            .is_err()
    );
    store.complete_external_reclamation(&permit).unwrap();
    store.complete_external_reclamation(&permit).unwrap();
    assert!(
        store
            .external_registration(&e, p.reference())
            .unwrap()
            .is_none()
    );
    assert!(store.retain_registration(&b, &hold()).is_err());
    assert!(
        store
            .reserve_external(&p, std::slice::from_ref(&e))
            .is_err()
    );
    assert_eq!(store.retire_external_registration(&a).unwrap(), retirement);
    assert_eq!(store.commit(&p), CommitOutcome::NoChange(receipt.clone()));
    assert_eq!(store.commit(&next), CommitOutcome::NoChange(second.clone()));
    assert_eq!(store.current("owner").unwrap(), Some(next));
    assert_eq!(store.committed(&receipt.commit_ref).unwrap(), Some(p));
    assert_eq!(store.receipt("owner", "second").unwrap(), Some(second));
}
#[test]
fn retirement_replay_shared_objects_and_aba_preserve_receipts() {
    lifecycle(&CommitStore::new(MemoryBackend::default()));
}
#[cfg(feature = "redb")]
#[test]
fn retirement_replay_shared_objects_and_aba_preserve_durable_receipts() {
    let dir = tempfile::tempdir().unwrap();
    lifecycle(&CommitStore::new(
        RedbBackend::create(dir.path().join("owner.redb")).unwrap(),
    ));
}
