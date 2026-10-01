// Revision foundation 0.10.0: extracted unchanged mechanics; no domain policy.
//! ZG1: bounded races and abrupt process exits at durable lifecycle boundaries.
use std::sync::Barrier;
use zixcel_revision::*;

fn source() -> (PreparedCommit, ExternalObjectRef, ExternalRoot) {
    (
        CommitIntent {
            domain: "owner".into(),
            operation_id: "source".into(),
            expected_revision: RevisionRef::default(),
            parents: vec![],
            payload: b"source-reference".to_vec(),
        }
        .prepare()
        .unwrap(),
        ExternalObjectRef {
            owner: "owner".into(),
            object: "exact-object".into(),
        },
        ExternalRoot {
            kind: ExternalRootKind::InFlight,
            holder: "reader".into(),
        },
    )
}
fn create<B: Backend>(
    store: &CommitStore<B>,
) -> (
    PreparedCommit,
    CommitReceipt,
    ExternalRegistration,
    ExternalRoot,
) {
    let (prepared, external, reader) = source();
    store
        .reserve_external(&prepared, std::slice::from_ref(&external))
        .unwrap();
    let CommitOutcome::Committed(receipt) = store.commit(&prepared) else {
        panic!("commit")
    };
    let registration = store
        .external_registration(&external, prepared.reference())
        .unwrap()
        .unwrap();
    (prepared, receipt, registration, reader)
}
fn races<B: Backend>(store: &CommitStore<B>) {
    let (prepared, receipt, registration, reader) = create(store);
    let barrier = Barrier::new(2);
    let retained = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            barrier.wait();
            store.retain_registration(&registration, &reader)
        });
        let b = scope.spawn(|| {
            barrier.wait();
            store.retire_external_registration(&registration)
        });
        let retained = a.join().unwrap().is_ok();
        assert_eq!(b.join().unwrap().unwrap().registration, registration);
        retained
    });
    assert_eq!(
        store.registration_status(&registration).unwrap(),
        if retained {
            ExternalRegistrationStatus::Retiring
        } else {
            ExternalRegistrationStatus::Retired
        }
    );
    if retained {
        assert!(
            !store
                .external_reclaimable(&registration.object, u64::MAX)
                .unwrap()
        );
    }
    std::thread::scope(|scope| {
        let threads: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    let result = store.retire_external_registration(&registration).unwrap();
                    store.release_registration(&registration, &reader).unwrap();
                    result
                })
            })
            .collect();
        for t in threads {
            assert_eq!(t.join().unwrap().registration, registration);
        }
    });
    let b = std::thread::scope(|scope| {
        let registration_thread = scope.spawn(|| store.reregister_external(&registration).unwrap());
        let old_retire = scope.spawn(|| store.retire_external_registration(&registration).unwrap());
        assert_eq!(old_retire.join().unwrap().registration, registration);
        registration_thread.join().unwrap()
    });
    assert_eq!(
        store.registration_status(&b).unwrap(),
        ExternalRegistrationStatus::Active
    );
    store.retire_external_registration(&b).unwrap();
    let barrier = Barrier::new(2);
    let (claim, registration) = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            barrier.wait();
            store.claim_external_reclamation(&registration.object, u64::MAX)
        });
        let b = scope.spawn(|| {
            barrier.wait();
            store.reregister_external(&b)
        });
        (a.join().unwrap().unwrap(), b.join().unwrap())
    });
    match (claim, registration) {
        (Some(permit), Err(_)) => store.complete_external_reclamation(&permit).unwrap(),
        (None, Ok(current)) => assert_eq!(
            store.registration_status(&current).unwrap(),
            ExternalRegistrationStatus::Active
        ),
        unexpected => panic!("invalid race linearization: {unexpected:?}"),
    }
    assert_eq!(store.commit(&prepared), CommitOutcome::NoChange(receipt));
}
#[test]
fn retain_retire_release_reregister_reclaim_races_are_linearizable() {
    for _ in 0..8 {
        races(&CommitStore::new(MemoryBackend::default()));
        #[cfg(feature = "redb")]
        {
            let dir = tempfile::tempdir().unwrap();
            races(&CommitStore::new(
                RedbBackend::create(dir.path().join("race.redb")).unwrap(),
            ));
        }
    }
}

