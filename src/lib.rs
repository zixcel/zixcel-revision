// Revision foundation 0.10.0: extracted unchanged mechanics; no domain policy.
//! Deterministic, domain-neutral commit control. No validation of payload meaning.
//!
//! Owners validate and authorize before preparing. Canonical payload bytes must
//! have a stable owner-defined encoding; timestamps/nonces are not added here.
//! Backends atomically publish content, head and receipt. No external callback,
//! provider effect, semantic policy, graph type or network service participates.
pub mod backend;
mod external_port;
mod model;
pub use external_port::ExternalRetention;
mod registration;
mod retention;
mod state;
mod store;
pub use registration::{ExternalRegistration, ExternalRegistrationStatus, RetirementReceipt};
pub use retention::{ExternalObjectRef, ExternalRoot, ExternalRootKind, ReclamationPermit};

#[cfg(feature = "redb")]
pub use backend::RedbBackend;
pub use backend::{Backend, MemoryBackend};
pub use model::*;
pub use state::CommitState;
pub use store::CommitStore;

pub mod attestation;

/// Shared physical backend handle; no domain interpretation or separate database.
#[cfg(feature = "redb")]
pub mod durable;

#[cfg(test)]
mod tests;
