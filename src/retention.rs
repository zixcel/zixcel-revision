// Revision foundation 0.10.0: extracted unchanged mechanics; no domain policy.
//! C4 external ownership protocol. No filesystem access or domain semantics.
use super::{Backend, CommitState, CommitStore, Failure, PreparedCommit, Rejection};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
const MAX_EXTERNAL: usize = 256;
const MAX_ROOTS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalObjectRef {
    pub owner: String,
    pub object: String,
}
impl ExternalObjectRef {
    pub(super) fn key(&self) -> Result<String, Failure> {
        if [&self.owner, &self.object]
            .iter()
            .any(|v| v.is_empty() || v.len() > 512 || v.chars().any(char::is_control))
        {
            return Err(Failure::Rejected(Rejection::InvalidIntent));
        }
        Ok(format!(
            "{}:{}{}",
            self.owner.len(),
            self.owner,
            self.object
        ))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ExternalRootKind {
    Retained,
    InFlight,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalRoot {
    pub kind: ExternalRootKind,
    pub holder: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReclamationPermit {
    pub object: ExternalObjectRef,
    pub sequence: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Tracked {
    pub(super) object: ExternalObjectRef,
    pub prepared: BTreeMap<String, super::registration::Registration>,
    pub(super) roots: BTreeSet<ExternalRoot>,
    pub(super) permit: Option<u64>,
}
impl CommitState {
    pub(super) fn validate_external(&self) -> Result<(), Failure> {
        if self.external.len() > MAX_EXTERNAL {
            return Err(Failure::Capacity);
        }
        self.validate_registrations()?;
        for (key, e) in &self.external {
            if &e.object.key().map_err(|_| Failure::Corrupt)? != key
                || e.prepared.is_empty()
                || e.prepared.len() > MAX_ROOTS
                || e.roots.len() > MAX_ROOTS
                || e.roots.iter().any(|r| !valid_root(r))
                || e.permit
                    .is_some_and(|n| n == 0 || n > self.reclamation_sequence)
            {
                return Err(Failure::Corrupt);
            }
            for r in e.prepared.keys() {
                if !self.staged.contains_key(r)
                    && !self.commits.values().any(|c| c.prepared.reference() == r)
                {
                    return Err(Failure::Corrupt);
                }
            }
            if e.permit.is_some() && !self.external_eligible(e, u64::MAX) {
                return Err(Failure::Corrupt);
            }
        }
        Ok(())
    }
    fn external_eligible(&self, e: &Tracked, epoch: u64) -> bool {
        e.roots.is_empty()
            && e.prepared.iter().all(|(r, registration)| {
                registration.roots.is_empty()
                    && (registration.retired
                        || self
                            .staged
                            .get(r)
                            .is_some_and(|p| p.reclaim_after.is_some_and(|after| after <= epoch)))
            })
    }
}
pub(super) fn valid_root(r: &ExternalRoot) -> bool {
    !r.holder.is_empty() && r.holder.len() <= 512 && !r.holder.chars().any(char::is_control)
}
impl<B: Backend> CommitStore<B> {
    /// Reserve immutable owner references BEFORE the owner creates external bytes.
    /// A crash before registration finishes cannot expose external data as canonical.
    /// # Errors
    /// Rejects invalid/abandoned intent, capacity or a reference already being reclaimed.
    pub fn reserve_external(
        &self,
        p: &PreparedCommit,
        objects: &[ExternalObjectRef],
    ) -> Result<(), Failure> {
        if objects.is_empty() || objects.len() > MAX_EXTERNAL {
            return Err(Failure::Rejected(Rejection::InvalidIntent));
        }
        let keys = objects
            .iter()
            .map(ExternalObjectRef::key)
            .collect::<Result<Vec<_>, _>>()?;
        self.stage(p)?;
        self.backend.write(|s| {
            // Standalone stage/reserve are two fences. Cancellation in between
            // must reject before the owner is permitted to create external bytes.
            if s.retired
                .contains_key(&super::state::operation_key(p.domain(), p.operation_id()))
                || s.staged
                    .get(p.reference())
                    .is_some_and(|v| v.reclaim_after.is_some())
            {
                return Err(Failure::Rejected(Rejection::Abandoned));
            }
            for (object, key) in objects.iter().zip(keys) {
                if !s.external.contains_key(&key)
                    && s.retirement_receipts.values().any(|r| r.object == *object)
                {
                    return Err(Failure::Rejected(Rejection::Reclaiming));
                }
                let needs_generation = !s
                    .external
                    .get(&key)
                    .is_some_and(|e| e.prepared.contains_key(p.reference()));
                let generation = if needs_generation {
                    s.next_registration_generation()?
                } else {
                    0
                };
                let entry = s.external.entry(key).or_insert_with(|| Tracked {
                    object: object.clone(),
                    prepared: BTreeMap::new(),
                    roots: BTreeSet::new(),
                    permit: None,
                });
                if entry.permit.is_some() {
                    return Err(Failure::Rejected(Rejection::Reclaiming));
                }
                if entry.prepared.get(p.reference()).is_some_and(|r| r.retired) {
                    return Err(Failure::Rejected(Rejection::Reclaiming));
                }
                entry
                    .prepared
                    .entry(p.reference().into())
                    .or_insert_with(|| super::registration::Registration::new(generation, None));
            }
            s.validate_external()
        })
    }
    /// Explicit durable protection; a crashed caller must explicitly release its root.
    /// # Errors
    /// Unknown/reclaiming objects, malformed holders and capacity reject.
    pub fn retain_external(
        &self,
        object: &ExternalObjectRef,
        root: &ExternalRoot,
    ) -> Result<(), Failure> {
        if !valid_root(root) {
            return Err(Failure::Rejected(Rejection::InvalidIntent));
        }
        let key = object.key()?;
        self.backend.write(|s| {
            let e = s
                .external
                .get_mut(&key)
                .ok_or(Failure::Rejected(Rejection::NotPrepared))?;
            if e.permit.is_some() || e.prepared.values().any(|r| r.retired) {
                return Err(Failure::Rejected(Rejection::Reclaiming));
            }
            e.roots.insert(root.clone());
            s.validate_external()
        })
    }
    /// # Errors
    /// Unknown or reclaiming objects fail without releasing another holder's root.
    pub fn release_external(
        &self,
        object: &ExternalObjectRef,
        root: &ExternalRoot,
    ) -> Result<(), Failure> {
        let key = object.key()?;
        self.backend.write(|s| {
            let e = s
                .external
                .get_mut(&key)
                .ok_or(Failure::Rejected(Rejection::NotPrepared))?;
            if e.permit.is_some() {
                return Err(Failure::Rejected(Rejection::Reclaiming));
            }
            e.roots.remove(root);
            Ok(())
        })
    }
    /// Pure eligibility inspection. No TTL, deletion, implicit abandonment or lock stealing.
    /// # Errors
    /// Malformed reference or backend failure is not treated as eligibility.
    pub fn external_reclaimable(
        &self,
        object: &ExternalObjectRef,
        epoch: u64,
    ) -> Result<bool, Failure> {
        let key = object.key()?;
        self.backend.read(|s| {
            s.external
                .get(&key)
                .is_some_and(|e| s.external_eligible(e, epoch))
        })
    }
    /// Atomically fence new roots/reservations before OWNER-controlled deletion.
    /// No physical deletion occurs here. Retry returns the original durable permit.
    /// # Errors
    /// Invalid reference, sequence exhaustion or storage error rejects.
    pub fn claim_external_reclamation(
        &self,
        object: &ExternalObjectRef,
        epoch: u64,
    ) -> Result<Option<ReclamationPermit>, Failure> {
        let key = object.key()?;
        self.backend.write(|s| {
            let Some(e) = s.external.get(&key) else {
                return Ok(None);
            };
            if let Some(sequence) = e.permit {
                return Ok(Some(ReclamationPermit {
                    object: object.clone(),
                    sequence,
                }));
            }
            if !s.external_eligible(e, epoch) {
                return Ok(None);
            }
            s.reclamation_sequence = s
                .reclamation_sequence
                .checked_add(1)
                .ok_or(Failure::Capacity)?;
            let sequence = s.reclamation_sequence;
            s.external.get_mut(&key).ok_or(Failure::Corrupt)?.permit = Some(sequence);
            Ok(Some(ReclamationPermit {
                object: object.clone(),
                sequence,
            }))
        })
    }
    /// Owner acknowledgement after idempotent physical reclamation. Owner must
    /// not delete through a stale permit or reuse an immutable object identity.
    /// # Errors
    /// A substituted/stale permit fails; absence is an already-acknowledged result.
    pub fn complete_external_reclamation(&self, permit: &ReclamationPermit) -> Result<(), Failure> {
        let key = permit.object.key()?;
        self.backend.write(|s| {
            if let Some(e) = s.external.get(&key) {
                if e.permit != Some(permit.sequence) {
                    return Err(Failure::Rejected(Rejection::Reclaiming));
                }
                s.external.remove(&key);
            }
            Ok(())
        })
    }
}
