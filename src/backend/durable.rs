// Revision foundation 0.10.0: extracted unchanged mechanics; no domain policy.
use super::{Backend, CommitState, Failure};
use redb::{Database, ReadableTable, TableDefinition};
use std::path::Path;

const IMAGE: TableDefinition<&str, &[u8]> = TableDefinition::new("zixcel_commit_image");
const MAX_IMAGE_BYTES: usize = 96 * 1024 * 1024;

pub struct RedbBackend {
    database: crate::durable::Handle,
}
impl RedbBackend {
    /// Explicit physical backend recovery, not a read or domain-state repair.
    /// # Errors
    /// Missing/corrupt logical image remains an error; no schema/table initialization.
    pub fn recover_existing(path: impl AsRef<Path>) -> Result<Self, Failure> {
        let backend = Self {
            database: crate::durable::Handle::recover(path.as_ref())?,
        };
        backend.read(|_| ())?;
        Ok(backend)
    }
    /// Creates exactly a new file. Existing, symlinked or missing parent paths reject.
    /// # Errors
    /// I/O failure or an existing path rejects, without opening it as a fallback.
    pub fn create(path: impl AsRef<Path>) -> Result<Self, Failure> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path.as_ref())
            .map_err(|_| Failure::Storage)?;
        let database = Database::builder()
            .create_file(file)
            .map_err(|_| Failure::Storage)?;
        let write = database.begin_write().map_err(|_| Failure::Storage)?;
        {
            let mut table = write.open_table(IMAGE).map_err(|_| Failure::Storage)?;
            let bytes = encode(&CommitState::default())?;
            table
                .insert("state", bytes.as_slice())
                .map_err(|_| Failure::Storage)?;
        }
        write.commit().map_err(|_| Failure::Storage)?;
        Ok(Self {
            database: crate::durable::Handle::created(path.as_ref(), database),
        })
    }
    /// Opens only an existing complete image. Never creates directories or tables.
    /// # Errors
    /// Invalid paths, corrupt state and I/O failure remain visible to the caller.
    pub fn open_existing(path: impl AsRef<Path>) -> Result<Self, Failure> {
        if !std::fs::symlink_metadata(path.as_ref())
            .map_err(|_| Failure::Storage)?
            .file_type()
            .is_file()
        {
            return Err(Failure::Storage);
        }
        let backend = Self {
            database: crate::durable::Handle::open(path.as_ref())?,
        };
        backend.read(|_| ())?;
        Ok(backend)
    }
}
impl Backend for RedbBackend {
    fn read<T>(&self, read: impl FnOnce(&CommitState) -> T) -> Result<T, Failure> {
        self.database.read(|tx| {
            let table = tx.open_table(IMAGE).map_err(|_| Failure::Corrupt)?;
            let bytes = table
                .get("state")
                .map_err(|_| Failure::Storage)?
                .ok_or(Failure::Corrupt)?;
            let image = decode(bytes.value())?;
            Ok(read(&image))
        })
    }
    fn write<T>(
        &self,
        change: impl FnOnce(&mut CommitState) -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        self.database.write(|database| {
            let tx = database.begin_write().map_err(|_| Failure::Storage)?;
            let result;
            {
                // Opening IMAGE here cannot silently initialize: an existing image is required.
                let mut table = tx.open_table(IMAGE).map_err(|_| Failure::Storage)?;
                let (mut state, previous) = {
                    let bytes = table
                        .get("state")
                        .map_err(|_| Failure::Storage)?
                        .ok_or(Failure::Corrupt)?;
                    (decode(bytes.value())?, bytes.value().to_vec())
                };
                result = change(&mut state)?;
                state.validate()?;
                let bytes = encode(&state)?;
                if bytes == previous {
                    return Ok(result);
                }
                table
                    .insert("state", bytes.as_slice())
                    .map_err(|_| Failure::Storage)?;
            }
            tx.commit().map_err(|_| Failure::Storage)?;
            Ok(result)
        })
    }
}
fn encode(state: &CommitState) -> Result<Vec<u8>, Failure> {
    let bytes = serde_json::to_vec(state).map_err(|_| Failure::Corrupt)?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(Failure::Capacity);
    }
    Ok(bytes)
}
fn decode(bytes: &[u8]) -> Result<CommitState, Failure> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(Failure::Capacity);
    }
    let state: CommitState = serde_json::from_slice(bytes).map_err(|_| Failure::Corrupt)?;
    state.validate()?;
    Ok(state)
}
