use df_protocol::common as wire;
use df_protocol::common::machine_intent::Payload;
use df_protocol::common::machine_receipt::Result as Outcome;
use prost::Message;

use crate::machine::{
    AdmittedMachineIntent, AdmittedPayload, IntentSlot, MAXIMUM_MESSAGE_BYTES, MachineIntentError,
    MachineReceiptError, MachineViewError, ReceiptSlot, RpcStatusClass, ViewSlot,
    admit_machine_intent, check_machine_receipt, check_machine_response, check_machine_view,
    sample,
};

fn admitted(request: &wire::SubmitMachineIntentRequest) -> AdmittedMachineIntent {
    admit_machine_intent(request).expect("the sample request is admitted")
}

fn receipt_for(intent: &AdmittedMachineIntent, result: Outcome) -> wire::MachineReceipt {
    wire::MachineReceipt {
        operation_id: Some(wire::OperationId {
            value: Some(intent.operation.as_bytes().to_vec()),
        }),
        revision: Some(sample::revision()),
        replayed: false,
        utterance_id: match &intent.payload {
            AdmittedPayload::SubmitText(line) | AdmittedPayload::Say(line) => {
                Some(line.utterance_id)
            }
            _ => None,
        },
        result: Some(result),
    }
}

fn offer(id: &str, label: &str) -> wire::MachineOffer {
    wire::MachineOffer {
        offer_id: id.to_owned(),
        label: label.to_owned(),
        ..Default::default()
    }
}

fn decline(code: wire::RejectionCode, reason: &str) -> wire::MachineDecline {
    wire::MachineDecline {
        code: code as i32,
        reason: reason.to_owned(),
        missing: None,
        alternatives: Vec::new(),
    }
}

#[test]
fn the_sample_answer_passes_and_so_does_each_well_formed_outcome() {
    let tap = admitted(&sample::tap_request());
    assert_eq!(
        check_machine_response(&sample::applied_response(), &tap),
        Ok(())
    );
    let outcomes = [
        Outcome::Applied(wire::MachineApplied {}),
        Outcome::Pending(wire::MachinePending {
            pending_id: Some(0),
        }),
        Outcome::NeedsClarification(wire::MachineClarification {
            question: "Which door?".to_owned(),
            options: vec![offer("left", "Left"), offer("right", "Right")],
        }),
        Outcome::Declined(decline(
            wire::RejectionCode::WrongTurn,
            "It isn't your turn.",
        )),
        // No public class: the reason text carries it.
        Outcome::Declined(decline(
            wire::RejectionCode::Unspecified,
            "I can't run that yet.",
        )),
    ];
    for outcome in outcomes {
        assert_eq!(
            check_machine_receipt(&receipt_for(&tap, outcome), &tap),
            Ok(())
        );
    }
}

#[test]
fn a_decline_needs_a_reason_a_machine_class_and_readable_alternatives() {
    let tap = admitted(&sample::tap_request());
    let check = |decline: wire::MachineDecline| {
        check_machine_receipt(&receipt_for(&tap, Outcome::Declined(decline)), &tap)
    };
    assert_eq!(
        check(decline(wire::RejectionCode::WrongTurn, "  ")),
        Err(MachineReceiptError::Blank(ReceiptSlot::Reason))
    );
    assert_eq!(
        check(decline(wire::RejectionCode::Unspecified, "")),
        Err(MachineReceiptError::Blank(ReceiptSlot::Reason))
    );
    let mut unknown = decline(wire::RejectionCode::WrongTurn, "Later.");
    unknown.code = 99;
    assert_eq!(check(unknown), Err(MachineReceiptError::UnknownCode(99)));
    for code in [
        wire::RejectionCode::RecoveryRequired,
        wire::RejectionCode::RevisionConflict,
        wire::RejectionCode::OperationConflict,
        wire::RejectionCode::OperationExpired,
    ] {
        assert_eq!(
            check(decline(code, "No.")),
            Err(MachineReceiptError::NotAMachineDecline(code))
        );
    }
    let mut blank_missing = decline(wire::RejectionCode::ResourceMissing, "You lack it.");
    blank_missing.missing = Some(String::new());
    assert_eq!(
        check(blank_missing),
        Err(MachineReceiptError::Blank(ReceiptSlot::Missing))
    );
    let mut blank_alternative = decline(wire::RejectionCode::StaleOffer, "Too late.");
    blank_alternative.alternatives = vec![offer("a", " ")];
    assert_eq!(
        check(blank_alternative),
        Err(MachineReceiptError::Offer(MachineViewError::Blank(
            ViewSlot::OfferLabel
        )))
    );
    let mut repeated = decline(wire::RejectionCode::StaleOffer, "Too late.");
    repeated.alternatives = vec![offer("a", "A"), offer("a", "Again")];
    assert_eq!(check(repeated), Err(MachineReceiptError::DuplicateOffer));
}

