// Revision foundation 0.10.0: extracted unchanged mechanics; no domain policy.
use super::backend::Backend;
use super::model::{
    CommitOutcome, CommitReceipt, CommitRef, Conflict, Failure, Lifecycle, PreparedCommit,
    Rejection, StoreStats,
};
use super::state::{Committed, Staged, operation_key};

pub struct CommitStore<B> {
    pub(super) backend: B,
}
impl<B: Backend> CommitStore<B> {
    /// Exact identities of noncanonical objects in one stream. This is explicit
    /// proposal inspection, not a current-state read or automatic merge.
    /// At most `MAX_PREPARED` small references are returned, never payload copies.
    /// # Errors
    /// Storage failure remains distinct from an empty stream.
    pub fn prepared_references(&self, domain: &str) -> Result<Vec<String>, Failure> {
        self.backend.read(|s| {
            s.staged
                .values()
                .filter(|v| v.prepared.domain == domain)
                .map(|v| v.prepared.reference().to_owned())
                .collect()
        })
    }
    /// Bounded reflog-like view of existing immutable receipts, not a second log.
    /// Domain is the stream identity; previous/committed revision is its head transition.
    /// # Errors
    /// Invalid bounds and storage failures remain typed. No implicit head movement.
    pub fn transitions(
        &self,
        domain: &str,
        after: u64,
        limit: usize,
    ) -> Result<Vec<CommitReceipt>, Failure> {
        if domain.is_empty()
            || domain.len() > 128
            || !domain.bytes().all(|b| b.is_ascii_graphic())
            || !(1..=256).contains(&limit)
        {
            return Err(Failure::Rejected(Rejection::InvalidIntent));
        }
        self.backend.read(|s| {
            // Bound the selection before cloning receipt contents. Never allocate
            // a second history image just to serve a small inspection page.
            let mut rows = std::collections::BTreeMap::new();
            for receipt in s.commits.values().map(|v| &v.receipt) {
                if receipt.domain == domain && receipt.committed_revision.sequence > after {
                    rows.insert(receipt.committed_revision.sequence, receipt);
                    if rows.len() > limit {
                        rows.pop_last();
                    }
                }
            }
            rows.into_values().cloned().collect()
        })
    }
    /// Pure generic CAS preflight; commit repeats this check at its atomic fence.
    /// # Errors
    /// Backend failures remain typed; this method never creates a proposal or receipt.
    pub fn check_base(
        &self,
        domain: &str,
        expected: &super::RevisionRef,
    ) -> Result<Option<Conflict>, Failure> {
        self.backend.read(|s| {
            let actual = s.head_revision(domain);
            (actual != *expected).then(|| Conflict::Stale {
                expected: expected.clone(),
                actual,
            })
        })
    }
    /// Reads canonical content by its exact preparation identity, not by latest head.
    /// # Errors
    /// Storage failures are not interpreted as absence.
    pub fn committed_by_prepared(
        &self,
        reference: &str,
    ) -> Result<Option<PreparedCommit>, Failure> {
        self.backend.read(|s| {
            s.commits
                .values()
                .find(|c| c.prepared.reference() == reference)
                .map(|c| c.prepared.clone())
        })
    }
    /// Explicit inspection of noncanonical prepared state, never a current/head read.
    /// # Errors
    /// Storage failures remain typed.
    pub fn prepared(&self, reference: &str) -> Result<Option<PreparedCommit>, Failure> {
        self.backend
            .read(|s| s.staged.get(reference).map(|v| v.prepared.clone()))
    }
    #[must_use]
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    /// Persists an invisible, immutable prepared object. Identical retries deduplicate.
    /// No domain action is performed, and no active worker lease is created.
    /// # Errors
    /// Invalid, reused, abandoned or excessive proposals and storage failures reject.
    pub fn stage(&self, p: &PreparedCommit) -> Result<(), Failure> {
        p.validate().map_err(Failure::Rejected)?;
        self.backend.write(|s| {
            if s.retired
                .contains_key(&operation_key(&p.domain, &p.operation_id))
            {
                return Err(Failure::Rejected(Rejection::Abandoned));
            }
            if let Some(receipt) = s.receipt(&p.domain, &p.operation_id) {
                return if receipt.prepared_ref == p.prepared_ref {
                    Ok(())
                } else {
                    Err(Failure::Rejected(Rejection::InvalidIntent))
                };
            }
            if let Some(old) = s.staged.get(p.reference()) {
                return if old.reclaim_after.is_none() {
                    Ok(())
                } else {
                    Err(Failure::Rejected(Rejection::Abandoned))
                };
            }
            if s.staged
                .values()
                .any(|v| v.prepared.domain == p.domain && v.prepared.operation_id == p.operation_id)
            {
                return Err(Failure::Rejected(Rejection::InvalidIntent));
            }
            s.validate_parents(p).map_err(Failure::Rejected)?;
            s.staged.insert(
                p.reference().into(),
                Staged {
                    prepared: p.clone(),
                    reclaim_after: None,
                },
            );
            s.capacity()
        })
    }

