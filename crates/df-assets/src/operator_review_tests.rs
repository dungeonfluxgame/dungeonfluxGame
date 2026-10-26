use super::operator_review::*;
use crate::asset_lifecycle::AssetState;
use crate::media_ids::{AssetId, AttemptId};
use df_types::{AccountId, OperationId};
use sha2::{Digest, Sha256};

fn id(byte: u8) -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    bytes
}

fn request(reason: &str) -> ReviewRequest {
    ReviewRequest {
        asset: AssetId::from_bytes(&id(1)).unwrap(),
        attempt: AttemptId::from_bytes(&id(2)).unwrap(),
        expected_revision: 42,
        operation: OperationId::from_bytes(&id(3)).unwrap(),
        decision: ReviewDecision::Approved,
        reason: reason.to_string(),
    }
}

#[test]
fn reason_must_be_one_to_five_hundred_trimmed_characters() {
    assert_eq!(
        request("").validated_reason(),
        Err(ReviewError::InvalidReason)
    );
    assert_eq!(
        request("   ").validated_reason(),
        Err(ReviewError::InvalidReason)
    );
    assert_eq!(request("  ok  ").validated_reason(), Ok("ok"));
    let longest = "x".repeat(MAX_REASON_CHARS);
    assert!(request(&longest).validated_reason().is_ok());
    let too_long = "x".repeat(MAX_REASON_CHARS + 1);
    assert_eq!(
        request(&too_long).validated_reason(),
        Err(ReviewError::InvalidReason)
    );
}

#[test]
fn fingerprint_changes_with_payload_but_not_reason_whitespace() {
    let operator = AccountId::from_bytes(&id(9)).unwrap();
    assert_eq!(
        request(" checked ").fingerprint(operator),
        request("checked").fingerprint(operator)
    );
    assert_ne!(
        request("checked").fingerprint(operator),
        request("other").fingerprint(operator)
    );
    let mut rejected = request("checked");
    rejected.decision = ReviewDecision::Rejected;
    assert_ne!(
        request("checked").fingerprint(operator),
        rejected.fingerprint(operator)
    );
}

#[test]
fn approval_requires_validated_and_decision_requires_a_pending_state() {
    assert!(approval_permitted(AssetState::Validated));
    assert!(!approval_permitted(AssetState::Quarantined));
    assert!(!approval_permitted(AssetState::Ready));
    assert!(decision_permitted(AssetState::Quarantined));
    assert!(decision_permitted(AssetState::Validated));
    for state in [
        AssetState::Uploading,
        AssetState::Ready,
        AssetState::Rejected,
        AssetState::Deleted,
    ] {
        assert!(!decision_permitted(state), "{state:?}");
    }
}

#[test]
fn local_bytes_are_exact_and_hash_checked() {
    let asset = AssetId::from_bytes(&id(1)).unwrap();
    let attempt = AttemptId::from_bytes(&id(2)).unwrap();
    let bytes = b"exact staged bytes".to_vec();
    let handle = ReviewByteHandle {
        asset,
        attempt,
        byte_len: bytes.len() as u64,
        sha256: Sha256::digest(&bytes).into(),
    };
    let store = LocalReviewBytes::new();
    assert_eq!(store.read_exact(handle), Err(ReviewError::BytesDenied));
    store.insert(asset, attempt, bytes.clone());
    assert_eq!(store.read_exact(handle), Ok(bytes));
    let mut wrong = handle;
    wrong.byte_len += 1;
    assert_eq!(store.read_exact(wrong), Err(ReviewError::IntegrityMismatch));
}
