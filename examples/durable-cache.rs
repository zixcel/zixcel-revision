//! Temporary physical-store cache/read/write playground; no product or cloud state.
use redb::TableDefinition;
use zixcel_revision::{Failure, durable::Handle};
const DATA: TableDefinition<&str, u64> = TableDefinition::new("synthetic-cache-playground");
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("fixture.redb");
    let cache_bytes = 1024 * 1024;
    let handle = Handle::create_with_cache(&path, cache_bytes)?;
    store(&handle, 1)?;
    drop(handle);
    let handle = Handle::open_with_cache(&path, cache_bytes)?;
    assert_eq!(load(&handle)?, 1);
    store(&handle, 2)?;
    assert_eq!(load(&handle)?, 2);
    assert_eq!(handle.cache_bytes(), Some(cache_bytes));
    println!(
        "synthetic cache target preserved across read/write transition; no RSS/disk guarantee"
    );
    Ok(())
}
fn store(handle: &Handle, value: u64) -> Result<(), Failure> {
    handle.write(|database| {
        let tx = database.begin_write().map_err(|_| Failure::Storage)?;
        {
            let mut table = tx.open_table(DATA).map_err(|_| Failure::Storage)?;
            table.insert("value", value).map_err(|_| Failure::Storage)?;
        }
        tx.commit().map_err(|_| Failure::Storage)
    })
}
fn load(handle: &Handle) -> Result<u64, Failure> {
    handle.read(|tx| {
        let table = tx.open_table(DATA).map_err(|_| Failure::Storage)?;
        Ok(table
            .get("value")
            .map_err(|_| Failure::Storage)?
            .ok_or(Failure::Storage)?
            .value())
    })
}