#[test]
fn a_clarification_and_a_pending_need_their_words_and_their_id() {
    let tap = admitted(&sample::tap_request());
    let check = |outcome: Outcome| check_machine_receipt(&receipt_for(&tap, outcome), &tap);
    assert_eq!(
        check(Outcome::NeedsClarification(wire::MachineClarification {
            question: " ".to_owned(),
            options: Vec::new(),
        })),
        Err(MachineReceiptError::Blank(ReceiptSlot::Question))
    );
    assert_eq!(
        check(Outcome::NeedsClarification(wire::MachineClarification {
            question: "Which?".to_owned(),
            options: vec![offer("a", "A"), offer("a", "B")],
        })),
        Err(MachineReceiptError::DuplicateOffer)
    );
    assert_eq!(
        check(Outcome::Pending(wire::MachinePending { pending_id: None })),
        Err(MachineReceiptError::Missing(ReceiptSlot::PendingId))
    );
}

#[test]
fn a_receipt_names_its_operation_and_a_typed_line_by_its_utterance_id() {
    let tap = admitted(&sample::tap_request());
    let say = admitted(&sample::say_request());
    let applied = || Outcome::Applied(wire::MachineApplied {});

    let mut other_operation = receipt_for(&tap, applied());
    other_operation.operation_id = Some(wire::OperationId {
        value: Some(vec![9; 16]),
    });
    assert_eq!(
        check_machine_receipt(&other_operation, &tap),
        Err(MachineReceiptError::OperationMismatch)
    );
    let mut no_operation = receipt_for(&tap, applied());
    no_operation.operation_id = None;
    assert_eq!(
        check_machine_receipt(&no_operation, &tap),
        Err(MachineReceiptError::Missing(ReceiptSlot::OperationId))
    );

    // A line is named by id; a tap has none.
    let mut unnamed_line = receipt_for(&say, applied());
    unnamed_line.utterance_id = None;
    assert_eq!(
        check_machine_receipt(&unnamed_line, &say),
        Err(MachineReceiptError::UtteranceMismatch)
    );
    let mut wrong_line = receipt_for(&say, applied());
    wrong_line.utterance_id = Some(sample::UTTERANCE_ID + 1);
    assert_eq!(
        check_machine_receipt(&wrong_line, &say),
        Err(MachineReceiptError::UtteranceMismatch)
    );
    let mut named_tap = receipt_for(&tap, applied());
    named_tap.utterance_id = Some(sample::UTTERANCE_ID);
    assert_eq!(
        check_machine_receipt(&named_tap, &tap),
        Err(MachineReceiptError::UtteranceMismatch)
    );

    let mut no_result = receipt_for(&tap, applied());
    no_result.result = None;
    assert_eq!(
        check_machine_receipt(&no_result, &tap),
        Err(MachineReceiptError::Missing(ReceiptSlot::Result))
    );
    let mut no_revision = receipt_for(&tap, applied());
    no_revision.revision = None;
    assert!(matches!(
        check_machine_receipt(&no_revision, &tap),
        Err(MachineReceiptError::Request(_))
    ));
}

#[test]
fn an_answer_too_large_for_the_bridge_is_refused_by_size() {
    let tap = admitted(&sample::tap_request());
    let huge = wire::MachineDecline {
        reason: "x".repeat(MAXIMUM_MESSAGE_BYTES),
        ..decline(wire::RejectionCode::WrongTurn, "")
    };
    assert!(matches!(
        check_machine_receipt(&receipt_for(&tap, Outcome::Declined(huge)), &tap),
        Err(MachineReceiptError::TooLarge { .. })
    ));
}