#[cfg(feature = "redb")]
fn child(file: &std::path::Path, operation: &str) {
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_child", "--nocapture"])
        .env("ZG1_CHILD_FILE", file)
        .env("ZG1_CHILD_OPERATION", operation)
        .output()
        .unwrap();
    assert_eq!(
        result.status.code(),
        Some(77),
        "child: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[cfg(feature = "redb")]
#[test]
fn durable_retirement_hold_drain_reclaim_and_lost_response_survive_abrupt_exit() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("owner.redb");
    let (prepared, receipt, registration, reader) = {
        let store = CommitStore::new(RedbBackend::create(&file).unwrap());
        let result = create(&store);
        store.retain_registration(&result.2, &result.3).unwrap();
        result
    };
    child(&file, "retire");
    {
        let store = CommitStore::new(RedbBackend::recover_existing(&file).unwrap());
        assert_eq!(
            store.registration_status(&registration).unwrap(),
            ExternalRegistrationStatus::Retiring
        );
        assert_eq!(
            store
                .retire_external_registration(&registration)
                .unwrap()
                .registration,
            registration
        );
        assert!(store.retain_registration(&registration, &reader).is_err());
        assert!(
            !store
                .external_reclaimable(&registration.object, u64::MAX)
                .unwrap()
        );
    }
    child(&file, "release");
    {
        let store = CommitStore::new(RedbBackend::recover_existing(&file).unwrap());
        assert_eq!(
            store.registration_status(&registration).unwrap(),
            ExternalRegistrationStatus::Retired
        );
        store.release_registration(&registration, &reader).unwrap();
        assert!(
            store
                .external_reclaimable(&registration.object, u64::MAX)
                .unwrap()
        );
    }
    child(&file, "claim");
    {
        let store = CommitStore::new(RedbBackend::recover_existing(&file).unwrap());
        let a = store
            .claim_external_reclamation(&registration.object, u64::MAX)
            .unwrap()
            .unwrap();
        assert_eq!(
            store
                .claim_external_reclamation(&registration.object, u64::MAX)
                .unwrap(),
            Some(a)
        );
        assert!(store.reregister_external(&registration).is_err());
    }
    child(&file, "complete");
    let store = CommitStore::new(RedbBackend::recover_existing(&file).unwrap());
    assert!(
        store
            .external_registration(&registration.object, prepared.reference())
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .retire_external_registration(&registration)
            .unwrap()
            .registration,
        registration
    );
    assert_eq!(store.commit(&prepared), CommitOutcome::NoChange(receipt));
}
#[cfg(feature = "redb")]
#[test]
fn crash_child() {
    let Ok(file) = std::env::var("ZG1_CHILD_FILE") else {
        return;
    };
    let store = CommitStore::new(RedbBackend::open_existing(file).unwrap());
    let (prepared, external, reader) = source();
    let registration = store
        .external_registration(&external, prepared.reference())
        .unwrap()
        .unwrap();
    match std::env::var("ZG1_CHILD_OPERATION").unwrap().as_str() {
        "retire" => {
            store.retire_external_registration(&registration).unwrap();
        }
        "release" => {
            store.release_registration(&registration, &reader).unwrap();
        }
        "claim" => {
            store
                .claim_external_reclamation(&external, u64::MAX)
                .unwrap()
                .unwrap();
        }
        "complete" => {
            let permit = store
                .claim_external_reclamation(&external, u64::MAX)
                .unwrap()
                .unwrap();
            store.complete_external_reclamation(&permit).unwrap();
        }
        _ => panic!("unknown operation"),
    }
    std::process::exit(77);
}
