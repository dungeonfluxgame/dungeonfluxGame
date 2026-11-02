//! The machine surface of `gameplay.proto` (plan O.16 W0, RES33, RES40): its presence and oneof
//! behaviour, its additive compatibility and where its methods and offers sit, read from the
//! generated code and the real protoc descriptor. Its ledger lines are checked, in both
//! directions, by `df-tools` `schema_ledger.rs`.

use df_protocol::common as wire;
use df_protocol::common::machine_intent::Payload;
use prost::Message;
use prost_types::FileDescriptorSet;

const PACKAGE: &str = "dungeonflux.public.v1";

/// Every method of every service in the real descriptor, as `Service.Method request response
/// client-stream server-stream`.
fn methods() -> Vec<String> {
    let set = FileDescriptorSet::decode(df_protocol::FILE_DESCRIPTOR_SET).expect("descriptor");
    let mut found = Vec::new();
    for file in set.file.iter().filter(|file| file.package() == PACKAGE) {
        for service in &file.service {
            for method in &service.method {
                found.push(format!(
                    "{}.{} {} {} {} {}",
                    service.name(),
                    method.name(),
                    method.input_type(),
                    method.output_type(),
                    method.client_streaming(),
                    method.server_streaming(),
                ));
            }
        }
    }
    found
}

// The field ledger itself (every name, number, presence and reserved entry of these messages, in
// both directions) is checked against the real descriptor by `schema_ledger.rs` in `df-tools`, so
// removing or renumbering a machine field fails there until a `reserved` entry covers it.

#[test]
fn the_two_methods_are_on_existing_services_in_the_modes_the_plan_names() {
    let found = methods();
    let method = |name: &str, request: &str, response: &str, server: bool| {
        format!("{name} .{PACKAGE}.{request} .{PACKAGE}.{response} false {server}")
    };
    assert!(found.contains(&method(
        "ActionService.SubmitMachineIntent",
        "SubmitMachineIntentRequest",
        "SubmitMachineIntentResponse",
        false
    )));
    assert!(found.contains(&method(
        "SessionService.WatchMachine",
        "WatchViewRequest",
        "MachineView",
        true
    )));
}

#[test]
fn offers_live_in_the_seat_view_and_field_seven_of_the_view_stays_reserved() {
    let set = FileDescriptorSet::decode(df_protocol::FILE_DESCRIPTOR_SET).expect("descriptor");
    let file = set
        .file
        .iter()
        .find(|file| file.package() == PACKAGE && file.name() == "gameplay.proto")
        .expect("gameplay.proto");
    let message = |name: &str| {
        file.message_type
            .iter()
            .find(|message| message.name() == name)
            .unwrap_or_else(|| panic!("{name} is declared"))
    };
    let view = message("MachineView");
    assert!(view.field.iter().all(|field| field.name() != "offers"));
    assert!(
        view.reserved_range
            .iter()
            .any(|range| range.start() <= 7 && 7 < range.end())
    );
    assert!(view.reserved_name.iter().any(|name| name == "offers"));
    assert!(
        message("MachineSeatView")
            .field
            .iter()
            .any(|field| field.name() == "offers" && field.number() == 6)
    );
}

fn sample_intent(payload: Payload) -> wire::MachineIntent {
    wire::MachineIntent {
        machine_id: "explore".to_owned(),
        offer_id: Some("look".to_owned()),
        payload: Some(payload),
    }
}

#[test]
fn each_intent_kind_round_trips_with_its_payload_and_no_other() {
    let payloads = [
        Payload::Tap(wire::MachineTap {}),
        Payload::SubmitText(wire::MachineText {
            utterance_id: Some(1),
            text: "open the door".to_owned(),
        }),
        Payload::Say(wire::MachineText {
            utterance_id: Some(2),
            text: "hello".to_owned(),
        }),
        Payload::PendingAnswer(wire::MachinePendingAnswer {
            pending_id: Some(3),
        }),
        Payload::SafetyCard(wire::MachineSafetyCard {}),
    ];
    for payload in payloads {
        let intent = sample_intent(payload);
        let decoded = wire::MachineIntent::decode(intent.encode_to_vec().as_slice()).unwrap();
        assert_eq!(decoded, intent);
    }
    // The last payload on the wire wins and only one is ever set.
    let mut bytes = sample_intent(Payload::Tap(wire::MachineTap {})).encode_to_vec();
    bytes.extend(
        sample_intent(Payload::PendingAnswer(wire::MachinePendingAnswer {
            pending_id: Some(9),
        }))
        .encode_to_vec(),
    );
    let merged = wire::MachineIntent::decode(bytes.as_slice()).unwrap();
    assert_eq!(
        merged.payload,
        Some(Payload::PendingAnswer(wire::MachinePendingAnswer {
            pending_id: Some(9)
        }))
    );
}

#[test]
fn an_empty_tap_is_present_and_an_absent_payload_is_absent() {
    let tap = sample_intent(Payload::Tap(wire::MachineTap {}));
    let encoded = tap.encode_to_vec();
    assert_eq!(
        wire::MachineIntent::decode(encoded.as_slice())
            .unwrap()
            .payload,
        Some(Payload::Tap(wire::MachineTap {}))
    );
    let none = wire::MachineIntent {
        machine_id: "explore".to_owned(),
        offer_id: None,
        payload: None,
    };
    assert_eq!(
        wire::MachineIntent::decode(none.encode_to_vec().as_slice())
            .unwrap()
            .payload,
        None
    );
}

