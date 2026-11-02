use df_protocol::common as wire;
use df_protocol::common::machine_intent::Payload;
use df_protocol::common::machine_view::Audience;
use prost::Message;

use crate::machine::{
    AdmittedPayload, IntentSlot, MAXIMUM_INTENT_TEXT_BYTES, MachineIntentError, MachineViewError,
    ViewSlot, admit_machine_intent, check_machine_view, sample,
};
use crate::request::{RequestError, RequestField};

/// One way to break a view, and the refusal that names it.
type ViewCase = (fn(&mut wire::MachineView), MachineViewError);
type ScreenCase = (fn(&mut wire::MachineSharedScreen), MachineViewError);

fn seat() -> wire::MachineView {
    sample::seat_view()
}

fn status_mut(view: &mut wire::MachineView) -> &mut wire::MachineStatus {
    view.status.as_mut().expect("the sample has a status")
}

fn seat_mut(view: &mut wire::MachineView) -> &mut wire::MachineSeatView {
    match view.audience.as_mut() {
        Some(Audience::Seat(seat)) => seat,
        other => panic!("the sample is a seat view, found {other:?}"),
    }
}

fn shared_mut(view: &mut wire::MachineView) -> &mut wire::MachineSharedScreen {
    match view.audience.as_mut() {
        Some(Audience::SharedScreen(screen)) => screen,
        other => panic!("the sample is a shared screen, found {other:?}"),
    }
}

fn intent_mut(request: &mut wire::SubmitMachineIntentRequest) -> &mut wire::MachineIntent {
    request.intent.as_mut().expect("the sample has an intent")
}

#[test]
fn both_sample_views_pass_and_survive_a_protobuf_round_trip() {
    for view in [sample::shared_screen_view(), seat()] {
        assert_eq!(check_machine_view(&view), Ok(()));
        let decoded = wire::MachineView::decode(view.encode_to_vec().as_slice()).expect("decodes");
        assert_eq!(decoded, view);
    }
}

#[test]
fn a_view_without_text_to_read_is_refused_at_each_slot() {
    let cases: [ViewCase; 8] = [
        (
            |view| view.readout = "  \n".to_owned(),
            MachineViewError::Blank(ViewSlot::Readout),
        ),
        (
            |view| view.machine_id.clear(),
            MachineViewError::Blank(ViewSlot::MachineId),
        ),
        (
            |view| view.state_label.clear(),
            MachineViewError::Blank(ViewSlot::StateLabel),
        ),
        (
            |view| seat_mut(view).offers[0].label.clear(),
            MachineViewError::Blank(ViewSlot::OfferLabel),
        ),
        (
            |view| seat_mut(view).offers[0].explain = Some(String::new()),
            MachineViewError::Blank(ViewSlot::OfferExplain),
        ),
        (
            |view| seat_mut(view).narration.push(" ".to_owned()),
            MachineViewError::Blank(ViewSlot::NarrationLine),
        ),
        (
            |view| seat_mut(view).hero.clear(),
            MachineViewError::Blank(ViewSlot::Hero),
        ),
        (
            |view| {
                seat_mut(view).note = Some(wire::MachineNote {
                    text: String::new(),
                });
            },
            MachineViewError::Blank(ViewSlot::NoteText),
        ),
    ];
    for (index, (break_it, expected)) in cases.into_iter().enumerate() {
        let mut view = seat();
        break_it(&mut view);
        assert_eq!(check_machine_view(&view), Err(expected), "case {index}");
    }
}

#[test]
fn the_shared_screen_needs_a_place_and_readable_caption_and_names() {
    let cases: [ScreenCase; 4] = [
        (
            |screen| screen.place.clear(),
            MachineViewError::Blank(ViewSlot::Place),
        ),
        (
            |screen| {
                if let Some(caption) = screen.caption.as_mut() {
                    caption.text = String::new();
                }
            },
            MachineViewError::Blank(ViewSlot::CaptionText),
        ),
        (
            |screen| screen.party[0].name.clear(),
            MachineViewError::Blank(ViewSlot::PartyName),
        ),
        (
            |screen| {
                if let Some(image) = screen.backdrop.as_mut() {
                    image.media.clear();
                }
            },
            MachineViewError::Blank(ViewSlot::MediaName),
        ),
    ];
    for (index, (break_it, expected)) in cases.into_iter().enumerate() {
        let mut view = sample::shared_screen_view();
        break_it(shared_mut(&mut view));
        assert_eq!(check_machine_view(&view), Err(expected), "case {index}");
    }
}

