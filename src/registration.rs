// Revision foundation 0.10.0: extracted unchanged mechanics; no domain policy.
//! ZG1: explicit association retirement; no receipt expiry or domain policy.
use super::retention::valid_root;
use super::{
    Backend, CommitState, CommitStore, ExternalObjectRef, ExternalRoot, Failure, Rejection,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Exact, non-reused association generation. Not a commit or authorization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalRegistration {
    pub object: ExternalObjectRef,
    pub prepared_ref: String,
    pub generation: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommitIntent, CommitOutcome, MemoryBackend, RevisionRef};

    #[test]
    fn generation_and_hold_capacity_fail_before_partial_lifecycle_mutation() {
        let store = CommitStore::new(MemoryBackend::default());
        let intent = CommitIntent {
            domain: "owner".into(),
            operation_id: "source".into(),
            expected_revision: RevisionRef::default(),
            parents: vec![],
            payload: vec![],
        }
        .prepare()
        .unwrap();
        let object = ExternalObjectRef {
            owner: "owner".into(),
            object: "source".into(),
        };
        store
            .reserve_external(&intent, std::slice::from_ref(&object))
            .unwrap();
        assert!(matches!(store.commit(&intent), CommitOutcome::Committed(_)));
        let registration = store
            .external_registration(&object, intent.reference())
            .unwrap()
            .unwrap();
        for index in 0..64 {
            store
                .retain_registration(
                    &registration,
                    &ExternalRoot {
                        kind: super::super::ExternalRootKind::InFlight,
                        holder: format!("reader-{index}"),
                    },
                )
                .unwrap();
        }
        let overflow = ExternalRoot {
            kind: super::super::ExternalRootKind::Retained,
            holder: "overflow".into(),
        };
        assert_eq!(
            store.retain_registration(&registration, &overflow),
            Err(Failure::Capacity)
        );
        assert_eq!(
            store
                .backend
                .read(|s| s
                    .external
                    .values()
                    .next()
                    .unwrap()
                    .prepared
                    .values()
                    .next()
                    .unwrap()
                    .roots
                    .len())
                .unwrap(),
            64
        );
        // Exhaustion cannot wrap to an old generation, even for a different object.
        store
            .backend
            .write(|s| {
                s.registration_sequence = u64::MAX;
                Ok(())
            })
            .unwrap();
        let other = ExternalObjectRef {
            object: "other".into(),
            ..object
        };
        assert_eq!(
            store.reserve_external(&intent, std::slice::from_ref(&other)),
            Err(Failure::Capacity)
        );
        assert!(
            store
                .external_registration(&other, intent.reference())
                .unwrap()
                .is_none()
        );
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetirementReceipt {
    pub registration: ExternalRegistration,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalRegistrationStatus {
    Active,
    Retiring,
    Retired,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Registration {
    pub generation: u64,
    previous: Option<u64>,
    pub retired: bool,
    pub roots: BTreeSet<ExternalRoot>,
}
impl Registration {
    pub(super) fn new(generation: u64, previous: Option<u64>) -> Self {
        Self {
            generation,
            previous,
            retired: false,
            roots: BTreeSet::new(),
        }
    }
}
fn missing() -> Failure {
    Failure::Rejected(Rejection::NotPrepared)
}
fn stale() -> Failure {
    Failure::Rejected(Rejection::InvalidIntent)
}
fn retiring() -> Failure {
    Failure::Rejected(Rejection::Reclaiming)
}

impl CommitState {
    pub(super) fn next_registration_generation(&mut self) -> Result<u64, Failure> {
        self.registration_sequence = self
            .registration_sequence
            .checked_add(1)
            .ok_or(Failure::Capacity)?;
        Ok(self.registration_sequence)
    }
    pub(super) fn validate_registrations(&self) -> Result<(), Failure> {
        if self.retirement_receipts.len() > super::MAX_COMMITS {
            return Err(Failure::Capacity);
        }
        let mut generations = BTreeSet::new();
        for (key, r) in &self.retirement_receipts {
            if *key != r.generation
                || *key == 0
                || *key > self.registration_sequence
                || r.object.key().is_err()
                || !self
                    .commits
                    .values()
                    .any(|c| c.prepared.reference() == r.prepared_ref)
            {
                return Err(Failure::Corrupt);
            }
        }
        for e in self.external.values() {
            if e.roots.len() + e.prepared.values().map(|r| r.roots.len()).sum::<usize>() > 64 {
                return Err(Failure::Capacity);
            }
            for (prepared, r) in &e.prepared {
                let reference = ExternalRegistration {
                    object: e.object.clone(),
                    prepared_ref: prepared.clone(),
                    generation: r.generation,
                };
                if r.generation == 0
                    || r.generation > self.registration_sequence
                    || !generations.insert(r.generation)
                    || r.roots.iter().any(|h| !valid_root(h))
                    || self
                        .retirement_receipts
                        .get(&r.generation)
                        .is_some_and(|v| *v != reference)
                    || r.retired
                        != self
                            .retirement_receipts
                            .get(&r.generation)
                            .is_some_and(|v| *v == reference)
                    || r.previous.is_some_and(|old| {
                        old >= r.generation
                            || !self.retirement_receipts.get(&old).is_some_and(|v| {
                                v.object == e.object && v.prepared_ref == *prepared
                            })
                    })
                {
                    return Err(Failure::Corrupt);
                }
            }
        }
        Ok(())
    }
    fn registration_mut(
        &mut self,
        reference: &ExternalRegistration,
    ) -> Result<&mut Registration, Failure> {
        let e = self
            .external
            .get_mut(&reference.object.key()?)
            .ok_or_else(missing)?;
        if e.permit.is_some() {
            return Err(retiring());
        }
        let r = e
            .prepared
            .get_mut(&reference.prepared_ref)
            .ok_or_else(missing)?;
        if r.generation != reference.generation {
            return Err(stale());
        }
        Ok(r)
    }
}
impl<B: Backend> CommitStore<B> {
    /// Explicit late registration against an exact original canonical receipt.
    /// Owners validate availability/integrity of their dependency closure first.
    /// # Errors
    /// Missing/substituted receipt, reclaiming references or capacity reject.
    pub fn register_committed_external(
        &self,
        receipt: &super::CommitReceipt,
        objects: &[ExternalObjectRef],
    ) -> Result<Vec<ExternalRegistration>, Failure> {
        let prepared = self
            .backend
            .read(|s| {
                s.commits
                    .get(&receipt.commit_ref)
                    .filter(|c| c.receipt == *receipt)
                    .map(|c| c.prepared.clone())
            })?
            .ok_or_else(missing)?;
        self.reserve_external(&prepared, objects)?;
        objects
            .iter()
            .map(|object| {
                self.external_registration(object, prepared.reference())?
                    .ok_or_else(missing)
            })
            .collect()
    }
    /// Exact lookup without registration, source access or initialization.
    /// # Errors
    /// Invalid references and backend failures remain explicit.
    pub fn external_registration(
        &self,
        object: &ExternalObjectRef,
        prepared: &str,
    ) -> Result<Option<ExternalRegistration>, Failure> {
        let key = object.key()?;
        self.backend.read(|s| {
            s.external
                .get(&key)
                .and_then(|e| e.prepared.get(prepared))
                .map(|r| ExternalRegistration {
                    object: object.clone(),
                    prepared_ref: prepared.into(),
                    generation: r.generation,
                })
        })
    }
    /// # Errors
    /// Unknown/substituted generations reject; original retired generations remain inspectable.
    pub fn registration_status(
        &self,
        reference: &ExternalRegistration,
    ) -> Result<ExternalRegistrationStatus, Failure> {
        let key = reference.object.key()?;
        self.backend.read(|s| {
            if let Some(e) = s.external.get(&key)
                && let Some(r) = e.prepared.get(&reference.prepared_ref)
                && r.generation == reference.generation
            {
                return Ok(if !r.retired {
                    ExternalRegistrationStatus::Active
                } else if !r.roots.is_empty() || !e.roots.is_empty() {
                    ExternalRegistrationStatus::Retiring
                } else {
                    ExternalRegistrationStatus::Retired
                });
            }
            if s.retirement_receipts.get(&reference.generation) == Some(reference) {
                Ok(ExternalRegistrationStatus::Retired)
            } else {
                Err(missing())
            }
        })?
    }
    /// Establishes the retirement fence, never removes existing holds. Replay
    /// returns the original receipt even after re-registration or physical reclaim.
    /// # Errors
    /// Only exact committed associations may retire; stale refs and capacity reject.
    pub fn retire_external_registration(
        &self,
        reference: &ExternalRegistration,
    ) -> Result<RetirementReceipt, Failure> {
        reference.object.key()?;
        self.backend.write(|s| {
            if let Some(original) = s.retirement_receipts.get(&reference.generation) {
                return if original == reference {
                    Ok(RetirementReceipt {
                        registration: original.clone(),
                    })
                } else {
                    Err(stale())
                };
            }
            if !s
                .commits
                .values()
                .any(|c| c.prepared.reference() == reference.prepared_ref)
            {
                return Err(missing());
            }
            if s.retirement_receipts.len() >= super::MAX_COMMITS {
                return Err(Failure::Capacity);
            }
            s.registration_mut(reference)?.retired = true;
            s.retirement_receipts
                .insert(reference.generation, reference.clone());
            s.validate_external()?;
            Ok(RetirementReceipt {
                registration: reference.clone(),
            })
        })
    }
    /// Explicit replacement while external bytes still exist. Owner must validate
    /// them first. Never resurrects a physically reclaimed immutable identity.
    /// # Errors
    /// Active/held/reclaiming or stale associations reject. Immediate retry replays B.
    pub fn reregister_external(
        &self,
        previous: &ExternalRegistration,
    ) -> Result<ExternalRegistration, Failure> {
        let key = previous.object.key()?;
        self.backend.write(|s| {
            if s.retirement_receipts.get(&previous.generation) != Some(previous) {
                return Err(stale());
            }
            let e = s.external.get(&key).ok_or_else(missing)?;
            if e.permit.is_some() || !e.roots.is_empty() {
                return Err(retiring());
            }
            let current = e.prepared.get(&previous.prepared_ref).ok_or_else(missing)?;
            if current.previous == Some(previous.generation) {
                return Ok(ExternalRegistration {
                    generation: current.generation,
                    ..previous.clone()
                });
            }
            if current.generation != previous.generation {
                return Err(stale());
            }
            if !current.retired || !current.roots.is_empty() {
                return Err(retiring());
            }
            let generation = s.next_registration_generation()?;
            s.external
                .get_mut(&key)
                .ok_or_else(missing)?
                .prepared
                .insert(
                    previous.prepared_ref.clone(),
                    Registration::new(generation, Some(previous.generation)),
                );
            s.validate_external()?;
            Ok(ExternalRegistration {
                generation,
                ..previous.clone()
            })
        })
    }
    /// Acquire under a specific registration fence, including shared objects.
    /// # Errors
    /// Retired/reclaiming, stale, unknown and capacity failures never create a hold.
    pub fn retain_registration(
        &self,
        reference: &ExternalRegistration,
        root: &ExternalRoot,
    ) -> Result<(), Failure> {
        if !valid_root(root) {
            return Err(stale());
        }
        self.backend.write(|s| {
            let r = s.registration_mut(reference)?;
            if r.retired {
                return Err(retiring());
            }
            r.roots.insert(root.clone());
            s.validate_external()
        })
    }
    /// Idempotent release of this generation only. Does not retire registration.
    /// # Errors
    /// Stale/substituted generations cannot release another reader's hold.
    pub fn release_registration(
        &self,
        reference: &ExternalRegistration,
        root: &ExternalRoot,
    ) -> Result<(), Failure> {
        if !valid_root(root) {
            return Err(stale());
        }
        self.backend.write(|s| {
            s.registration_mut(reference)?.roots.remove(root);
            Ok(())
        })
    }
}
