use std::num::{NonZeroU16, NonZeroU128};

use crate::event::{FailureCause, TableEvent};
use crate::identity::{FactId, SeatId};
use crate::outcome::{Decline, DeclineCode, OutcomeKind};
use crate::ports::{
    ActionId, Audience, Broker, Clock, CommittedOutcome, Dice, DrawRequest, EventSink,
    Interpretation, Interpreter, KnowledgeView, Narrator, PortError, PortName, Preparation,
    PurposeName, Readiness, Resolution, ResourceHandle, RulesOwner,
};
use crate::private_data::PlayerText;
use crate::testkit::{
    FailingNarrator, FixedKnowledgeView, ManualClock, QueuedDice, RecordingSink,
    SENTINEL_PUBLIC_FACT, SENTINEL_SECRET, ScriptedBroker, ScriptedInterpreter, ScriptedNarrator,
    ScriptedRulesOwner, correlation,
};

fn action(name: &str) -> ActionId {
    ActionId::new(name).expect("valid action id")
}

fn sides(count: u16) -> NonZeroU16 {
    NonZeroU16::new(count).expect("a die has sides")
}

fn fact(number: u128) -> FactId {
    FactId::from_nonzero_u128(NonZeroU128::new(number).expect("nonzero"))
}

fn committed() -> CommittedOutcome {
    CommittedOutcome {
        seat: SeatId::new(1),
        action: action("attack"),
        outcome: OutcomeKind::Applied,
        steps: vec![action("attack")],
        facts: Vec::new(),
    }
}

fn purpose() -> PurposeName {
    PurposeName::new("plate").expect("valid purpose")
}

#[test]
fn every_fake_is_send_and_sync() {
    fn shared<T: Send + Sync>() {}
    shared::<ManualClock>();
    shared::<ScriptedBroker>();
    shared::<ScriptedInterpreter>();
    shared::<ScriptedRulesOwner>();
    shared::<ScriptedNarrator>();
    shared::<FailingNarrator>();
    shared::<QueuedDice>();
    shared::<FixedKnowledgeView>();
    shared::<RecordingSink>();
}

#[test]
fn the_manual_clock_moves_only_when_told_to() {
    let clock = ManualClock::starting_at(1_000);
    assert_eq!(clock.now_ms(), 1_000);
    assert_eq!(clock.now_ms(), 1_000);
    clock.advance_ms(250);
    assert_eq!(clock.now_ms(), 1_250);
    assert_eq!(ManualClock::default().now_ms(), 0);
}

#[test]
fn each_broker_handle_answers_from_its_own_script_in_order() {
    let broker = ScriptedBroker::default()
        .with_answers_for_next_request([Ok(Readiness::Pending), Ok(Readiness::Ready)])
        .with_answers_for_next_request([Err(FailureCause::Refused)]);
    let first = broker
        .request(correlation(1), &purpose())
        .expect("a request is accepted");
    let second = broker
        .request(correlation(2), &purpose())
        .expect("a request is accepted");
    assert_eq!((first.get(), second.get()), (1, 2));
    assert_eq!(broker.requests(), [purpose(), purpose()]);
    assert_eq!(
        broker.readiness(correlation(2), second),
        Err(PortError::new(PortName::Broker, FailureCause::Refused))
    );
    assert_eq!(
        broker.readiness(correlation(1), first),
        Ok(Readiness::Pending)
    );
    assert_eq!(
        broker.readiness(correlation(1), first),
        Ok(Readiness::Ready)
    );
}

#[test]
fn readiness_of_an_unissued_handle_is_an_error() {
    let broker = ScriptedBroker::default().with_answers_for_next_request([Ok(Readiness::Ready)]);
    assert_eq!(
        broker.readiness(correlation(1), ResourceHandle::new(1)),
        Err(PortError::new(PortName::Broker, FailureCause::Stale))
    );
    let issued = broker
        .request(correlation(1), &purpose())
        .expect("a request is accepted");
    assert_eq!(
        broker.readiness(correlation(1), ResourceHandle::new(issued.get() + 1)),
        Err(PortError::new(PortName::Broker, FailureCause::Stale))
    );
}

#[test]
fn the_broker_forgets_the_oldest_requests_beyond_its_maximum_and_counts_them() {
    let broker = ScriptedBroker::remembering_newest(2)
        .with_answers_for_next_request([Ok(Readiness::Ready)])
        .with_answers_for_next_request([Ok(Readiness::Ready)])
        .with_answers_for_next_request([Ok(Readiness::Ready)]);
    let handles: Vec<_> = (1..=3)
        .map(|turn| {
            broker
                .request(correlation(turn), &purpose())
                .expect("a request is accepted")
        })
        .collect();
    assert_eq!(broker.requests().len(), 2);
    assert_eq!(broker.forgotten_requests(), 1);
    assert_eq!(
        broker.readiness(correlation(1), handles[0]),
        Err(PortError::new(PortName::Broker, FailureCause::Stale))
    );
    assert_eq!(
        broker.readiness(correlation(3), handles[2]),
        Ok(Readiness::Ready)
    );
}

