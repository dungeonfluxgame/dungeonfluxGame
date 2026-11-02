use std::collections::BTreeSet;

use crate::envelope::{
    EngineInput, InputEnvelope, InputKind, InputOrigin, PendingId, PlayerInput, ProposalOrigin,
    ProposerRef, TimerName, UtteranceId, ViewStamp,
};
use crate::identity::{EvidenceRef, OperationId, RecoveryEpoch, SeatId, SessionRevision};
use crate::ports::{ActionId, Readiness, ResourceHandle};
use crate::private_data::{LengthClass, PlayerText};

const SENTINEL_LINE: &str = "SENTINEL-TYPED-LINE-5d21";

fn operation() -> OperationId {
    OperationId::from_hex("00000000000000000000000000000abc").expect("valid operation id")
}

fn view() -> ViewStamp {
    ViewStamp::new(SessionRevision::new(RecoveryEpoch::FIRST, 4))
}

fn action(name: &str) -> ActionId {
    ActionId::new(name).expect("valid action")
}

fn every_envelope() -> Vec<InputEnvelope> {
    let seat = SeatId::new(2);
    let text = || PlayerText::new(SENTINEL_LINE);
    let player = |input| InputEnvelope::from_player(operation(), seat, view(), input);
    let engine = |input| InputEnvelope::from_engine(operation(), view(), input);
    vec![
        player(PlayerInput::Tap {
            action: action("look"),
        }),
        player(PlayerInput::SubmitText {
            utterance: UtteranceId::new(1),
            text: text(),
        }),
        player(PlayerInput::Say {
            utterance: UtteranceId::new(2),
            text: text(),
        }),
        player(PlayerInput::PrivateMessage {
            utterance: UtteranceId::new(3),
            to: SeatId::new(3),
            text: text(),
        }),
        player(PlayerInput::SafetyCard),
        engine(EngineInput::Proposal {
            origin: ProposalOrigin::Npc,
            proposer: ProposerRef::new("drunk_patron").expect("valid proposer"),
        }),
        engine(EngineInput::Timer {
            timer: TimerName::new("idle_nudge").expect("valid timer"),
        }),
        engine(EngineInput::ResourceResult {
            handle: ResourceHandle::new(7),
            readiness: Readiness::Ready,
        }),
        player(PlayerInput::PendingAnswer {
            pending: PendingId::new(5),
            choice: action("accept"),
        }),
    ]
}

#[test]
fn the_nine_inputs_have_the_nine_distinct_kinds_of_the_plan() {
    let kinds: Vec<InputKind> = every_envelope().iter().map(InputEnvelope::kind).collect();
    let distinct: BTreeSet<&str> = kinds.iter().map(|kind| kind.key()).collect();
    assert_eq!(distinct.len(), InputKind::ALL.len());
    assert!(InputKind::ALL.iter().all(|kind| kinds.contains(kind)));
}

#[test]
fn a_player_input_names_its_seat_and_an_engine_input_never_does() {
    for envelope in every_envelope() {
        match envelope.origin() {
            InputOrigin::Player { seat, .. } => {
                assert_eq!(envelope.seat(), Some(*seat), "{:?}", envelope.kind());
            }
            InputOrigin::Engine(_) => assert_eq!(envelope.seat(), None, "{:?}", envelope.kind()),
        }
    }
}

#[test]
fn only_the_three_text_kinds_have_a_length_and_the_rest_are_empty() {
    let with_text: BTreeSet<&str> = every_envelope()
        .iter()
        .filter(|envelope| envelope.length_class() != LengthClass::Empty)
        .map(|envelope| envelope.kind().key())
        .collect();
    assert_eq!(
        with_text,
        BTreeSet::from(["submit_text", "say", "private_message"])
    );
}

#[test]
fn the_length_class_follows_the_characters_of_the_text() {
    let line = |characters: usize| {
        InputEnvelope::from_player(
            operation(),
            SeatId::new(1),
            view(),
            PlayerInput::SubmitText {
                utterance: UtteranceId::new(1),
                text: PlayerText::new("\u{e9}".repeat(characters)),
            },
        )
        .length_class()
    };
    assert_eq!(line(0), LengthClass::Empty);
    assert_eq!(line(40), LengthClass::Short);
    assert_eq!(line(41), LengthClass::Medium);
    assert_eq!(line(280), LengthClass::Medium);
    assert_eq!(line(281), LengthClass::Long);
}

#[test]
fn printing_an_envelope_never_shows_the_typed_text() {
    for envelope in every_envelope() {
        let printed = format!("{envelope:?}");
        assert!(!printed.contains(SENTINEL_LINE), "{printed}");
    }
}

#[test]
fn an_envelope_keeps_its_view_stamp_and_takes_evidence_only_when_given() {
    let envelope =
        InputEnvelope::from_player(operation(), SeatId::new(1), view(), PlayerInput::SafetyCard);
    assert_eq!(envelope.view(), view());
    assert_eq!(envelope.view().revision().sequence(), 4);
    assert_eq!(envelope.operation_id(), operation());
    assert_eq!(envelope.evidence(), None);
    let specimen = EvidenceRef::from_hex("00000000000000000000000000000e01").expect("valid");
    assert_eq!(envelope.with_evidence(specimen).evidence(), Some(specimen));
}
