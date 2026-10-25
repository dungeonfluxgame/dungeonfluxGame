//! SHA-256 and the domain-separated business keys of planning/finance-ledger-research.md 20.
//!
//! The hash is the `sha2` crate's (a test checks the published vectors and that PostgreSQL's
//! `sha256` agrees on a posted entry); `ledger::canonical_v1` is the entry hash input.

use sha2::{Digest, Sha256};

/// The SHA-256 digest of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// The hash of one posted entry in a stream chain: version 1 is
/// `SHA-256("df-ledger/1" || prev_hash || canonical_v1)`.
pub fn entry_hash_v1(prev_hash: &[u8; 32], canonical: &[u8]) -> [u8; 32] {
    let mut input = b"df-ledger/1".to_vec();
    input.extend_from_slice(prev_hash);
    input.extend_from_slice(canonical);
    sha256(&input)
}

/// A business key (20): SHA-256 over a domain tag and the fields, each length-prefixed, so no two
/// different field lists encode the same bytes.
pub fn business_key(domain: &str, fields: &[&[u8]]) -> [u8; 32] {
    let mut input = Vec::new();
    for field in std::iter::once(domain.as_bytes()).chain(fields.iter().copied()) {
        input.extend_from_slice(&u32::try_from(field.len()).unwrap_or(u32::MAX).to_be_bytes());
        input.extend_from_slice(field);
    }
    sha256(&input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finance_ids::hex;

    #[test]
    fn sha256_matches_the_published_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let million_a = vec![b'a'; 1_000_000];
        assert_eq!(
            hex(&sha256(&million_a)),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn two_field_lists_that_concatenate_alike_hash_differently() {
        assert_ne!(
            business_key("d", &[b"ab", b"c"]),
            business_key("d", &[b"a", b"bc"])
        );
    }
}
