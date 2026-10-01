#![cfg(feature = "attestation")]
use ed25519_dalek::SigningKey;
use zixcel_revision::{attestation::*, *};

#[test]
fn exact_proof_epoch_replay_cas_and_bounded_transition_history() {
    let key = [17; 32];
    let trust = VerificationMaterial {
        authority: "provider/retained/evidence".into(),
        epoch: 7,
        public_key: SigningKey::from_bytes(&key).verifying_key().to_bytes(),
    };
    let statement = Statement {
        subject_ref: content_digest(b"execution"),
        authority: trust.authority.clone(),
        authority_epoch: trust.epoch,
        claim_digest: content_digest(b"result"),
        provenance_refs: vec![content_digest(b"original-provider-generation")],
    };
    let attestation = Attestation::sign(statement, &key).unwrap();
    attestation.verify(&trust).unwrap();
    for field in 0..6 {
        let mut forged = attestation.clone();
        match field {
            0 => forged.statement.authority.push('x'),
            1 => forged.statement.authority_epoch += 1,
            2 => forged.statement.claim_digest = content_digest(b"different"),
            3 => forged.statement.subject_ref = content_digest(b"different-execution"),
            4 => forged.statement.provenance_refs.clear(),
            _ => forged.signature.clear(),
        }
        assert!(forged.verify(&trust).is_err());
    }
    let mut replacement = trust.clone();
    replacement.public_key = SigningKey::from_bytes(&[18; 32]).verifying_key().to_bytes();
    assert!(attestation.verify(&replacement).is_err());
    let store = CommitStore::new(MemoryBackend::default());
    let original = CommitIntent {
        domain: "execution/one".into(),
        operation_id: attestation.reference().unwrap(),
        expected_revision: RevisionRef::default(),
        parents: vec![],
        payload: serde_json::to_vec(&attestation).unwrap(),
    }
    .prepare()
    .unwrap();
    let CommitOutcome::Committed(first) = store.commit(&original) else {
        panic!("first")
    };
    let next = CommitIntent {
        domain: first.domain.clone(),
        operation_id: "confirmed".into(),
        expected_revision: first.committed_revision.clone(),
        parents: vec![first.commit_ref.clone()],
        payload: b"owner-accepted-reference".to_vec(),
    }
    .prepare()
    .unwrap();
    assert!(matches!(store.commit(&next), CommitOutcome::Committed(_)));
    assert_eq!(
        store.commit(&original),
        CommitOutcome::NoChange(first.clone())
    );
    let divergent = CommitIntent {
        operation_id: "divergent".into(),
        domain: first.domain.clone(),
        expected_revision: RevisionRef::default(),
        parents: vec![],
        payload: vec![],
    }
    .prepare()
    .unwrap();
    store.stage(&divergent).unwrap();
    assert!(matches!(
        store.commit(&divergent),
        CommitOutcome::Conflict(Conflict::Stale { .. })
    ));
    assert!(store.prepared(divergent.reference()).unwrap().is_some());
    let history = store.transitions(&first.domain, 0, 2).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0], first);
    assert_eq!(history[1].previous_revision, history[0].committed_revision);
    assert!(store.transitions(&first.domain, 0, 257).is_err());
}
