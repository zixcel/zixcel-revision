// Revision foundation 0.10.0: extracted unchanged mechanics; no domain policy.
use super::model::{
    CommitReceipt, CommitRef, Failure, MAX_COMMITS, MAX_PREPARED, MAX_STORE_BYTES, PreparedCommit,
    Rejection, RevisionRef, StoreStats,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Staged {
    pub prepared: PreparedCommit,
    // None is Prepared (never reclaimed); Some is explicitly Abandoned.
    pub reclaim_after: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Committed {
    pub prepared: PreparedCommit,
    pub receipt: CommitReceipt,
}

/// Backend-neutral image. Fields stay private; validation is mandatory on decode.
/// No domain payload interpretation is performed, only commit integrity.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitState {
    pub(super) external: BTreeMap<String, super::retention::Tracked>,
    pub(super) reclamation_sequence: u64,
    pub(super) registration_sequence: u64,
    pub(super) retirement_receipts: BTreeMap<u64, super::ExternalRegistration>,
    pub(super) retired: BTreeMap<String, String>,
    format: u32,
    pub(super) heads: BTreeMap<String, RevisionRef>,
    pub(super) commits: BTreeMap<CommitRef, Committed>,
    pub(super) staged: BTreeMap<String, Staged>,
    pub(super) operations: BTreeMap<String, CommitRef>,
}
impl Default for CommitState {
    fn default() -> Self {
        Self {
            external: BTreeMap::new(),
            reclamation_sequence: 0,
            registration_sequence: 0,
            retirement_receipts: BTreeMap::new(),
            retired: BTreeMap::new(),
            format: 2,
            heads: BTreeMap::new(),
            commits: BTreeMap::new(),
            staged: BTreeMap::new(),
            operations: BTreeMap::new(),
        }
    }
}
impl CommitState {
    /// Reads the exact canonical head; absence does not initialize a domain.
    #[must_use]
    pub fn head_revision(&self, domain: &str) -> RevisionRef {
        self.heads.get(domain).cloned().unwrap_or_default()
    }
    /// Verifies digests, references, DAG ordering, receipt index and capacity.
    /// # Errors
    /// Corrupt or excessive images must never be repaired implicitly.
    pub fn validate(&self) -> Result<(), Failure> {
        self.validate_external()?;
        if self.retired.len() > MAX_COMMITS
            || self.retired.iter().any(|(key, value)| {
                key.len() > 260
                    || value.len() != 64
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                    || self.operations.contains_key(key)
            })
        {
            return Err(Failure::Corrupt);
        }
        if self.format != 2 || self.commits.len() != self.operations.len() {
            return Err(Failure::Corrupt);
        }
        self.capacity()?;
        for (reference, value) in &self.commits {
            let p = &value.prepared;
            p.validate().map_err(|_| Failure::Corrupt)?;
            if p.receipt().map_err(|_| Failure::Corrupt)? != value.receipt
                || &value.receipt.commit_ref != reference
                || self
                    .operations
                    .get(&operation_key(&p.domain, &p.operation_id))
                    != Some(reference)
            {
                return Err(Failure::Corrupt);
            }
            self.validate_parents(p).map_err(|_| Failure::Corrupt)?;
        }
        for (domain, revision) in &self.heads {
            let value = revision
                .commit
                .as_ref()
                .and_then(|r| self.commits.get(r))
                .ok_or(Failure::Corrupt)?;
            if value.prepared.domain != *domain || value.receipt.committed_revision != *revision {
                return Err(Failure::Corrupt);
            }
        }
        for (r, staged) in &self.staged {
            staged.prepared.validate().map_err(|_| Failure::Corrupt)?;
            if r != staged.prepared.reference()
                || self
                    .receipt(&staged.prepared.domain, &staged.prepared.operation_id)
                    .is_some()
            {
                return Err(Failure::Corrupt);
            }
        }
        Ok(())
    }
    pub(super) fn capacity(&self) -> Result<(), Failure> {
        let bytes = self.stats().content_bytes;
        if self.staged.len() > MAX_PREPARED
            || self.commits.len() > MAX_COMMITS
            || bytes > MAX_STORE_BYTES
        {
            return Err(Failure::Capacity);
        }
        Ok(())
    }
    pub(super) fn stats(&self) -> StoreStats {
        StoreStats {
            prepared: self
                .staged
                .values()
                .filter(|v| v.reclaim_after.is_none())
                .count(),
            abandoned: self
                .staged
                .values()
                .filter(|v| v.reclaim_after.is_some())
                .count(),
            committed: self.commits.len(),
            content_bytes: self
                .staged
                .values()
                .map(|v| v.prepared.payload.len())
                .sum::<usize>()
                + self
                    .commits
                    .values()
                    .map(|v| v.prepared.payload.len())
                    .sum::<usize>(),
        }
    }
    pub(super) fn receipt(&self, domain: &str, operation: &str) -> Option<&CommitReceipt> {
        self.operations
            .get(&operation_key(domain, operation))
            .and_then(|r| self.commits.get(r))
            .map(|v| &v.receipt)
    }
    pub(super) fn validate_parents(&self, p: &PreparedCommit) -> Result<(), Rejection> {
        if p.base_revision.commit.is_none() && !p.parents.is_empty() {
            return Err(Rejection::InvalidParent);
        }
        if let Some(base) = &p.base_revision.commit {
            if !p.parents.contains(base) {
                return Err(Rejection::InvalidParent);
            }
            let base = self.commits.get(base).ok_or(Rejection::InvalidParent)?;
            if base.receipt.committed_revision != p.base_revision {
                return Err(Rejection::InvalidParent);
            }
        }
        for parent in &p.parents {
            let prior = self.commits.get(parent).ok_or(Rejection::InvalidParent)?;
            if prior.prepared.domain != p.domain
                || prior.receipt.committed_revision.sequence > p.base_revision.sequence
            {
                return Err(Rejection::InvalidParent);
            }
        }
        Ok(())
    }
    pub(super) fn roots(&self) -> Vec<CommitRef> {
        // Every valid receipt is a root too, so replay results survive head advances.
        self.operations
            .values()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    pub(super) fn reachable(&self, roots: &[CommitRef]) -> Result<BTreeSet<CommitRef>, Failure> {
        let mut seen = BTreeSet::new();
        let mut pending = roots.to_vec();
        if pending.len() > MAX_COMMITS {
            return Err(Failure::Capacity);
        }
        while let Some(r) = pending.pop() {
            if !seen.insert(r.clone()) {
                continue;
            }
            let entry = self.commits.get(&r).ok_or(Failure::Corrupt)?;
            pending.extend(entry.receipt.parent_refs.iter().cloned());
        }
        Ok(seen)
    }
    pub(super) fn reclaim_candidates(&self, epoch: u64) -> Vec<String> {
        self.staged
            .iter()
            .filter(|(_, s)| s.reclaim_after.is_some_and(|after| after <= epoch))
            .filter(|(r, _)| !self.external.values().any(|e| e.prepared.contains_key(*r)))
            .map(|(r, _)| r.clone())
            .collect()
    }
}
pub(super) fn operation_key(domain: &str, operation: &str) -> String {
    format!("{}:{domain}{operation}", domain.len())
}
