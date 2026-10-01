// Revision foundation 0.10.0: extracted unchanged mechanics; no domain policy.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
pub const MAX_PREPARED: usize = 256;
pub const MAX_COMMITS: usize = 4096;
pub const MAX_STORE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PARENTS: usize = 8;

pub type OperationId = String;
pub type CommitRef = String;
pub type ParentRef = CommitRef;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionRef {
    pub sequence: u64,
    pub commit: Option<CommitRef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lifecycle {
    Draft,
    Prepared,
    Committed,
    Abandoned,
}

/// Mutable, noncanonical proposal. `prepare_with` consumes it after owner checks.
#[derive(Clone, Debug)]
pub struct CommitIntent<T = Vec<u8>> {
    pub domain: String,
    pub operation_id: OperationId,
    pub expected_revision: RevisionRef,
    pub parents: Vec<ParentRef>,
    pub payload: T,
}

impl<T> CommitIntent<T> {
    /// Encodes an owner-validated proposal into immutable canonical bytes.
    ///
    /// # Errors
    /// Rejects invalid identities, duplicate parents, oversized payloads or encoding.
    pub fn prepare_with(
        self,
        encode: impl FnOnce(T) -> Result<Vec<u8>, Rejection>,
    ) -> Result<PreparedCommit, Rejection> {
        if !token(&self.domain)
            || !token(&self.operation_id)
            || self.parents.len() > MAX_PARENTS
            || self.parents.iter().any(|p| !digest_ref(p))
            || self
                .expected_revision
                .commit
                .as_ref()
                .is_some_and(|p| !digest_ref(p))
            || (self.expected_revision.sequence == 0) != self.expected_revision.commit.is_none()
        {
            return Err(Rejection::InvalidIntent);
        }
        let mut parents = self.parents;
        parents.sort();
        if parents.windows(2).any(|p| p[0] == p[1]) {
            return Err(Rejection::InvalidIntent);
        }
        let payload = encode(self.payload)?;
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(Rejection::Capacity);
        }
        let payload_digest = hash(&[b"content/1", &payload]);
        let mut components: Vec<&[u8]> = vec![
            b"prepared/1",
            self.domain.as_bytes(),
            self.operation_id.as_bytes(),
        ];
        let sequence = self.expected_revision.sequence.to_be_bytes();
        components.push(&sequence);
        components.push(
            self.expected_revision
                .commit
                .as_deref()
                .unwrap_or("")
                .as_bytes(),
        );
        components.extend(parents.iter().map(String::as_bytes));
        components.push(payload_digest.as_bytes());
        let prepared_ref = hash(&components);
        Ok(PreparedCommit {
            domain: self.domain,
            operation_id: self.operation_id,
            base_revision: self.expected_revision,
            parents,
            payload,
            payload_digest,
            prepared_ref,
        })
    }
}
impl CommitIntent<Vec<u8>> {
    /// Seals bytes whose deterministic encoding and domain validity the owner checked.
    /// # Errors
    /// Returns structural or capacity rejection without any I/O.
    pub fn prepare(self) -> Result<PreparedCommit, Rejection> {
        self.prepare_with(Ok)
    }
}