#[test]
#[should_panic(expected = "the broker fake was asked the readiness of handle 1")]
fn a_readiness_question_beyond_the_script_fails_the_test() {
    let broker = ScriptedBroker::default();
    let handle = broker
        .request(correlation(1), &purpose())
        .expect("a request is accepted");
    let _ = broker.readiness(correlation(1), handle);
}

#[test]
fn the_scripted_interpreter_knows_only_the_lines_it_was_given() {
    let interpreter = ScriptedInterpreter::default()
        .with_line("I attack", Interpretation::Action(action("attack")))
        .with_line("hmm", Interpretation::NeedsClarification);
    let ask = |line: &str| {
        interpreter.interpret(correlation(1), SeatId::new(1), &PlayerText::new(line), &[])
    };
    assert_eq!(
        ask("I attack"),
        Ok(Interpretation::Action(action("attack")))
    );
    assert_eq!(ask("hmm"), Ok(Interpretation::NeedsClarification));
}

#[test]
#[should_panic(
    expected = "the interpreter fake was asked to interpret a line (PlayerText(length=short))"
)]
fn a_line_outside_the_script_fails_the_test_without_printing_the_line() {
    let interpreter = ScriptedInterpreter::default();
    let _ = interpreter.interpret(
        correlation(1),
        SeatId::new(1),
        &PlayerText::new("SENTINEL-PRIVATE"),
        &[],
    );
}

#[test]
fn a_failing_interpreter_fails_every_line_with_its_cause() {
    let interpreter = ScriptedInterpreter::failing(FailureCause::TimedOut);
    assert_eq!(
        interpreter.interpret(correlation(1), SeatId::new(1), &PlayerText::new("x"), &[]),
        Err(PortError::new(
            PortName::Interpreter,
            FailureCause::TimedOut
        ))
    );
}

#[test]
fn the_scripted_rules_owner_lists_choices_prepares_and_resolves() {
    let rules = ScriptedRulesOwner::default()
        .with_choices(SeatId::new(1), vec![action("attack"), action("dash")])
        .with_action(
            action("attack"),
            Preparation::Ready {
                draws: vec![DrawRequest { sides: sides(20) }],
            },
            Resolution::applied(),
        );
    let seat = SeatId::new(1);
    assert_eq!(
        rules.legal_choices(correlation(1), seat),
        Ok(vec![action("attack"), action("dash")])
    );
    assert_eq!(
        rules.prepare(correlation(1), seat, &action("attack")),
        Ok(Preparation::Ready {
            draws: vec![DrawRequest { sides: sides(20) }]
        })
    );
    assert_eq!(
        rules.resolve(correlation(1), seat, &action("attack"), &[]),
        Ok(Resolution::applied())
    );
}

#[test]
fn a_rules_decline_reaches_the_caller_unchanged_with_its_missing_piece_and_alternatives() {
    let decline = Decline::new(DeclineCode::MissingRequirement, "You need a torch.")
        .and_then(|decline| decline.with_missing_piece("a torch"))
        .and_then(|decline| decline.with_alternative("Feel along the wall"))
        .expect("a well-formed decline builds");
    let rules = ScriptedRulesOwner::default().with_action(
        action("search"),
        Preparation::Declined(decline.clone()),
        Resolution::Declined(decline.clone()),
    );
    let seat = SeatId::new(1);
    assert_eq!(
        rules.prepare(correlation(1), seat, &action("search")),
        Ok(Preparation::Declined(decline.clone()))
    );
    let Ok(Resolution::Declined(read_back)) =
        rules.resolve(correlation(1), seat, &action("search"), &[])
    else {
        panic!("the scripted resolution is a decline");
    };
    assert_eq!(read_back.missing_piece(), Some("a torch"));
    assert_eq!(read_back.alternatives(), ["Feel along the wall".to_owned()]);
    assert_eq!(read_back, decline);
}

#[test]
fn an_action_the_rules_owner_cannot_run_yet_is_scripted_as_not_supported_yet() {
    let rules = ScriptedRulesOwner::default().with_unsupported_action(action("polymorph"));
    let Ok(Preparation::Declined(decline)) =
        rules.prepare(correlation(1), SeatId::new(1), &action("polymorph"))
    else {
        panic!("the scripted preparation is a decline");
    };
    assert_eq!(decline.code(), DeclineCode::NotSupportedYet);
    assert_eq!(decline.reason(), "I can't run that yet.");
}

#[test]
#[should_panic(expected = "the rules_owner fake was asked about the action polymorph")]
fn an_action_outside_the_script_is_not_a_decline_it_fails_the_test() {
    let rules = ScriptedRulesOwner::default();
    let _ = rules.prepare(correlation(1), SeatId::new(1), &action("polymorph"));
}

#[test]
#[should_panic(expected = "the rules_owner fake was asked for the legal choices of seat 1")]
fn a_seat_outside_the_script_fails_the_test() {
    let rules = ScriptedRulesOwner::default();
    let _ = rules.legal_choices(correlation(1), SeatId::new(1));
}