#[test]
fn a_view_must_name_its_revision_status_and_audience() {
    let mut view = seat();
    view.audience = None;
    assert_eq!(
        check_machine_view(&view),
        Err(MachineViewError::Missing(ViewSlot::Audience))
    );

    let mut view = seat();
    view.revision = None;
    assert_eq!(
        check_machine_view(&view),
        Err(MachineViewError::Revision(RequestError::Missing(
            RequestField::Revision
        )))
    );

    let mut view = seat();
    view.status = None;
    assert_eq!(
        check_machine_view(&view),
        Err(MachineViewError::Missing(ViewSlot::Status))
    );

    let mut view = seat();
    status_mut(&mut view).kind = wire::MachineStatusKind::Unspecified as i32;
    assert_eq!(
        check_machine_view(&view),
        Err(MachineViewError::Missing(ViewSlot::Status))
    );

    // A status number from a newer server is named, not treated as ok.
    let mut view = seat();
    status_mut(&mut view).kind = 99;
    assert_eq!(
        check_machine_view(&view),
        Err(MachineViewError::UnknownStatus(99))
    );
}

#[test]
fn a_status_other_than_ok_has_a_reason_and_ok_has_none() {
    let mut view = seat();
    status_mut(&mut view).kind = wire::MachineStatusKind::Reading as i32;
    assert_eq!(
        check_machine_view(&view),
        Err(MachineViewError::Missing(ViewSlot::Reason))
    );
    view.reason = Some("  ".to_owned());
    assert_eq!(
        check_machine_view(&view),
        Err(MachineViewError::Blank(ViewSlot::Reason))
    );
    view.reason = Some("Reading what you wrote.".to_owned());
    assert_eq!(check_machine_view(&view), Ok(()));

    let mut view = seat();
    view.reason = Some("Everything is fine.".to_owned());
    assert_eq!(
        check_machine_view(&view),
        Err(MachineViewError::Unexpected(ViewSlot::Reason))
    );
}

#[test]
fn waiting_for_names_who_and_what_and_nothing_else_does() {
    let mut view = seat();
    view.reason = Some("Waiting for Sam to choose.".to_owned());
    status_mut(&mut view).kind = wire::MachineStatusKind::WaitingFor as i32;
    assert_eq!(
        check_machine_view(&view),
        Err(MachineViewError::Missing(ViewSlot::WaitingWho))
    );
    status_mut(&mut view).waiting_who = Some("Sam".to_owned());
    assert_eq!(
        check_machine_view(&view),
        Err(MachineViewError::Missing(ViewSlot::WaitingWhat))
    );
    status_mut(&mut view).waiting_what = Some(" ".to_owned());
    assert_eq!(
        check_machine_view(&view),
        Err(MachineViewError::Blank(ViewSlot::WaitingWhat))
    );
    status_mut(&mut view).waiting_what = Some("a choice".to_owned());
    assert_eq!(check_machine_view(&view), Ok(()));

    let mut view = seat();
    status_mut(&mut view).waiting_who = Some("Sam".to_owned());
    assert_eq!(
        check_machine_view(&view),
        Err(MachineViewError::Unexpected(ViewSlot::WaitingWho))
    );
}

#[test]
fn two_offers_with_one_id_are_refused() {
    let mut view = seat();
    let duplicate = seat_mut(&mut view).offers[0].clone();
    seat_mut(&mut view).offers.push(duplicate);
    assert_eq!(
        check_machine_view(&view),
        Err(MachineViewError::DuplicateOffer)
    );
}

#[test]
fn the_sample_requests_are_admitted_with_typed_identities() {
    let tap = admit_machine_intent(&sample::tap_request()).expect("the tap sample is admitted");
    assert_eq!(tap.machine_id, sample::MACHINE_ID);
    assert_eq!(tap.session.as_bytes(), &sample::SESSION_BYTES);
    assert_eq!(tap.run.as_bytes(), &sample::RUN_BYTES);
    assert_eq!(tap.operation.as_bytes(), &sample::OPERATION_BYTES);
    assert_eq!(tap.observed_revision.sequence(), sample::SEQUENCE);
    assert_eq!(
        tap.payload,
        AdmittedPayload::Tap {
            offer_id: sample::OFFER_ID.to_owned()
        }
    );

    let say = admit_machine_intent(&sample::say_request()).expect("the say sample is admitted");
    let AdmittedPayload::Say(line) = &say.payload else {
        panic!("the say sample admits as a say, found {:?}", say.payload);
    };
    assert_eq!(line.utterance_id, sample::UTTERANCE_ID);
    assert_eq!(line.text, sample::SAY_TEXT);
}

#[test]
fn an_admitted_line_never_prints_its_words() {
    let say = admit_machine_intent(&sample::say_request()).expect("admitted");
    let shown = format!("{say:?}");
    assert!(!shown.contains(sample::SAY_TEXT), "{shown}");
    assert!(shown.contains("text_bytes"), "{shown}");
}

#[test]
fn a_request_with_a_bad_identity_or_revision_is_named_by_field() {
    let mut request = sample::tap_request();
    request.session_id = None;
    assert_eq!(
        admit_machine_intent(&request),
        Err(MachineIntentError::Request(RequestError::Missing(
            RequestField::Session
        )))
    );

    let mut request = sample::tap_request();
    request.operation_id = Some(wire::OperationId {
        value: Some(vec![0; 16]),
    });
    assert!(matches!(
        admit_machine_intent(&request),
        Err(MachineIntentError::Request(RequestError::Identity {
            field: RequestField::Operation,
            ..
        }))
    ));

    let mut request = sample::tap_request();
    request.observed_revision = None;
    assert_eq!(
        admit_machine_intent(&request),
        Err(MachineIntentError::Request(RequestError::Missing(
            RequestField::Revision
        )))
    );
}