/// Fields are private; decoding a stored image is followed by identity validation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedCommit {
    pub(super) domain: String,
    pub(super) operation_id: OperationId,
    pub(super) base_revision: RevisionRef,
    pub(super) parents: Vec<ParentRef>,
    pub(super) payload: Vec<u8>,
    pub(super) payload_digest: String,
    pub(super) prepared_ref: String,
}
impl PreparedCommit {
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.prepared_ref
    }
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
    #[must_use]
    pub fn domain(&self) -> &str {
        &self.domain
    }
    #[must_use]
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
    #[must_use]
    pub fn base_revision(&self) -> &RevisionRef {
        &self.base_revision
    }
    pub(super) fn validate(&self) -> Result<(), Rejection> {
        let rebuilt = CommitIntent {
            domain: self.domain.clone(),
            operation_id: self.operation_id.clone(),
            expected_revision: self.base_revision.clone(),
            parents: self.parents.clone(),
            payload: self.payload.clone(),
        }
        .prepare()?;
        if &rebuilt != self {
            return Err(Rejection::InvalidIntent);
        }
        Ok(())
    }
    pub(super) fn receipt(&self) -> Result<CommitReceipt, Rejection> {
        let next = self
            .base_revision
            .sequence
            .checked_add(1)
            .ok_or(Rejection::Capacity)?;
        let commit_ref = hash(&[
            b"commit/1",
            self.prepared_ref.as_bytes(),
            &next.to_be_bytes(),
        ]);
        Ok(CommitReceipt {
            domain: self.domain.clone(),
            operation_id: self.operation_id.clone(),
            prepared_ref: self.prepared_ref.clone(),
            parent_refs: self.parents.clone(),
            previous_revision: self.base_revision.clone(),
            committed_revision: RevisionRef {
                sequence: next,
                commit: Some(commit_ref.clone()),
            },
            commit_ref,
            payload_digest: self.payload_digest.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitReceipt {
    pub domain: String,
    pub operation_id: OperationId,
    pub prepared_ref: String,
    pub commit_ref: CommitRef,
    pub parent_refs: Vec<ParentRef>,
    pub previous_revision: RevisionRef,
    pub committed_revision: RevisionRef,
    pub payload_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conflict {
    Stale {
        expected: RevisionRef,
        actual: RevisionRef,
    },
    OperationReuse {
        original_prepared_ref: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Rejection {
    #[error("prepared content is not canonical")]
    PreparedNotCanonical,
    #[error("external object reclamation is fenced")]
    Reclaiming,
    #[error("invalid commit intent")]
    InvalidIntent,
    #[error("commit capacity exceeded")]
    Capacity,
    #[error("invalid commit parent")]
    InvalidParent,
    #[error("prepared commit is abandoned")]
    Abandoned,
    #[error("commit is canonical or missing")]
    NotPrepared,
}
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Failure {
    #[error("storage operation failed")]
    Storage,
    #[error("stored commit image is invalid")]
    Corrupt,
    /// Backend opening requires explicit recovery. This does not assert the
    /// cause of interruption or that the logical commit image is valid.
    #[error("explicit storage recovery is required")]
    RecoveryRequired,
    #[error("commit capacity exceeded")]
    Capacity,
    #[error("commit rejected: {0}")]
    Rejected(Rejection),
    #[error("outcome unavailable; query the same operation receipt")]
    DeliveryUnknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryClass {
    None,
    Revalidate,
    ReplayOnly,
}
impl Failure {
    #[must_use]
    pub fn retry_class(&self) -> RetryClass {
        match self {
            Self::Storage | Self::DeliveryUnknown => RetryClass::ReplayOnly,
            Self::Corrupt | Self::RecoveryRequired | Self::Capacity | Self::Rejected(_) => {
                RetryClass::None
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitOutcome {
    Committed(CommitReceipt),
    NoChange(CommitReceipt),
    Conflict(Conflict),
    Rejected(Rejection),
    Failure(Failure),
}
impl CommitOutcome {
    #[must_use]
    pub fn retry_class(&self) -> RetryClass {
        match self {
            Self::Conflict(Conflict::Stale { .. }) => RetryClass::Revalidate,
            Self::Failure(f) => f.retry_class(),
            _ => RetryClass::None,
        }
    }
}

/// Owners alone determine whether independent payload changes may merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeRequest {
    pub base: CommitRef,
    pub left: CommitRef,
    pub right: CommitRef,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeOutcome<T, C> {
    Merged(T),
    Conflict(C),
    NeedsOwnerResolution(C),
    NoChange(CommitRef),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoreStats {
    pub prepared: usize,
    pub abandoned: usize,
    pub committed: usize,
    pub content_bytes: usize,
}

fn token(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_graphic())
}
fn digest_ref(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn hash(parts: &[&[u8]]) -> String {
    use std::fmt::Write;
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    let mut text = String::with_capacity(64);
    for b in hash.finalize() {
        let _ = write!(text, "{b:02x}");
    }
    text
}
