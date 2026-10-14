// Generic durable access and cache settings; logical schema and domain policy remain with consumers.
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
    cache_bytes: Option<usize>,
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
            cache_bytes: None,
            access: Mutex::new(Some(Access::Write(database))),
        }
    }
    /// Open existing storage with an explicit redb cache target, preserved on every access transition.
    /// This is backend cache configuration, not a process RSS or file-size quota.
    /// # Errors
    /// Missing/damaged storage is never created or repaired.
    pub fn open_with_cache(path: &Path, cache_bytes: usize) -> Result<Self, Failure> {
        Ok(Self {
            path: path.to_owned(),
            cache_bytes: Some(cache_bytes),
            access: Mutex::new(Some(Access::Read(
                builder(Some(cache_bytes))
                    .open_read_only(path)
                    .map_err(|e| database_error(&e))?,
            ))),
        })
    }
    /// Explicitly create/open writable physical storage with a persistent cache target.
    /// Logical table/schema initialization remains the caller's responsibility.
    /// # Errors
    /// Invalid files, unavailable storage and recovery failures are preserved.
    pub fn create_with_cache(path: &Path, cache_bytes: usize) -> Result<Self, Failure> {
        Ok(Self {
            path: path.to_owned(),
            cache_bytes: Some(cache_bytes),
            access: Mutex::new(Some(Access::Write(
                builder(Some(cache_bytes))
                    .create(path)
                    .map_err(|e| database_error(&e))?,
            ))),
        })
    }
    /// Explicit writable recovery with the same cache target on later transitions.
    /// # Errors
    /// Refuses missing/symlinked/non-regular paths and preserves backend failures.
    pub fn recover_with_cache(path: &Path, cache_bytes: usize) -> Result<Self, Failure> {
        if !std::fs::symlink_metadata(path)
            .map_err(|_| Failure::Storage)?
            .file_type()
            .is_file()
        {
            return Err(Failure::Storage);
        }
        Ok(Self {
            path: path.to_owned(),
            cache_bytes: Some(cache_bytes),
            access: Mutex::new(Some(Access::Write(
                builder(Some(cache_bytes))
                    .open(path)
                    .map_err(|e| database_error(&e))?,
            ))),
        })
    }
    /// The explicitly configured backend target. None preserves redb defaults.
    #[must_use]
    pub fn cache_bytes(&self) -> Option<usize> {
        self.cache_bytes
    }
    /// # Errors
    /// Opening missing or damaged storage never initializes or repairs it.
    pub fn open(path: &Path) -> Result<Self, Failure> {
        Ok(Self {
            path: path.to_owned(),
            cache_bytes: None,
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
                builder(self.cache_bytes)
                    .open_read_only(&self.path)
                    .map_err(|e| database_error(&e))?,
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
                builder(self.cache_bytes)
                    .open(&self.path)
                    .map_err(|e| database_error(&e))?,
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

fn builder(cache_bytes: Option<usize>) -> redb::Builder {
    let mut builder = Database::builder();
    if let Some(bytes) = cache_bytes {
        builder.set_cache_size(bytes);
    }
    builder
}
