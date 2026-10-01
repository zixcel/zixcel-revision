//! Registry-only revision durability preflight. No product hooks or fake storage.
//! Parent observes a completed durable stage, then releases or kills this process.
#[cfg(feature = "durable")]
fn main() {
    use std::io::{Read, Write};
    use zixcel_revision::{CommitIntent, CommitOutcome, CommitStore, RedbBackend, RevisionRef};
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert_eq!(args.len(), 2, "create|writer|stage|read|recover PATH");
    let store = match args[0].as_str() {
        "create" => CommitStore::new(RedbBackend::create(&args[1]).expect("explicit create")),
        "writer" | "stage" | "recover" => {
            CommitStore::new(RedbBackend::recover_existing(&args[1]).expect("explicit writer"))
        }
        "read" => match RedbBackend::open_existing(&args[1]) {
            Ok(backend) => CommitStore::new(backend),
            Err(error) => {
                println!("READ_FAILURE:{error:?}");
                std::process::exit(2);
            }
        },
        _ => panic!("unknown command"),
    };
    let domain = "installation/hat-source-curator";
    if args[0] == "create" {
        let a = CommitIntent {
            domain: domain.into(),
            operation_id: "install/A".into(),
            expected_revision: RevisionRef::default(),
            parents: vec![],
            payload: b"artifact-A-reference".to_vec(),
        }
        .prepare()
        .unwrap();
        store.stage(&a).unwrap();
        assert!(matches!(store.commit(&a), CommitOutcome::Committed(_)));
    }
    if args[0] == "stage" {
        let a = store.receipt(domain, "install/A").unwrap().unwrap();
        let b = CommitIntent {
            domain: domain.into(),
            operation_id: "install/B".into(),
            expected_revision: a.committed_revision,
            parents: vec![a.commit_ref],
            payload: b"artifact-B-reference".to_vec(),
        }
        .prepare()
        .unwrap();
        store.stage(&b).unwrap();
        // Durable API returned. Keep the actual writable handle alive just as
        // an installer may between prepare and publication; do not fake a crash.
        println!("STAGED:{}", b.reference());
    }
    if args[0] == "writer" {
        println!("WRITER:OPEN");
    }
    if matches!(args[0].as_str(), "writer" | "stage") {
        std::io::stdout().flush().unwrap();
        let mut release = [0];
        std::io::stdin().read_exact(&mut release).unwrap();
        assert_eq!(release, [b'R']);
    }
    let current = store.current(domain).unwrap().unwrap();
    assert_eq!(current.payload(), b"artifact-A-reference");
    let receipt = store.receipt(domain, "install/A").unwrap().unwrap();
    let stats = store.stats().unwrap();
    let prepared = store.prepared_references(domain).unwrap();
    assert_eq!(stats.committed, 1);
    assert!(store.receipt(domain, "install/B").unwrap().is_none());
    assert_eq!(receipt.prepared_ref, current.reference());
    assert_eq!(receipt.previous_revision, RevisionRef::default());
    assert_eq!(receipt.committed_revision.sequence, 1);
    if args[0] == "stage" {
        assert_eq!(prepared.len(), 1);
    }
    assert!(prepared.len() <= 1);
    for reference in &prepared {
        let b = store.prepared(reference).unwrap().unwrap();
        assert_eq!(b.operation_id(), "install/B");
        assert_eq!(b.payload(), b"artifact-B-reference");
        assert_eq!(b.base_revision(), &receipt.committed_revision);
        let exact = CommitIntent {
            domain: domain.into(),
            operation_id: "install/B".into(),
            expected_revision: receipt.committed_revision.clone(),
            parents: vec![receipt.commit_ref.clone()],
            payload: b"artifact-B-reference".to_vec(),
        }
        .prepare()
        .unwrap();
        assert_eq!(b, exact);
    }
    println!(
        "CURRENT:A:{}:{}",
        receipt.commit_ref, receipt.committed_revision.sequence
    );
    println!("PREPARED:{}", prepared.len());
}

#[cfg(not(feature = "durable"))]
fn main() {
    panic!("this crash preflight requires the durable feature")
}
