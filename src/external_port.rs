// Revision foundation 0.10.0: extracted unchanged mechanics; no domain policy.
//! A lifecycle-only port shared by canonical owners using different physical stores.
//! It cannot publish a commit, rewrite a DAG or interpret the external objects.
use super::{
    Backend, CommitReceipt, CommitStore, ExternalObjectRef, ExternalRegistration,
    ExternalRegistrationStatus, ExternalRoot, Failure, ReclamationPermit, RetirementReceipt,
};

pub trait ExternalRetention {
    /// # Errors
    /// Exact receipt, registration and storage failures are preserved.
    fn register_committed_external(
        &self,
        receipt: &CommitReceipt,
        objects: &[ExternalObjectRef],
    ) -> Result<Vec<ExternalRegistration>, Failure>;
    /// # Errors
    /// Malformed references and storage failures are preserved.
    fn external_registration(
        &self,
        object: &ExternalObjectRef,
        prepared: &str,
    ) -> Result<Option<ExternalRegistration>, Failure>;
    /// # Errors
    /// Unknown/substituted generations and storage failures are preserved.
    fn registration_status(
        &self,
        registration: &ExternalRegistration,
    ) -> Result<ExternalRegistrationStatus, Failure>;
    /// # Errors
    /// Stale generations, active reclamation and storage failures are preserved.
    fn retire_external_registration(
        &self,
        registration: &ExternalRegistration,
    ) -> Result<RetirementReceipt, Failure>;
    /// # Errors
    /// Retirement, generation, capacity and storage failures are preserved.
    fn retain_registration(
        &self,
        registration: &ExternalRegistration,
        root: &ExternalRoot,
    ) -> Result<(), Failure>;
    /// # Errors
    /// Stale/substituted generations and storage failures are preserved.
    fn release_registration(
        &self,
        registration: &ExternalRegistration,
        root: &ExternalRoot,
    ) -> Result<(), Failure>;
    /// # Errors
    /// Malformed references, capacity and storage failures are preserved.
    fn claim_external_reclamation(
        &self,
        object: &ExternalObjectRef,
        epoch: u64,
    ) -> Result<Option<ReclamationPermit>, Failure>;
    /// # Errors
    /// Stale permits and storage failures are preserved.
    fn complete_external_reclamation(&self, permit: &ReclamationPermit) -> Result<(), Failure>;
}

impl<B: Backend> ExternalRetention for CommitStore<B> {
    fn register_committed_external(
        &self,
        receipt: &CommitReceipt,
        objects: &[ExternalObjectRef],
    ) -> Result<Vec<ExternalRegistration>, Failure> {
        self.register_committed_external(receipt, objects)
    }
    fn external_registration(
        &self,
        object: &ExternalObjectRef,
        prepared: &str,
    ) -> Result<Option<ExternalRegistration>, Failure> {
        self.external_registration(object, prepared)
    }
    fn registration_status(
        &self,
        registration: &ExternalRegistration,
    ) -> Result<ExternalRegistrationStatus, Failure> {
        self.registration_status(registration)
    }
    fn retire_external_registration(
        &self,
        registration: &ExternalRegistration,
    ) -> Result<RetirementReceipt, Failure> {
        self.retire_external_registration(registration)
    }
    fn retain_registration(
        &self,
        registration: &ExternalRegistration,
        root: &ExternalRoot,
    ) -> Result<(), Failure> {
        self.retain_registration(registration, root)
    }
    fn release_registration(
        &self,
        registration: &ExternalRegistration,
        root: &ExternalRoot,
    ) -> Result<(), Failure> {
        self.release_registration(registration, root)
    }
    fn claim_external_reclamation(
        &self,
        object: &ExternalObjectRef,
        epoch: u64,
    ) -> Result<Option<ReclamationPermit>, Failure> {
        self.claim_external_reclamation(object, epoch)
    }
    fn complete_external_reclamation(&self, permit: &ReclamationPermit) -> Result<(), Failure> {
        self.complete_external_reclamation(permit)
    }
}
