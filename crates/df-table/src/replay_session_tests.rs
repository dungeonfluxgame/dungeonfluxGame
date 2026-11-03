//! The recording format and the recorder: every kind of input round-trips without its text, and a
//! recorder refuses a turn that breaks the chain, the order, the bound or its own input.

use crate::envelope::{
    CancelCause, EngineControl, EngineInput, InputEnvelope, InputKind, PendingId, PlayerInput,
    ProposalOrigin, ProposerRef, TimerName, UtteranceId, ViewStamp,
};
use crate::event::FailureCause;
use crate::identity::{OperationId, RecoveryEpoch, SeatId, SessionRevision, TableId};
use crate::pipeline::TurnRecord;
use crate::pipeline_fixtures_tests::{action, seat};
use crate::ports::{Readiness, ResourceHandle, RulingKey, RulingScope};
use crate::private_data::{LengthClass, PlayerText};
use crate::replay::{LineKind, LineTextError, RecordedInput, SessionError, SessionRecorder};
use crate::replay_tests::scripted_world;
use crate::table_state::Presence;

fn operation() -> OperationId {
    OperationId::from_nonzero_u128(std::num::NonZeroU128::new(9).expect("nonzero"))
}

fn view() -> ViewStamp {
    ViewStamp::new(SessionRevision::new(RecoveryEpoch::FIRST, 4))
}

fn player(input: PlayerInput) -> InputEnvelope {
    InputEnvelope::from_player(operation(), seat(2), view(), input)
}

fn engine(input: EngineInput) -> InputEnvelope {
    InputEnvelope::from_engine(operation(), view(), input)
}

fn every_kind_of_input() -> Vec<(InputEnvelope, Option<PlayerText>)> {
    let line = PlayerText::new("a line of some length");
    let utterance = UtteranceId::new(5);
    vec![
        (
            player(PlayerInput::Tap {
                action: action("look"),
            }),
            None,
        ),
        (
            player(PlayerInput::SubmitText {
                utterance,
                text: line.clone(),
            }),
            Some(line.clone()),
        ),
        (
            player(PlayerInput::Say {
                utterance,
                text: line.clone(),
            }),
            Some(line.clone()),
        ),
        (
            player(PlayerInput::PrivateMessage {
                utterance,
                to: SeatId::new(3),
                text: line.clone(),
            }),
            Some(line),
        ),
        (player(PlayerInput::SafetyCard), None),
        (
            player(PlayerInput::PendingAnswer {
                pending: PendingId::new(7),
                choice: action("keep"),
            }),
            None,
        ),
        (
            engine(EngineInput::Control(EngineControl::SeatPresence {
                seat: seat(1),
                presence: Presence::Sleeping,
            })),
            None,
        ),
        (
            engine(EngineInput::Control(EngineControl::RevokeRuling {
                key: RulingKey::new("rope-climb").expect("valid key"),
                scope: RulingScope::Table,
            })),
            None,
        ),
        (
            engine(EngineInput::Control(EngineControl::CancelPending {
                pending: PendingId::new(2),
                cause: CancelCause::HostDecision,
            })),
            None,
        ),
        (
            engine(EngineInput::Control(EngineControl::ResumeTable)),
            None,
        ),
        (
            engine(EngineInput::Proposal {
                origin: ProposalOrigin::Encounter,
                proposer: ProposerRef::new("ambush").expect("valid proposer"),
            }),
            None,
        ),
        (
            engine(EngineInput::Timer {
                timer: TimerName::new("idle_nudge").expect("valid timer"),
            }),
            None,
        ),
        (
            engine(EngineInput::ResourceResult {
                handle: ResourceHandle::new(11),
                readiness: Readiness::Failed(FailureCause::TimedOut),
            }),
            None,
        ),
    ]
}

#[test]
fn every_kind_of_input_is_recorded_and_builds_the_same_envelope_again() {
    let inputs = every_kind_of_input();
    let kinds: std::collections::BTreeSet<&str> = inputs
        .iter()
        .map(|(envelope, _)| envelope.kind().key())
        .collect();
    assert_eq!(
        kinds.len(),
        InputKind::ALL.len(),
        "a kind of input is untested"
    );

    for (envelope, text) in inputs {
        let recorded = RecordedInput::of(&envelope);

        assert_eq!(recorded.kind(), envelope.kind());
        assert_eq!(recorded.sender(), envelope.sender());
        let json = serde_json::to_string(&recorded).expect("writes");
        let read: RecordedInput = serde_json::from_str(&json).expect("reads");
        assert_eq!(read, recorded, "{json}");
        let built = read
            .into_envelope(operation(), view(), text.as_ref())
            .expect("builds");
        assert_eq!(built, envelope, "{json}");
    }
}