    /// Atomically publishes payload, head and receipt, or changes none of them.
    /// Replay is checked before expected revision, and never runs a new operation.
    #[must_use]
    pub fn commit(&self, p: &PreparedCommit) -> CommitOutcome {
        self.commit_inner(p, |_| Ok(()))
    }

    fn commit_inner(
        &self,
        p: &PreparedCommit,
        point: impl Fn(CommitPoint) -> Result<(), Failure>,
    ) -> CommitOutcome {
        if let Err(e) = p.validate() {
            return CommitOutcome::Rejected(e);
        }
        let result = self.backend.write(|s| {
            point(CommitPoint::BeforePrepare)?;
            if s.retired
                .contains_key(&operation_key(&p.domain, &p.operation_id))
            {
                return Ok(CommitOutcome::Rejected(Rejection::Abandoned));
            }
            if let Some(receipt) = s.receipt(&p.domain, &p.operation_id) {
                return Ok(if receipt.prepared_ref == p.prepared_ref {
                    CommitOutcome::NoChange(receipt.clone())
                } else {
                    CommitOutcome::Conflict(Conflict::OperationReuse {
                        original_prepared_ref: receipt.prepared_ref.clone(),
                    })
                });
            }
            if s.staged.values().any(|v| {
                v.prepared.domain == p.domain
                    && v.prepared.operation_id == p.operation_id
                    && v.prepared.prepared_ref != p.prepared_ref
            }) {
                return Ok(CommitOutcome::Rejected(Rejection::InvalidIntent));
            }
            if s.staged
                .get(p.reference())
                .is_some_and(|v| v.reclaim_after.is_some())
            {
                return Ok(CommitOutcome::Rejected(Rejection::Abandoned));
            }
            let actual = s.heads.get(&p.domain).cloned().unwrap_or_default();
            if actual != p.base_revision {
                return Ok(CommitOutcome::Conflict(Conflict::Stale {
                    expected: p.base_revision.clone(),
                    actual,
                }));
            }
            if let Err(e) = s.validate_parents(p) {
                return Ok(CommitOutcome::Rejected(e));
            }
            let receipt = match p.receipt() {
                Ok(v) => v,
                Err(e) => return Ok(CommitOutcome::Rejected(e)),
            };
            point(CommitPoint::DuringPrepare)?;
            s.staged.insert(
                p.prepared_ref.clone(),
                Staged {
                    prepared: p.clone(),
                    reclaim_after: None,
                },
            );
            s.capacity()?;
            point(CommitPoint::AfterPrepare)?;
            point(CommitPoint::BeforeHead)?;
            s.heads
                .insert(p.domain.clone(), receipt.committed_revision.clone());
            point(CommitPoint::DuringHead)?;
            s.operations.insert(
                operation_key(&p.domain, &p.operation_id),
                receipt.commit_ref.clone(),
            );
            s.commits.insert(
                receipt.commit_ref.clone(),
                Committed {
                    prepared: p.clone(),
                    receipt: receipt.clone(),
                },
            );
            s.staged.remove(p.reference());
            s.capacity()?;
            point(CommitPoint::AfterHeadBeforeReceiptDelivery)?;
            Ok(CommitOutcome::Committed(receipt))
        });
        match result {
            Ok(outcome) => outcome,
            Err(e) => CommitOutcome::Failure(e),
        }
    }