#[test]
fn an_intent_carries_exactly_the_fields_of_its_kind() {
    let mut request = sample::tap_request();
    request.intent = None;
    assert_eq!(
        admit_machine_intent(&request),
        Err(MachineIntentError::Missing(IntentSlot::Intent))
    );

    let mut request = sample::tap_request();
    intent_mut(&mut request).machine_id = " ".to_owned();
    assert_eq!(
        admit_machine_intent(&request),
        Err(MachineIntentError::Blank(IntentSlot::MachineId))
    );

    let mut request = sample::tap_request();
    intent_mut(&mut request).payload = None;
    assert_eq!(
        admit_machine_intent(&request),
        Err(MachineIntentError::Missing(IntentSlot::Payload))
    );

    // A tap names its offer.
    let mut request = sample::tap_request();
    intent_mut(&mut request).offer_id = None;
    assert_eq!(
        admit_machine_intent(&request),
        Err(MachineIntentError::Missing(IntentSlot::OfferId))
    );
    intent_mut(&mut request).offer_id = Some(String::new());
    assert_eq!(
        admit_machine_intent(&request),
        Err(MachineIntentError::Blank(IntentSlot::OfferId))
    );

    // A line names no offer.
    let mut request = sample::say_request();
    intent_mut(&mut request).offer_id = Some("look-around".to_owned());
    assert_eq!(
        admit_machine_intent(&request),
        Err(MachineIntentError::Unexpected(IntentSlot::OfferId))
    );
}

#[test]
fn a_line_needs_an_utterance_id_readable_text_and_a_bounded_length() {
    let mut request = sample::say_request();
    if let Some(Payload::Say(text)) = intent_mut(&mut request).payload.as_mut() {
        text.utterance_id = None;
    }
    assert_eq!(
        admit_machine_intent(&request),
        Err(MachineIntentError::Missing(IntentSlot::UtteranceId))
    );

    // Utterance id zero is a value, not an absence.
    let mut request = sample::say_request();
    if let Some(Payload::Say(text)) = intent_mut(&mut request).payload.as_mut() {
        text.utterance_id = Some(0);
    }
    assert!(admit_machine_intent(&request).is_ok());

    let mut request = sample::say_request();
    if let Some(Payload::Say(text)) = intent_mut(&mut request).payload.as_mut() {
        text.text = "  ".to_owned();
    }
    assert_eq!(
        admit_machine_intent(&request),
        Err(MachineIntentError::Blank(IntentSlot::Text))
    );

    let mut request = sample::say_request();
    let too_long = MAXIMUM_INTENT_TEXT_BYTES + 1;
    if let Some(Payload::Say(text)) = intent_mut(&mut request).payload.as_mut() {
        text.text = "a".repeat(too_long);
    }
    assert_eq!(
        admit_machine_intent(&request),
        Err(MachineIntentError::TextTooLong { bytes: too_long })
    );

    // The longest allowed line is admitted, and still fits the bridge's message limit.
    let mut request = sample::say_request();
    if let Some(Payload::Say(text)) = intent_mut(&mut request).payload.as_mut() {
        text.text = "a".repeat(MAXIMUM_INTENT_TEXT_BYTES);
    }
    assert!(admit_machine_intent(&request).is_ok());
    assert!(request.encoded_len() < 64 * 1024);
}

#[test]
fn a_pending_answer_names_the_pending_and_the_chosen_offer() {
    let answer = |pending_id: Option<u64>, offer_id: Option<&str>| {
        let mut request = sample::tap_request();
        let intent = intent_mut(&mut request);
        intent.offer_id = offer_id.map(str::to_owned);
        intent.payload = Some(Payload::PendingAnswer(wire::MachinePendingAnswer {
            pending_id,
        }));
        admit_machine_intent(&request)
    };
    assert_eq!(
        answer(Some(9), Some("yes")).expect("admitted").payload,
        AdmittedPayload::PendingAnswer {
            pending_id: 9,
            offer_id: "yes".to_owned()
        }
    );
    assert_eq!(
        answer(None, Some("yes")),
        Err(MachineIntentError::Missing(IntentSlot::PendingId))
    );
    assert_eq!(
        answer(Some(9), None),
        Err(MachineIntentError::Missing(IntentSlot::OfferId))
    );
}

#[test]
fn a_view_with_a_field_from_a_newer_server_still_decodes_and_checks() {
    // Tag 15 (a field this build does not know) appended to a sample: protobuf keeps decoding
    // and the guard judges only the fields it knows.
    let mut bytes = seat().encode_to_vec();
    bytes.extend_from_slice(&[0x7A, 0x00]);
    let decoded = wire::MachineView::decode(bytes.as_slice()).expect("unknown fields are skipped");
    assert_eq!(check_machine_view(&decoded), Ok(()));
}