#[test]
fn a_recorded_line_holds_its_kind_utterance_and_length_class_and_never_its_words() {
    let words = "the secret words of a player";
    let envelope = player(PlayerInput::PrivateMessage {
        utterance: UtteranceId::new(5),
        to: SeatId::new(3),
        text: PlayerText::new(words),
    });

    let recorded = RecordedInput::of(&envelope);

    assert_eq!(
        recorded,
        RecordedInput::Line {
            seat: seat(2),
            line: LineKind::PrivateMessage { to: SeatId::new(3) },
            utterance: 5,
            length: LengthClass::Short,
        }
    );
    assert!(
        !serde_json::to_string(&recorded)
            .expect("writes")
            .contains("secret")
    );
    assert!(!format!("{recorded:?}").contains("secret"));
}

#[test]
fn words_of_another_length_class_or_no_words_cannot_rebuild_a_recorded_line() {
    let recorded = RecordedInput::Line {
        seat: seat(2),
        line: LineKind::Say,
        utterance: 1,
        length: LengthClass::Short,
    };
    let long = PlayerText::new("x".repeat(500));

    assert_eq!(
        recorded.into_envelope(operation(), view(), None),
        Err(LineTextError::Missing)
    );
    assert_eq!(
        recorded.into_envelope(operation(), view(), Some(&long)),
        Err(LineTextError::LengthChanged {
            recorded: LengthClass::Short,
            given: LengthClass::Long
        })
    );
}

/// Two turn records from a real run, to feed a recorder.
fn two_turns() -> (
    (InputEnvelope, TurnRecord, u64),
    (InputEnvelope, TurnRecord, u64),
    u64,
) {
    let mut world = scripted_world().expect("builds");
    let initial = world.table.state_hash();
    let mut run = |name: &str| {
        let probe = crate::testkit::correlation(world.next_turn());
        let envelope = InputEnvelope::from_player(
            probe.operation_id,
            seat(1),
            ViewStamp::new(probe.revision),
            PlayerInput::Tap {
                action: action(name),
            },
        );
        let result = world.run_envelope(&envelope).expect("runs");
        (envelope, result.record, world.table.state_hash())
    };
    let first = run("look");
    let second = run("look");
    (first, second, initial)
}

#[test]
fn a_recorder_takes_turns_in_order_and_finishes_a_session_that_verifies() {
    let (first, second, initial) = two_turns();
    let mut recorder = SessionRecorder::begin(TableId::new(1), initial);

    recorder
        .record(&first.0, &first.1, first.2)
        .expect("the first");
    recorder
        .record(&second.0, &second.1, second.2)
        .expect("the second");
    let session = recorder.finish();

    assert_eq!(session.turns().len(), 2);
    assert_eq!(session.final_state_hash(), second.2);
    assert_eq!(session.verify(), Ok(()));
}

#[test]
fn a_recorder_refuses_a_turn_that_does_not_start_where_the_last_one_settled_and_stays_as_it_was() {
    let (first, second, initial) = two_turns();
    let mut recorder = SessionRecorder::begin(TableId::new(1), initial);

    let skipped = recorder.record(&second.0, &second.1, second.2);

    assert!(matches!(
        skipped,
        Err(SessionError::ChainBroken { turn: 0, expected, .. }) if expected == initial
    ));
    assert!(recorder.is_empty());
    recorder
        .record(&first.0, &first.1, first.2)
        .expect("the first still fits");
    assert_eq!(recorder.len(), 1);
}

#[test]
fn a_recorder_refuses_a_turn_numbered_at_or_before_the_one_before_it() {
    let (first, second, initial) = two_turns();
    let mut recorder = SessionRecorder::begin(TableId::new(1), initial);
    recorder
        .record(&first.0, &first.1, first.2)
        .expect("the first");
    let mut again = second.1.clone();
    again.correlation.turn_seq = first.1.correlation.turn_seq;

    let refused = recorder.record(&second.0, &again, second.2);

    assert_eq!(refused, Err(SessionError::TurnOutOfOrder { turn: 1 }));
}

#[test]
fn a_recorder_refuses_a_record_that_names_another_kind_of_input_than_the_input_it_ran() {
    let (first, _, initial) = two_turns();
    let mut recorder = SessionRecorder::begin(TableId::new(1), initial);
    let mut other = first.1.clone();
    other.kind = InputKind::Timer;

    let refused = recorder.record(&first.0, &other, first.2);

    assert_eq!(refused, Err(SessionError::InputDisagrees { turn: 0 }));
}

#[test]
fn a_recorder_stops_at_its_bound_and_names_the_bound_in_the_error() {
    let (first, second, initial) = two_turns();
    let mut recorder = SessionRecorder::begin(TableId::new(1), initial).holding_at_most(1);
    recorder
        .record(&first.0, &first.1, first.2)
        .expect("the first fits");

    let refused = recorder.record(&second.0, &second.1, second.2);

    assert_eq!(refused, Err(SessionError::TooManyTurns));
    assert_eq!(recorder.len(), 1);
}
