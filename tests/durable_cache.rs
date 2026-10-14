#![cfg(feature = "redb")]
use redb::TableDefinition;
use zixcel_revision::{Failure, durable::Handle};
const DATA: TableDefinition<&str, u64> = TableDefinition::new("cache-fixture");
fn read(handle: &Handle) -> Result<u64, Failure> {
    handle.read(|tx| {
        let table = tx.open_table(DATA).map_err(|_| Failure::Storage)?;
        Ok(table
            .get("value")
            .map_err(|_| Failure::Storage)?
            .ok_or(Failure::Storage)?
            .value())
    })
}
fn write(handle: &Handle, value: u64) -> Result<(), Failure> {
    handle.write(|database| {
        let tx = database.begin_write().map_err(|_| Failure::Storage)?;
        {
            let mut table = tx.open_table(DATA).map_err(|_| Failure::Storage)?;
            table.insert("value", value).map_err(|_| Failure::Storage)?;
        }
        tx.commit().map_err(|_| Failure::Storage)
    })
}
#[test]
fn zero_and_finite_cache_targets_survive_read_write_transitions() {
    for cache in [0, 1024 * 1024] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.redb");
        let handle = Handle::create_with_cache(&path, cache).unwrap();
        write(&handle, 41).unwrap();
        assert_eq!(handle.cache_bytes(), Some(cache));
        drop(handle);
        let handle = Handle::open_with_cache(&path, cache).unwrap();
        assert_eq!(read(&handle).unwrap(), 41);
        write(&handle, 42).unwrap();
        assert_eq!(read(&handle).unwrap(), 42);
        assert_eq!(handle.cache_bytes(), Some(cache));
        drop(handle);
        let recovered = Handle::recover_with_cache(&path, cache).unwrap();
        assert_eq!(read(&recovered).unwrap(), 42);
        assert_eq!(recovered.cache_bytes(), Some(cache));
    }
}
#[test]
fn opening_does_not_initialize_and_foreign_writer_refusal_does_not_lose_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.redb");
    assert!(Handle::open_with_cache(&missing, 0).is_err());
    assert!(Handle::recover_with_cache(&missing, 0).is_err());
    assert!(!missing.exists());
    let path = dir.path().join("fixture.redb");
    let initial = Handle::create_with_cache(&path, 0).unwrap();
    write(&initial, 7).unwrap();
    drop(initial);
    let first = Handle::open_with_cache(&path, 0).unwrap();
    let second = Handle::open_with_cache(&path, 0).unwrap();
    assert_eq!(read(&first).unwrap(), 7);
    assert_eq!(read(&second).unwrap(), 7);
    assert!(write(&first, 99).is_err());
    assert_eq!(first.cache_bytes(), Some(0));
    assert_eq!(read(&first).unwrap(), 7);
    drop(second);
    write(&first, 8).unwrap();
    assert_eq!(read(&first).unwrap(), 8);
}
