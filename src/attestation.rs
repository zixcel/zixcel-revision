//! Immutable, bounded authority statements. Verification proves attribution,
//! not truth, execution permission, payload availability or semantic adoption.
#[cfg(feature = "attestation")]
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationMaterial {
    pub authority: String,
    pub epoch: u64,
    pub public_key: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Statement {
    pub subject_ref: String,
    pub authority: String,
    pub authority_epoch: u64,
    pub claim_digest: String,
    pub provenance_refs: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attestation {
    pub statement: Statement,
    pub signature: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AttestationError {
    #[error("invalid or excessive attestation")]
    Invalid,
    #[error("attestation authority or epoch differs")]
    Authority,
    #[error("attestation proof rejected")]
    Proof,
}
/// Stable SHA256 of exact owner bytes. Encoding canonicalization belongs to the owner.
#[must_use]
pub fn content_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
#[must_use]
pub fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl VerificationMaterial {
    /// Validates a declaration without importing a cryptographic implementation.
    /// Signature acceptance still requires `validate`/`Attestation::verify`.
    /// # Errors
    /// Rejects unbounded identity, zero epoch and an empty key declaration.
    pub fn validate_declaration(&self) -> Result<(), AttestationError> {
        if self.authority.is_empty()
            || self.authority.len() > 256
            || self.authority.chars().any(char::is_control)
            || self.epoch == 0
            || self.public_key == [0; 32]
        {
            return Err(AttestationError::Invalid);
        }
        Ok(())
    }
    /// # Errors
    /// Rejects invalid bounded identity, epoch and weak/malformed verification keys.
    #[cfg(feature = "attestation")]
    pub fn validate(&self) -> Result<(), AttestationError> {
        self.validate_declaration()?;
        let key =
            VerifyingKey::from_bytes(&self.public_key).map_err(|_| AttestationError::Proof)?;
        if key.is_weak() {
            return Err(AttestationError::Proof);
        }
        Ok(())
    }
}
#[cfg(feature = "attestation")]
impl Statement {
    fn bytes(&self) -> Result<Vec<u8>, AttestationError> {
        if !valid_digest(&self.subject_ref)
            || !valid_digest(&self.claim_digest)
            || self.authority.is_empty()
            || self.authority.len() > 256
            || self.authority.chars().any(char::is_control)
            || self.authority_epoch == 0
            || self.provenance_refs.len() > 16
            || self.provenance_refs.iter().any(|r| !valid_digest(r))
        {
            return Err(AttestationError::Invalid);
        }
        // Domain separation prevents cross-protocol signature reuse.
        serde_json::to_vec(&("zixcel/revision/attestation/1", self))
            .map_err(|_| AttestationError::Invalid)
    }
}
#[cfg(feature = "attestation")]
impl Attestation {
    /// Authority custody remains with the caller, never with a worker registry here.
    /// # Errors
    /// Invalid statements are rejected before signing.
    pub fn sign(statement: Statement, secret: &[u8; 32]) -> Result<Self, AttestationError> {
        let bytes = statement.bytes()?;
        let signature = SigningKey::from_bytes(secret)
            .sign(&bytes)
            .to_bytes()
            .to_vec();
        Ok(Self {
            statement,
            signature,
        })
    }
    /// Verify against independently trusted ORIGINAL verification material, never
    /// material supplied by the attestation or current replacement signer.
    /// # Errors
    /// Wrong authority, epoch, content or signature is refused.
    pub fn verify(&self, trusted: &VerificationMaterial) -> Result<(), AttestationError> {
        trusted.validate()?;
        if self.statement.authority != trusted.authority
            || self.statement.authority_epoch != trusted.epoch
        {
            return Err(AttestationError::Authority);
        }
        let bytes = self.statement.bytes()?;
        let signature =
            Signature::from_slice(&self.signature).map_err(|_| AttestationError::Proof)?;
        VerifyingKey::from_bytes(&trusted.public_key)
            .map_err(|_| AttestationError::Proof)?
            .verify_strict(&bytes, &signature)
            .map_err(|_| AttestationError::Proof)
    }
    /// Identity includes the exact proof, not just a claim label.
    /// # Errors
    /// Rejects malformed statement or proof length; this does not verify authority.
    pub fn reference(&self) -> Result<String, AttestationError> {
        self.statement.bytes()?;
        if self.signature.len() != 64 {
            return Err(AttestationError::Proof);
        }
        let bytes = serde_json::to_vec(&("zixcel/revision/attestation/object/1", self))
            .map_err(|_| AttestationError::Invalid)?;
        Ok(content_digest(&bytes))
    }
}
