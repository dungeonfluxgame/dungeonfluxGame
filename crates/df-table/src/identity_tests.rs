use std::num::NonZeroU128;

use crate::identity::{
    EvidenceRef, FactId, IdentityError, MachineId, OperationId, RecoveryEpoch, RevisionError,
    SceneId, SessionRevision, TableId, TableScope, TextIdentityError, TurnSeq,
};

const MIXED_DIGITS: &str = "0123456789abcdef0011223344556677";

#[test]
fn an_operation_id_is_thirty_two_lowercase_hex_digits() {
    let id = OperationId::from_nonzero_u128(NonZeroU128::new(0xabc).expect("nonzero"));
    assert_eq!(id.to_string(), "00000000000000000000000000000abc");
    let widest = OperationId::from_bytes(&[0xff; 16]).expect("all ones is valid");
    assert_eq!(widest.to_string(), "f".repeat(32));
}

#[test]
fn an_operation_id_serializes_as_its_hex_text_and_parses_back() {
    let id = OperationId::from_hex(MIXED_DIGITS).expect("valid id");
    let json = serde_json::to_string(&id).expect("an id serializes");
    assert_eq!(json, format!("\"{MIXED_DIGITS}\""));
    let parsed: OperationId = serde_json::from_str(&json).expect("an id parses");
    assert_eq!(parsed, id);
    assert_eq!(id.as_bytes()[0], 0x01);
    assert_eq!(id.as_bytes()[15], 0x77);
}

#[test]
fn an_operation_id_of_zero_is_refused_in_both_forms() {
    assert_eq!(
        OperationId::from_hex(&"0".repeat(32)),
        Err(TextIdentityError::Zero)
    );
    assert_eq!(OperationId::from_bytes(&[0; 16]), Err(IdentityError::Zero));
    assert!(serde_json::from_str::<OperationId>(&format!("\"{}\"", "0".repeat(32))).is_err());
}

#[test]
fn an_uppercase_operation_id_is_refused() {
    assert_eq!(
        OperationId::from_hex("0123456789ABCDEF0011223344556677"),
        Err(TextIdentityError::Malformed)
    );
    assert_eq!(
        OperationId::from_hex(&"F".repeat(32)),
        Err(TextIdentityError::Malformed)
    );
}

#[test]
fn text_of_the_wrong_length_or_with_other_characters_is_not_an_operation_id() {
    assert_eq!(
        OperationId::from_hex(""),
        Err(TextIdentityError::InvalidLength { actual: 0 })
    );
    assert_eq!(
        OperationId::from_hex(&"a".repeat(31)),
        Err(TextIdentityError::InvalidLength { actual: 31 })
    );
    assert_eq!(
        OperationId::from_hex(&"a".repeat(33)),
        Err(TextIdentityError::InvalidLength { actual: 33 })
    );
    assert_eq!(
        OperationId::from_hex(&"g".repeat(32)),
        Err(TextIdentityError::Malformed)
    );
    assert_eq!(
        OperationId::from_hex(&format!("+{}", "a".repeat(31))),
        Err(TextIdentityError::Malformed)
    );
    assert_eq!(
        OperationId::from_bytes(&[1; 15]),
        Err(IdentityError::InvalidLength { actual: 15 })
    );
}

#[test]
fn a_fact_id_can_start_with_a_digit_like_the_real_fact_id() {
    let id = FactId::from_hex("9f000000000000000000000000000001").expect("a digit may lead");
    assert_eq!(id.to_string(), "9f000000000000000000000000000001");
    assert_eq!(
        FactId::from_bytes(id.as_bytes()),
        Ok(id),
        "bytes and text agree"
    );
    assert_eq!(FactId::from_bytes(&[0; 16]), Err(IdentityError::Zero));
}

#[test]
fn an_evidence_reference_is_an_opaque_identity_of_the_same_shape() {
    let reference = EvidenceRef::from_hex(MIXED_DIGITS).expect("valid reference");
    assert_eq!(reference.to_string(), MIXED_DIGITS);
    assert!(EvidenceRef::from_hex("not a reference").is_err());
}

#[test]
fn a_zero_recovery_epoch_is_refused_and_the_first_epoch_is_one() {
    assert_eq!(RecoveryEpoch::new(0), Err(RevisionError::ZeroEpoch));
    assert_eq!(RecoveryEpoch::new(1), Ok(RecoveryEpoch::FIRST));
    assert_eq!(RecoveryEpoch::FIRST.get(), 1);
}

#[test]
fn revisions_order_by_epoch_then_by_sequence() {
    let first = RecoveryEpoch::FIRST;
    let second = RecoveryEpoch::new(2).expect("two is a valid epoch");
    assert!(SessionRevision::new(first, 9) < SessionRevision::new(second, 0));
    assert!(SessionRevision::new(first, 3) < SessionRevision::new(first, 4));
    assert_eq!(SessionRevision::new(second, 5).sequence(), 5);
    assert_eq!(SessionRevision::new(second, 5).epoch(), second);
}

#[test]
fn the_next_sequence_stays_in_its_epoch_and_refuses_to_overflow() {
    let revision = SessionRevision::new(RecoveryEpoch::FIRST, 41);
    assert_eq!(
        revision.next_sequence(),
        Ok(SessionRevision::new(RecoveryEpoch::FIRST, 42))
    );
    assert_eq!(
        SessionRevision::new(RecoveryEpoch::FIRST, u64::MAX).next_sequence(),
        Err(RevisionError::SequenceOverflow)
    );
}

#[test]
fn a_machine_id_follows_the_name_rules_and_refuses_them_when_parsed_too() {
    assert!(MachineId::new("lobby").is_ok());
    assert!(MachineId::new("Lobby").is_err());
    assert!(serde_json::from_str::<MachineId>("\"lobby\"").is_ok());
    assert!(serde_json::from_str::<MachineId>("\"Lobby\"").is_err());
    assert!(serde_json::from_str::<MachineId>("\"\"").is_err());
}

#[test]
fn a_scope_names_the_table_and_adds_the_scene_and_machine_when_known() {
    let scope = TableScope::of_table(TableId::new(1));
    assert_eq!(scope.scene, None);
    assert_eq!(scope.machine, None);
    let routed = scope
        .in_scene(SceneId::new(2))
        .in_machine(MachineId::new("explore").expect("valid machine"));
    assert_eq!(routed.scene, Some(SceneId::new(2)));
    assert_eq!(routed.machine.expect("a machine").as_str(), "explore");
}

#[test]
fn number_ids_display_and_serialize_as_plain_numbers() {
    assert_eq!(TurnSeq::new(41).to_string(), "41");
    assert_eq!(
        serde_json::to_string(&TableId::new(7)).expect("serializes"),
        "7"
    );
    assert_eq!(
        serde_json::from_str::<SceneId>("12").expect("parses"),
        SceneId::new(12)
    );
}
