//! Neutral journal bounds and pure equality: no database, clock or provider here.

use super::*;
use crate::request::{ImageOwner, OperationIdentity};
use df_types::{AccountId, CampaignId, OperationId, RunId};

const MAX_BYTES: usize = 64;

fn account_owner() -> ImageOwner {
    ImageOwner::Draft {
        account: AccountId::from_hex("00000000000000000000000000000011").unwrap(),
    }
}

fn run_owner() -> ImageOwner {
    ImageOwner::Run {
        campaign: CampaignId::from_hex("00000000000000000000000000000033").unwrap(),
        run: RunId::from_hex("00000000000000000000000000000044").unwrap(),
    }
}

fn snapshot(byte: u8) -> VersionedSnapshot {
    VersionedSnapshot::new(3, vec![byte; 8], MAX_BYTES).unwrap()
}

fn identity(owner: ImageOwner, fingerprint: [u8; 32]) -> OperationIdentity {
    OperationIdentity {
        owner,
        operation: OperationId::from_hex("00000000000000000000000000000055").unwrap(),
        fingerprint,
    }
}

#[test]
fn snapshot_rejects_zero_revision_empty_and_overflow() {
    assert_eq!(
        VersionedSnapshot::new(0, vec![7; 4], MAX_BYTES).unwrap_err(),
        SnapshotError::ZeroRevision
    );
    assert_eq!(
        VersionedSnapshot::new(1, Vec::new(), MAX_BYTES).unwrap_err(),
        SnapshotError::Empty
    );
    assert_eq!(
        VersionedSnapshot::new(1, vec![7; MAX_BYTES + 1], MAX_BYTES).unwrap_err(),
        SnapshotError::TooLarge {
            len_bytes: MAX_BYTES + 1,
            max_bytes: MAX_BYTES
        }
    );
    let kept = VersionedSnapshot::new(1, vec![7; MAX_BYTES], MAX_BYTES).unwrap();
    assert_eq!(kept.revision(), 1);
    assert_eq!(kept.bytes(), &[7; MAX_BYTES]);
}

#[test]
fn snapshot_debug_hides_payload_bytes() {
    let kept = snapshot(0xab);
    let rendered = format!("{kept:?}");
    assert!(rendered.contains("revision"));
    assert!(!rendered.contains("abab"));
}

#[test]
fn identity_conflict_only_on_same_scope_and_different_fingerprint() {
    let same = identity(account_owner(), [1; 32]);
    assert!(!same.conflicts_with(&identity(account_owner(), [1; 32])));
    assert!(same.conflicts_with(&identity(account_owner(), [2; 32])));
    assert!(!same.conflicts_with(&identity(run_owner(), [2; 32])));
    let mut other_operation = same.clone();
    other_operation.operation = OperationId::from_hex("00000000000000000000000000000066").unwrap();
    assert!(!same.conflicts_with(&other_operation));
}

#[test]
fn owner_bytes_are_versioned_typed_and_invertible() {
    let run_bytes = owner_scope_bytes(&run_owner());
    let draft_bytes = owner_scope_bytes(&account_owner());
    assert_eq!(run_bytes.len(), 34);
    assert_eq!(draft_bytes.len(), 18);
    assert_eq!(run_bytes[0], OWNER_SCOPE_VERSION);
    assert_eq!(draft_bytes[0], OWNER_SCOPE_VERSION);
    assert_ne!(run_bytes, draft_bytes);
    assert_eq!(owner_of_bytes(&run_bytes), Some(run_owner()));
    assert_eq!(owner_of_bytes(&draft_bytes), Some(account_owner()));
    assert_eq!(owner_of_bytes(&[]), None);
    assert_eq!(owner_of_bytes(&[9; 34]), None);
    assert_eq!(owner_of_bytes(&run_bytes[..33]), None);
}

#[test]
fn job_and_interest_ids_reject_zero_and_bad_lengths() {
    assert_eq!(
        ImageJobId::from_bytes(&[0; 16]).unwrap_err(),
        JournalIdError::Zero
    );
    assert_eq!(
        ImageJobId::from_bytes(&[1; 15]).unwrap_err(),
        JournalIdError::InvalidLength { actual: 15 }
    );
    assert_eq!(
        ImageInterestId::from_hex("ZZ000000000000000000000000000011").unwrap_err(),
        JournalIdError::InvalidHex
    );
    assert_eq!(
        ImageJobId::from_hex("0000000000000000000000000000007g").unwrap_err(),
        JournalIdError::InvalidHex
    );
    assert_eq!(
        ImageJobId::from_hex("000000000000000000000000000077").unwrap_err(),
        JournalIdError::InvalidLength { actual: 30 }
    );
    let parsed = ImageJobId::from_hex("00000000000000000000000000000077").unwrap();
    assert_eq!(
        parsed.as_bytes(),
        &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x77]
    );
}

#[test]
fn expiry_boundary_is_exact_and_states_round_trip() {
    assert!(!is_expired(1_000, 999));
    assert!(is_expired(1_000, 1_000));
    assert!(is_expired(1_000, 1_001));
    assert_eq!(JobState::parse_label("planning"), Some(JobState::Planning));
    assert_eq!(
        JobState::parse_label("recipe_frozen"),
        Some(JobState::RecipeFrozen)
    );
    assert_eq!(JobState::parse_label("retired"), Some(JobState::Retired));
    assert_eq!(JobState::parse_label("ready"), None);
    assert_eq!(
        InterestState::parse_label("attached"),
        Some(InterestState::Attached)
    );
    assert_eq!(
        InterestState::parse_label("withdrawn"),
        Some(InterestState::Withdrawn)
    );
    assert_eq!(InterestState::parse_label("gone"), None);
}
