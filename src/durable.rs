// Revision foundation 0.10.0: extracted unchanged mechanics; no domain policy.
//! C4: read-only open; writable backend recovery is acquired only by an explicit mutation.
use crate::Failure;
use redb::{Database, ReadOnlyDatabase, ReadableDatabase};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

enum Access {
    Read(ReadOnlyDatabase),
    Write(Database),
}
pub struct Handle {
    path: PathBuf,
    access: Mutex<Option<Access>>,
}
impl Handle {
    /// Explicit recovery; never initializes missing or corrupt logical objects.
    /// # Errors
    /// Invalid files or backend recovery failures are preserved.
    pub fn recover(path: &Path) -> Result<Self, Failure> {
        if !std::fs::symlink_metadata(path)
            .map_err(|_| Failure::Storage)?
            .file_type()
            .is_file()
        {
            return Err(Failure::Storage);
        }
        Ok(Self::created(
            path,
            Database::open(path).map_err(|e| database_error(&e))?,
        ))
    }
    #[must_use]
    pub fn created(path: &Path, database: Database) -> Self {
        Self {
            path: path.to_owned(),
            access: Mutex::new(Some(Access::Write(database))),
        }
    }
    /// # Errors
    /// Opening missing or damaged storage never initializes or repairs it.
    pub fn open(path: &Path) -> Result<Self, Failure> {
        Ok(Self {
            path: path.to_owned(),
            access: Mutex::new(Some(Access::Read(
                ReadOnlyDatabase::open(path).map_err(|e| database_error(&e))?,
            ))),
        })
    }
    /// # Errors
    /// Propagates backend and caller validation failures.
    pub fn read<T, E: From<Failure>>(
        &self,
        f: impl FnOnce(&redb::ReadTransaction) -> Result<T, E>,
    ) -> Result<T, E> {
        let mut guard = self.access.lock().map_err(|_| Failure::Storage)?;
        if guard.is_none() {
            *guard = Some(Access::Read(
                ReadOnlyDatabase::open(&self.path).map_err(|e| database_error(&e))?,
            ));
        }
        let tx = match guard.as_ref().ok_or(Failure::Storage)? {
            Access::Read(d) => d.begin_read(),
            Access::Write(d) => d.begin_read(),
        }
        .map_err(|_| Failure::Storage)?;
        f(&tx)
    }
    /// # Errors
    /// Propagates backend and caller transaction failures.
    pub fn write<T, E: From<Failure>>(
        &self,
        f: impl FnOnce(&Database) -> Result<T, E>,
    ) -> Result<T, E> {
        let mut guard = self.access.lock().map_err(|_| Failure::Storage)?;
        if !matches!(guard.as_ref(), Some(Access::Write(_))) {
            // No read transaction escapes the handle. Never steal a foreign reader's lock.
            drop(guard.take());
            *guard = Some(Access::Write(
                Database::open(&self.path).map_err(|e| database_error(&e))?,
            ));
        }
        match guard.as_ref().ok_or(Failure::Storage)? {
            Access::Write(d) => f(d),
            Access::Read(_) => Err(Failure::Storage.into()),
        }
    }
}
fn database_error(error: &redb::DatabaseError) -> Failure {
    match error {
        redb::DatabaseError::RepairAborted => Failure::RecoveryRequired,
        redb::DatabaseError::UpgradeRequired(_)
        | redb::DatabaseError::Storage(redb::StorageError::Corrupted(_)) => Failure::Corrupt,
        _ => Failure::Storage,
    }
}