#[test]
fn a_response_that_did_not_commit_carries_a_known_code() {
    let tap = admitted(&sample::tap_request());
    let observation = |code: i32| wire::SubmitMachineIntentResponse {
        outcome: Some(
            wire::submit_machine_intent_response::Outcome::OperationObservation(
                wire::OperationObservation { code },
            ),
        ),
    };
    assert_eq!(
        check_machine_response(
            &observation(wire::RejectionCode::OperationExpired as i32),
            &tap
        ),
        Ok(())
    );
    assert_eq!(
        check_machine_response(&observation(wire::RejectionCode::Unspecified as i32), &tap),
        Err(MachineReceiptError::Missing(ReceiptSlot::ObservationCode))
    );
    assert_eq!(
        check_machine_response(&observation(99), &tap),
        Err(MachineReceiptError::UnknownCode(99))
    );
    assert_eq!(
        check_machine_response(&wire::SubmitMachineIntentResponse::default(), &tap),
        Err(MachineReceiptError::Missing(ReceiptSlot::Outcome))
    );
}

#[test]
fn a_view_too_large_for_the_bridge_is_refused_by_size() {
    let mut view = sample::seat_view();
    if let Some(wire::machine_view::Audience::Seat(seat)) = view.audience.as_mut() {
        seat.narration = (0..2_000)
            .map(|_| "a long line of narration ".repeat(4))
            .collect();
    }
    assert!(view.encoded_len() > MAXIMUM_MESSAGE_BYTES);
    assert!(matches!(
        check_machine_view(&view),
        Err(MachineViewError::TooLarge { .. })
    ));
}

#[test]
fn a_request_too_large_for_the_bridge_is_refused_by_size() {
    let mut request = sample::tap_request();
    if let Some(intent) = request.intent.as_mut() {
        intent.machine_id = "m".repeat(MAXIMUM_MESSAGE_BYTES);
    }
    assert!(matches!(
        admit_machine_intent(&request),
        Err(MachineIntentError::RequestTooLarge { .. })
    ));
}

#[test]
fn the_shared_screen_cannot_carry_an_offer() {
    // Offers live in the seat view. A server that still wrote them at the old top-level field
    // (7, now reserved) to a shared screen has them skipped as an unknown field, not drawn.
    let shared = sample::shared_screen_view();
    let mut bytes = shared.encode_to_vec();
    let old_offer = offer("hidden-choice", "A choice the table must not see");
    let mut field = vec![
        0x3A,
        u8::try_from(old_offer.encoded_len()).expect("a short offer"),
    ];
    field.extend(old_offer.encode_to_vec());
    bytes.extend(field);
    let decoded = wire::MachineView::decode(bytes.as_slice()).expect("unknown fields are skipped");
    assert_eq!(decoded, shared);
    assert_eq!(check_machine_view(&decoded), Ok(()));
    assert!(
        !decoded
            .encode_to_vec()
            .windows(b"hidden-choice".len())
            .any(|window| window == b"hidden-choice")
    );
}

#[test]
fn the_pause_card_is_a_payload_that_names_no_offer() {
    let card = admitted(&sample::safety_card_request());
    assert_eq!(card.payload, AdmittedPayload::SafetyCard);
    let mut with_offer = sample::safety_card_request();
    if let Some(intent) = with_offer.intent.as_mut() {
        intent.offer_id = Some("look-around".to_owned());
    }
    assert_eq!(
        admit_machine_intent(&with_offer),
        Err(MachineIntentError::Unexpected(IntentSlot::OfferId))
    );
    let mut empty = sample::safety_card_request();
    if let Some(intent) = empty.intent.as_mut() {
        intent.payload = None::<Payload>;
    }
    assert_eq!(
        admit_machine_intent(&empty),
        Err(MachineIntentError::Missing(IntentSlot::Payload))
    );
}

#[test]
fn a_refusal_says_which_rpc_status_it_is() {
    assert_eq!(
        MachineIntentError::Missing(IntentSlot::Payload).rpc_status(),
        RpcStatusClass::InvalidArgument
    );
    assert_eq!(
        MachineIntentError::TextTooLong { bytes: 1 }.rpc_status(),
        RpcStatusClass::InvalidArgument
    );
    assert_eq!(
        MachineViewError::DuplicateOffer.rpc_status(),
        RpcStatusClass::Internal
    );
    assert_eq!(
        MachineReceiptError::OperationMismatch.rpc_status(),
        RpcStatusClass::Internal
    );
}