#[test]
fn the_scripted_narrator_speaks_its_lines_in_order() {
    let narrator = ScriptedNarrator::default()
        .with_line("Pip strikes.")
        .with_line("Dain falls.");
    let narrate = || narrator.narrate(correlation(1), &committed());
    assert_eq!(narrate().expect("first line").expose(), "Pip strikes.");
    assert_eq!(narrate().expect("second line").expose(), "Dain falls.");
}

#[test]
#[should_panic(expected = "the narrator fake was asked to narrate beyond its queued lines")]
fn narrating_beyond_the_queued_lines_fails_the_test() {
    let narrator = ScriptedNarrator::default();
    let _ = narrator.narrate(correlation(1), &committed());
}

#[test]
fn the_failing_narrator_fails_with_the_cause_it_was_given() {
    let narrator = FailingNarrator {
        cause: FailureCause::TextInvalid,
    };
    assert_eq!(
        narrator
            .narrate(correlation(1), &committed())
            .map(|narration| narration.expose().to_owned()),
        Err(PortError::new(
            PortName::Narrator,
            FailureCause::TextInvalid
        ))
    );
}

#[test]
fn queued_dice_give_the_queued_values_in_order() {
    let dice = QueuedDice::new([17, 3]);
    let request = DrawRequest { sides: sides(20) };
    assert_eq!(
        dice.draw(correlation(1), request).map(|draw| draw.value()),
        Ok(17)
    );
    assert_eq!(
        dice.draw(correlation(1), request).map(|draw| draw.value()),
        Ok(3)
    );
}

#[test]
#[should_panic(expected = "the dice fake was asked for a die beyond its queued values")]
fn drawing_beyond_the_queued_values_fails_the_test() {
    let dice = QueuedDice::new([]);
    let _ = dice.draw(correlation(1), DrawRequest { sides: sides(20) });
}

#[test]
fn a_queued_value_that_is_not_on_the_die_is_an_invalid_answer() {
    let dice = QueuedDice::new([21]);
    assert_eq!(
        dice.draw(correlation(1), DrawRequest { sides: sides(20) }),
        Err(PortError::new(PortName::Dice, FailureCause::InvalidAnswer))
    );
}

fn knowledge() -> FixedKnowledgeView {
    FixedKnowledgeView::default()
        .with_fact(Audience::Public, fact(1), SENTINEL_PUBLIC_FACT)
        .with_fact(Audience::Seat(SeatId::new(1)), fact(2), SENTINEL_SECRET)
}

fn ask(view: &FixedKnowledgeView, audience: Audience, fact: FactId) -> Option<String> {
    view.fact_for(correlation(1), audience, &fact)
        .expect("a fixed view never fails")
        .map(|secret| secret.expose().clone())
}

#[test]
fn a_seat_knows_the_public_facts_and_its_own_secret() {
    let view = knowledge();
    let seat_one = Audience::Seat(SeatId::new(1));
    assert_eq!(
        ask(&view, seat_one, fact(1)).as_deref(),
        Some(SENTINEL_PUBLIC_FACT)
    );
    assert_eq!(
        ask(&view, seat_one, fact(2)).as_deref(),
        Some(SENTINEL_SECRET)
    );
}

#[test]
fn another_seat_and_the_public_never_receive_a_seats_secret() {
    let view = knowledge();
    assert_eq!(ask(&view, Audience::Seat(SeatId::new(2)), fact(2)), None);
    assert_eq!(ask(&view, Audience::Public, fact(2)), None);
    assert_eq!(
        ask(&view, Audience::Public, fact(1)).as_deref(),
        Some(SENTINEL_PUBLIC_FACT)
    );
}

#[test]
fn the_recording_sink_keeps_events_in_arrival_order() {
    let sink = RecordingSink::default();
    sink.record(&TableEvent::KindsRejected { errors: 1 });
    sink.record(&TableEvent::KindsRejected { errors: 2 });
    assert_eq!(
        sink.events(),
        [
            TableEvent::KindsRejected { errors: 1 },
            TableEvent::KindsRejected { errors: 2 }
        ]
    );
    assert_eq!(sink.dropped_events(), 0);
}

#[test]
fn the_recording_sink_keeps_only_the_newest_events_and_counts_the_rest() {
    let sink = RecordingSink::keeping_newest(2);
    for errors in 1..=5 {
        sink.record(&TableEvent::KindsRejected { errors });
    }
    assert_eq!(
        sink.events(),
        [
            TableEvent::KindsRejected { errors: 4 },
            TableEvent::KindsRejected { errors: 5 }
        ]
    );
    assert_eq!(sink.dropped_events(), 3);
}

#[test]
fn sample_correlations_differ_in_every_field() {
    let first = correlation(1);
    let second = correlation(2);
    assert_ne!(first.operation_id, second.operation_id);
    assert_ne!(first.turn_seq, second.turn_seq);
    assert_ne!(first.revision, second.revision);
    assert_ne!(correlation(0).operation_id, correlation(1).operation_id);
}
