// Revision foundation 0.10.0: extracted unchanged mechanics; no domain policy.
//! Backends provide one atomic fence, not their own CAS or replay implementation.
use super::{CommitState, Failure};
use std::sync::RwLock;

/// An implementation MUST roll back all mutation when a closure returns Err.
/// Reads never create state. The write fence covers the entire closure and publication.
/// Callbacks are internal bounded commit mechanics, never domain/external effects.
pub trait Backend: Send + Sync {
    /// # Errors
    /// Missing/corrupt storage is an error, never implicit initialization.
    fn read<T>(&self, read: impl FnOnce(&CommitState) -> T) -> Result<T, Failure>;
    /// # Errors
    /// Failed closure or persistence must leave the prior image and receipts intact.
    fn write<T>(
        &self,
        write: impl FnOnce(&mut CommitState) -> Result<T, Failure>,
    ) -> Result<T, Failure>;
}

#[derive(Default)]
pub struct MemoryBackend {
    state: RwLock<CommitState>,
}
impl Backend for MemoryBackend {
    fn read<T>(&self, read: impl FnOnce(&CommitState) -> T) -> Result<T, Failure> {
        let state = self.state.read().map_err(|_| Failure::Storage)?;
        Ok(read(&state))
    }
    fn write<T>(
        &self,
        write: impl FnOnce(&mut CommitState) -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        let mut guard = self.state.write().map_err(|_| Failure::Storage)?;
        let mut proposed = guard.clone();
        let result = write(&mut proposed)?;
        proposed.validate()?;
        *guard = proposed;
        Ok(result)
    }
}

#[cfg(feature = "redb")]
mod durable;
#[cfg(feature = "redb")]
pub use durable::RedbBackend;