#[test]
fn utterance_and_pending_ids_distinguish_absent_from_explicit_zero() {
    let absent = wire::MachineText::decode(&[][..]).unwrap();
    let zero = wire::MachineText::decode(&[8, 0][..]).unwrap();
    assert_eq!(absent.utterance_id, None);
    assert_eq!(zero.utterance_id, Some(0));
    assert_eq!(zero.encode_to_vec(), [8, 0]);
    assert_eq!(
        wire::MachinePendingAnswer::decode(&[][..])
            .unwrap()
            .pending_id,
        None
    );
    assert_eq!(
        wire::MachinePendingAnswer::decode(&[8, 0][..])
            .unwrap()
            .pending_id,
        Some(0)
    );
    let absent = wire::MachineOffer::decode(&[][..]).unwrap();
    assert_eq!(absent.explain, None);
    let empty = wire::MachineOffer::decode(&[26, 0][..]).unwrap();
    assert_eq!(empty.explain, Some(String::new()));
}

#[test]
fn a_decline_carries_only_the_closed_rejection_codes_and_keeps_unknown_numbers() {
    for code in [
        wire::RejectionCode::Unspecified,
        wire::RejectionCode::StaleOffer,
        wire::RejectionCode::WrongTurn,
        wire::RejectionCode::InvalidSelection,
        wire::RejectionCode::Paused,
        wire::RejectionCode::ResourceMissing,
        wire::RejectionCode::CapabilityUnavailable,
        wire::RejectionCode::RecoveryRequired,
        wire::RejectionCode::RevisionConflict,
        wire::RejectionCode::OperationConflict,
        wire::RejectionCode::OperationExpired,
    ] {
        let decline = wire::MachineDecline {
            code: code as i32,
            reason: "It isn't your turn.".to_owned(),
            missing: None,
            alternatives: Vec::new(),
        };
        let decoded = wire::MachineDecline::decode(decline.encode_to_vec().as_slice()).unwrap();
        assert_eq!(decoded, decline);
        assert_eq!(wire::RejectionCode::try_from(decoded.code), Ok(code));
    }
    // A code from a newer server survives the round trip and is not mistaken for a known one.
    let future = wire::MachineDecline {
        code: 99,
        reason: "Later.".to_owned(),
        missing: None,
        alternatives: Vec::new(),
    };
    let decoded = wire::MachineDecline::decode(future.encode_to_vec().as_slice()).unwrap();
    assert_eq!(decoded.code, 99);
    assert!(wire::RejectionCode::try_from(decoded.code).is_err());
}

#[test]
fn the_four_receipt_outcomes_round_trip() {
    let results = [
        wire::machine_receipt::Result::Applied(wire::MachineApplied {}),
        wire::machine_receipt::Result::NeedsClarification(wire::MachineClarification {
            question: "Which door?".to_owned(),
            options: vec![wire::MachineOffer {
                offer_id: "left".to_owned(),
                label: "The left door".to_owned(),
                ..Default::default()
            }],
        }),
        wire::machine_receipt::Result::Declined(wire::MachineDecline {
            code: wire::RejectionCode::WrongTurn as i32,
            reason: "It isn't your turn.".to_owned(),
            missing: Some("your turn".to_owned()),
            alternatives: Vec::new(),
        }),
        wire::machine_receipt::Result::Pending(wire::MachinePending {
            pending_id: Some(4),
        }),
    ];
    for result in results {
        let receipt = wire::MachineReceipt {
            operation_id: Some(wire::OperationId {
                value: Some(vec![3; 16]),
            }),
            revision: Some(wire::SessionRevision {
                epoch: Some(wire::RecoveryEpoch { value: Some(1) }),
                sequence: Some(8),
            }),
            replayed: true,
            utterance_id: Some(5),
            result: Some(result),
        };
        let response = wire::SubmitMachineIntentResponse {
            outcome: Some(wire::submit_machine_intent_response::Outcome::Receipt(
                receipt,
            )),
        };
        assert_eq!(
            wire::SubmitMachineIntentResponse::decode(response.encode_to_vec().as_slice()).unwrap(),
            response
        );
    }
}

#[test]
fn fields_added_later_are_skipped_by_this_reader_wherever_they_appear() {
    // Field 15 (not allocated) with an empty length-delimited value, before and after the
    // known fields of a message and inside a nested one.
    let future_field = [0x7A, 0x00];
    let intent = sample_intent(Payload::Tap(wire::MachineTap {}));
    let known = intent.encode_to_vec();
    for bytes in [
        [known.as_slice(), &future_field].concat(),
        [&future_field, known.as_slice()].concat(),
    ] {
        assert_eq!(
            wire::MachineIntent::decode(bytes.as_slice()).unwrap(),
            intent
        );
    }
    let nested = wire::MachineSeatView {
        offers: vec![wire::MachineOffer {
            offer_id: "a".to_owned(),
            label: "A".to_owned(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut offer = nested.offers[0].encode_to_vec();
    offer.extend_from_slice(&future_field);
    // Field 6 of the seat view is its offers.
    let mut seat = vec![0x32, u8::try_from(offer.len()).unwrap()];
    seat.extend(offer);
    assert_eq!(
        wire::MachineSeatView::decode(seat.as_slice()).unwrap(),
        nested
    );
}

#[test]
fn a_view_audience_is_one_of_the_two_and_the_last_on_the_wire_wins() {
    let screen = wire::MachineView {
        audience: Some(wire::machine_view::Audience::SharedScreen(
            wire::MachineSharedScreen {
                place: "Wharf".to_owned(),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    let seat = wire::MachineView {
        audience: Some(wire::machine_view::Audience::Seat(wire::MachineSeatView {
            hero: "Kira".to_owned(),
            ..Default::default()
        })),
        ..Default::default()
    };
    let bytes = [screen.encode_to_vec(), seat.encode_to_vec()].concat();
    let merged = wire::MachineView::decode(bytes.as_slice()).unwrap();
    assert_eq!(merged.audience, seat.audience);
}
