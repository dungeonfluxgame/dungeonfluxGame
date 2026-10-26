//! Namespaced storage identities; address identity is exact and never proof of account ownership.
use sha2::{Digest, Sha256};

pub(super) fn identifier(kind: &str, namespace: &str, value: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(namespace.len().to_le_bytes());
    digest.update(namespace.as_bytes());
    digest.update(value.as_bytes());
    let encoded: String = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{kind}-{encoded}")
}