    /// Explicit cancellation; reclamation cannot run while a backend write is in flight.
    /// `reclaim_after` is an owner-selected monotonic epoch, not an identity input.
    /// Prepared is otherwise protected indefinitely (within the bounded store).
    /// # Errors
    /// Missing/canonical records cannot be abandoned; repeated cancellation keeps grace.
    pub fn abandon(&self, reference: &str, reclaim_after: u64) -> Result<(), Failure> {
        self.backend.write(|s| {
            let staged = s
                .staged
                .get_mut(reference)
                .ok_or(Failure::Rejected(Rejection::NotPrepared))?;
            staged.reclaim_after.get_or_insert(reclaim_after);
            Ok(())
        })
    }
    /// # Errors
    /// Storage failures never turn into an empty current value.
    pub fn current(&self, domain: &str) -> Result<Option<PreparedCommit>, Failure> {
        self.backend.read(|s| {
            s.heads
                .get(domain)
                .and_then(|r| r.commit.as_ref())
                .and_then(|r| s.commits.get(r))
                .map(|v| v.prepared.clone())
        })
    }
    /// # Errors
    /// Storage failures are returned unchanged.
    pub fn receipt(&self, domain: &str, operation: &str) -> Result<Option<CommitReceipt>, Failure> {
        self.backend.read(|s| s.receipt(domain, operation).cloned())
    }
    /// Reads the original immutable result even after its domain head advances.
    /// # Errors
    /// Storage failures are not substituted by a current or empty result.
    pub fn committed(&self, reference: &str) -> Result<Option<PreparedCommit>, Failure> {
        self.backend.read(|s| {
            if s.staged.contains_key(reference) {
                return Err(Failure::Rejected(Rejection::PreparedNotCanonical));
            }
            Ok(s.commits.get(reference).map(|v| v.prepared.clone()))
        })?
    }
    /// # Errors
    /// Storage failures are returned unchanged.
    pub fn lifecycle(&self, reference: &str) -> Result<Option<Lifecycle>, Failure> {
        self.backend.read(|s| {
            if s.commits
                .values()
                .any(|v| v.prepared.reference() == reference)
            {
                Some(Lifecycle::Committed)
            } else {
                s.staged.get(reference).map(|v| {
                    if v.reclaim_after.is_some() {
                        Lifecycle::Abandoned
                    } else {
                        Lifecycle::Prepared
                    }
                })
            }
        })
    }
    /// # Errors
    /// Storage failures are returned unchanged.
    pub fn roots(&self) -> Result<Vec<CommitRef>, Failure> {
        self.backend.read(super::CommitState::roots)
    }
    /// # Errors
    /// Unknown roots and storage failures reject the traversal.
    pub fn reachable_from(&self, roots: &[CommitRef]) -> Result<Vec<CommitRef>, Failure> {
        self.backend
            .read(|s| s.reachable(roots).map(|r| r.into_iter().collect()))?
    }
    /// # Errors
    /// Storage failures are returned unchanged.
    pub fn reclaim_candidates(&self, epoch: u64) -> Result<Vec<String>, Failure> {
        self.backend.read(|s| s.reclaim_candidates(epoch))
    }
    /// Rechecks candidates within the same write fence; receipts and Prepared are roots.
    /// # Errors
    /// Storage errors abort reclamation without partial mutation.
    pub fn reclaim(&self, epoch: u64) -> Result<usize, Failure> {
        self.backend.write(|s| {
            let refs = s.reclaim_candidates(epoch);
            if refs.len() > super::model::MAX_COMMITS.saturating_sub(s.retired.len()) {
                return Err(Failure::Capacity);
            }
            for r in &refs {
                if let Some(value) = s.staged.remove(r) {
                    s.retired.insert(
                        operation_key(&value.prepared.domain, &value.prepared.operation_id),
                        r.clone(),
                    );
                }
            }
            Ok(refs.len())
        })
    }
    /// # Errors
    /// Storage failures are returned unchanged.
    pub fn stats(&self) -> Result<StoreStats, Failure> {
        self.backend.read(super::CommitState::stats)
    }

    #[cfg(test)]
    pub(super) fn fail_at(&self, p: &PreparedCommit, at: CommitPoint) -> CommitOutcome {
        self.commit_inner(p, |point| {
            if point == at {
                Err(Failure::Storage)
            } else {
                Ok(())
            }
        })
    }

    #[cfg(test)]
    pub(super) fn pause_before_head(
        &self,
        p: &PreparedCommit,
        entered: &std::sync::Barrier,
        release: &std::sync::Barrier,
    ) -> CommitOutcome {
        self.commit_inner(p, |point| {
            if point == CommitPoint::BeforeHead {
                entered.wait();
                release.wait();
            }
            Ok(())
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CommitPoint {
    BeforePrepare,
    DuringPrepare,
    AfterPrepare,
    BeforeHead,
    DuringHead,
    AfterHeadBeforeReceiptDelivery,
}
