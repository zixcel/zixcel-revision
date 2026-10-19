#![cfg(feature = "attestation")]
use zixcel_revision::attestation::content_digest;

#[test]
fn digest_text_preserves_the_standard_sha256_bytes() {
    assert_eq!(
        content_digest(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        content_digest(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
