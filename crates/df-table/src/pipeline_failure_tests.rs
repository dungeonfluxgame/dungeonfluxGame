//! The model-failure fallbacks of C.7 and the port failures around them: a failed or slow model
//! call never guesses and never freezes the table, and each failure is recorded once.

use std::sync::Arc;

use crate::event::FailureCause;
use crate::golden::{FixedNarrator, SlowNarrator, World};
use crate::outcome::{DeclineCode, Outcome};
use crate::pipeline::{ClarifyCause, PipelineLimits};
use crate::pipeline_fixtures_tests::{
    action, allows, changes, d20, failures_of, lines, narrated, ran, seat, slot_of, world_of,
};
use crate::ports::{Interpretation, PortName, Preparation, Resolution};
use crate::testkit::{FailingNarrator, RulesCall, ScriptedInterpreter, ScriptedRulesOwner};

fn reading_world(interpreter: ScriptedInterpreter) -> World {
    let rules = allows(
        allows(
            ScriptedRulesOwner::default()
                .with_choices(seat(1), vec![action("wave"), action("bow")]),
            "wave",
        ),
        "bow",
    );
    let mut world = world_of(&["wave", "bow"], 2, rules);
    world.interpreter = interpreter;
    world
}

#[test]
fn a_first_attempt_that_times_out_is_retried_once_and_the_retry_applies() {
    let interpreter = ScriptedInterpreter::default().with_attempts([
        Err(FailureCause::TimedOut),
        Ok(Interpretation::Action(action("wave"))),
    ]);
    let mut world = reading_world(interpreter);

    let result = world.text(1, "wave at them").expect("the line runs");

    assert_eq!(result.outcome, Outcome::Applied);
    assert_eq!(world.interpreter.calls().first_attempts, 1);
    assert_eq!(world.interpreter.calls().retries, 1);
    assert_eq!(failures_of(&world.events()), []);
}

#[test]
fn an_interpreter_that_fails_twice_gives_the_legal_choices_as_buttons_and_keeps_the_draft() {
    let interpreter = ScriptedInterpreter::failing(FailureCause::TimedOut);
    let mut world = reading_world(interpreter);
    let slot_before = slot_of(&world);

    let result = world.text(1, "do something clever").expect("the line runs");

    assert_eq!(result.outcome, Outcome::NeedsClarification);
    let clarification = result.clarification.as_ref().expect("a clarification");
    assert_eq!(clarification.cause, ClarifyCause::Unreadable);
    assert!(clarification.keeps_draft);
    let names: Vec<&str> = clarification
        .options
        .iter()
        .map(|option| option.as_str())
        .collect();
    assert_eq!(names, ["wave", "bow"]);
    assert_eq!(result.readout.as_str(), "I could not read that. Pick one:");
    assert_eq!(
        failures_of(&world.events()),
        [(PortName::Interpreter, FailureCause::TimedOut)]
    );
    assert!(
        !world
            .rules
            .calls()
            .iter()
            .any(|call| matches!(call, RulesCall::Prepare(_))),
        "a failed reading is never guessed at"
    );
    assert_eq!(
        slot_before,
        slot_of(&world),
        "a failed reading changes nothing of the machine"
    );
    assert!(changes(&result).is_empty());
}

#[test]
fn a_table_whose_interpreter_failed_is_not_frozen_the_next_tap_applies() {
    let interpreter = ScriptedInterpreter::failing(FailureCause::Unavailable);
    let mut world = reading_world(interpreter);
    world.text(1, "do something clever").expect("the line runs");

    let result = ran(&mut world, 1, "wave");

    assert_eq!(result.outcome, Outcome::Applied);
}

#[test]
fn a_tap_on_a_button_after_a_failed_reading_is_the_same_as_any_tap() {
    let interpreter = ScriptedInterpreter::failing(FailureCause::Unavailable);
    let mut world = reading_world(interpreter);
    world.text(1, "do something clever").expect("the line runs");

    let result = ran(&mut world, 1, "bow");

    assert_eq!(result.outcome, Outcome::Applied);
    assert!(result.record.draws.is_empty());
}

#[test]
fn a_refusal_is_never_retried() {
    let interpreter = ScriptedInterpreter::failing(FailureCause::Refused);
    let mut world = reading_world(interpreter);

    let result = world
        .text(1, "something the model refuses")
        .expect("the line runs");

    assert_eq!(result.outcome, Outcome::NeedsClarification);
    assert_eq!(world.interpreter.calls().first_attempts, 1);
    assert_eq!(world.interpreter.calls().retries, 0);
    assert_eq!(
        failures_of(&world.events()),
        [(PortName::Interpreter, FailureCause::Refused)]
    );
}

#[test]
fn an_unregistered_action_from_the_model_is_garbage_not_a_guess() {
    let interpreter = ScriptedInterpreter::default().with_line(
        "fly away",
        Interpretation::Action(action("fly_to_the_moon")),
    );
    let mut world = reading_world(interpreter);

    let result = world.text(1, "fly away").expect("the line runs");

    assert_eq!(result.outcome, Outcome::NeedsClarification);
    let clarification = result.clarification.as_ref().expect("the buttons stand in");
    assert_eq!(clarification.cause, ClarifyCause::Unreadable);
    assert!(clarification.keeps_draft);
    assert_eq!(
        world.interpreter.calls().retries,
        1,
        "garbage is retried once, like a failed call"
    );
    assert_eq!(
        failures_of(&world.events()),
        [(PortName::Interpreter, FailureCause::TextInvalid)],
        "the failed reading is recorded once for the input, not once per attempt"
    );
    assert!(changes(&result).is_empty());
    assert_eq!(world.dice.drawn(), 0);
}

#[test]
fn an_ambiguity_with_one_candidate_or_an_unknown_one_is_garbage_and_asks_with_the_buttons() {
    let interpreter = ScriptedInterpreter::default()
        .with_line(
            "one",
            Interpretation::Ambiguous {
                candidates: vec![action("wave")],
            },
        )
        .with_line(
            "two",
            Interpretation::Ambiguous {
                candidates: vec![action("wave"), action("nothing_like_it")],
            },
        );
    let mut world = reading_world(interpreter);

    let one = world.text(1, "one").expect("runs");
    let two = world.text(1, "two").expect("runs");

    for result in [&one, &two] {
        assert_eq!(result.outcome, Outcome::NeedsClarification);
        assert_eq!(
            result
                .clarification
                .as_ref()
                .map(|c| (c.cause, c.keeps_draft)),
            Some((ClarifyCause::Unreadable, true))
        );
    }
    assert_eq!(world.interpreter.calls().retries, 2);
    assert_eq!(
        failures_of(&world.events()).len(),
        2,
        "each unreadable input is recorded once"
    );
}

#[test]
fn a_model_that_declines_the_line_is_a_valid_answer_with_no_failure_recorded() {
    let interpreter = ScriptedInterpreter::default().with_line("hmm", Interpretation::Rejected);
    let mut world = reading_world(interpreter);

    let result = world.text(1, "hmm").expect("the line runs");

    assert_eq!(result.record.declined, Some(DeclineCode::IntentInvalid));
    assert_eq!(failures_of(&world.events()), []);
}

#[test]
fn an_unclear_reading_asks_with_the_legal_choices_and_does_not_keep_a_draft() {
    let interpreter =
        ScriptedInterpreter::default().with_line("um", Interpretation::NeedsClarification);
    let mut world = reading_world(interpreter);

    let result = world.text(1, "um").expect("the line runs");

    let clarification = result.clarification.as_ref().expect("a clarification");
    assert_eq!(clarification.cause, ClarifyCause::Unclear);
    assert!(!clarification.keeps_draft);
    assert_eq!(clarification.options.len(), 2);
    assert!(
        lines(&world)
            .contains(&"INPUT clarified scene=1 machine=scene options=2 cause=unclear".to_owned())
    );
}

#[test]
fn a_rules_owner_that_fails_while_the_options_are_listed_declines_and_records_it_once() {
    let mut world = reading_world(ScriptedInterpreter::default());
    world.rules = ScriptedRulesOwner::failing(FailureCause::Unavailable);

    let result = world.text(1, "wave").expect("the line runs");

    assert_eq!(result.record.declined, Some(DeclineCode::NotSupportedYet));
    assert_eq!(
        failures_of(&world.events()),
        [(PortName::RulesOwner, FailureCause::Unavailable)]
    );
}

#[test]
fn a_failed_narrator_leaves_the_structured_readout_alone_and_is_recorded_once() {
    let mut world = world_of(&["wave"], 1, allows(ScriptedRulesOwner::default(), "wave"));
    world.narrator = Box::new(FailingNarrator {
        cause: FailureCause::Unavailable,
    });

    let result = ran(&mut world, 1, "wave");
    let narration = narrated(&mut world, &result);

    assert_eq!(result.outcome, Outcome::Applied);
    assert!(narration.is_none());
    assert_eq!(result.readout.as_str(), "Seat 1: wave done.");
    assert_eq!(
        failures_of(&world.events()),
        [(PortName::Narrator, FailureCause::Unavailable)]
    );
}

#[test]
fn narration_that_is_empty_or_over_length_is_text_invalid() {
    let mut world = world_of(&["wave"], 1, allows(ScriptedRulesOwner::default(), "wave"));
    world.narrator = Box::new(FixedNarrator("   ".to_owned()));
    let first = ran(&mut world, 1, "wave");
    let blank = narrated(&mut world, &first);
    world.narrator = Box::new(FixedNarrator("far too long".to_owned()));
    world.limits = PipelineLimits {
        maximum_narration_chars: 5,
        ..PipelineLimits::default()
    };
    let second = ran(&mut world, 1, "wave");
    let long = narrated(&mut world, &second);

    assert!(blank.is_none() && long.is_none());
    assert_eq!(
        failures_of(&world.events()),
        [
            (PortName::Narrator, FailureCause::TextInvalid),
            (PortName::Narrator, FailureCause::TextInvalid)
        ]
    );
}

#[test]
fn narration_within_the_late_bound_is_appended_and_after_it_is_skipped() {
    let mut world = world_of(&["wave"], 1, allows(ScriptedRulesOwner::default(), "wave"));
    world.narrator = Box::new(SlowNarrator {
        clock: Arc::clone(&world.clock),
        delay_ms: 7_000,
        line: "A little late.".to_owned(),
    });
    let first = ran(&mut world, 1, "wave");
    let in_time = narrated(&mut world, &first);
    world.narrator = Box::new(SlowNarrator {
        clock: Arc::clone(&world.clock),
        delay_ms: 9_000,
        line: "Far too late.".to_owned(),
    });
    let second = ran(&mut world, 1, "wave");
    let too_late = narrated(&mut world, &second);

    assert_eq!(in_time.as_deref(), Some("A little late."));
    assert!(too_late.is_none());
    assert_eq!(second.readout.as_str(), "Seat 1: wave done.");
    assert_eq!(
        failures_of(&world.events()),
        [(PortName::Narrator, FailureCause::TimedOut)]
    );
}

#[test]
fn the_pipeline_never_calls_the_narrator_it_only_stages_the_job() {
    let mut world = world_of(&["wave"], 1, allows(ScriptedRulesOwner::default(), "wave"));
    world.narrator = Box::new(FailingNarrator {
        cause: FailureCause::Unavailable,
    });

    let result = ran(&mut world, 1, "wave");

    let job = result
        .narration_job
        .as_ref()
        .expect("a narration job is staged");
    assert_eq!(job.committed.steps, [action("wave")]);
    assert_eq!(
        failures_of(&world.events()),
        [],
        "nothing was asked of the narrator"
    );
}

#[test]
fn a_rules_owner_that_fails_declines_the_input_and_commits_nothing() {
    let mut world = world_of(
        &["wave"],
        1,
        ScriptedRulesOwner::failing(FailureCause::TimedOut),
    );

    let result = ran(&mut world, 1, "wave");

    assert_eq!(result.record.declined, Some(DeclineCode::NotSupportedYet));
    assert_eq!(
        result.record.state_hash_before,
        result.record.state_hash_after
    );
    assert_eq!(
        failures_of(&world.events()),
        [(PortName::RulesOwner, FailureCause::TimedOut)]
    );
}

#[test]
fn a_die_off_the_die_is_an_invalid_answer_never_replaced_by_a_roll() {
    let rules = ScriptedRulesOwner::default().with_action(
        action("wave"),
        Preparation::Ready { draws: vec![d20()] },
        Resolution::applied(),
    );
    let mut world = world_of(&["wave"], 1, rules);
    world.dice = crate::golden::CountingDice::new([25]);

    let result = ran(&mut world, 1, "wave");

    assert_eq!(result.record.declined, Some(DeclineCode::NotSupportedYet));
    assert_eq!(world.dice.drawn(), 1);
    assert_eq!(
        failures_of(&world.events()),
        [(PortName::Dice, FailureCause::InvalidAnswer)]
    );
    assert_eq!(
        result.record.state_hash_before,
        result.record.state_hash_after
    );
}

#[test]
fn a_resolution_that_asks_for_more_dice_is_given_them_and_asked_again() {
    let rules = ScriptedRulesOwner::default()
        .with_action(
            action("wave"),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        )
        .with_resolutions(action("wave"), vec![Resolution::NeedsDraws(vec![d20()])]);
    let mut world = world_of(&["wave"], 1, rules);
    world.dice = crate::golden::CountingDice::new([4, 17]);

    let result = ran(&mut world, 1, "wave");

    assert_eq!(result.outcome, Outcome::Applied);
    assert_eq!(world.dice.drawn(), 2);
    assert_eq!(result.record.draws.len(), 2);
    assert!(world.rules.calls().contains(&RulesCall::Resolve {
        action: action("wave"),
        draws: 2
    }));
}

#[test]
fn a_resolution_that_keeps_asking_for_dice_stops_at_the_round_limit() {
    let rules = ScriptedRulesOwner::default().with_action(
        action("wave"),
        Preparation::Ready { draws: Vec::new() },
        Resolution::NeedsDraws(vec![d20()]),
    );
    let mut world = world_of(&["wave"], 1, rules);
    world.dice = crate::golden::CountingDice::new([1, 2, 3, 4, 5]);
    world.limits = PipelineLimits {
        maximum_draw_rounds: 2,
        ..PipelineLimits::default()
    };

    let result = ran(&mut world, 1, "wave");

    assert_eq!(result.record.declined, Some(DeclineCode::NotSupportedYet));
    assert_eq!(world.dice.drawn(), 2);
    assert_eq!(
        failures_of(&world.events()),
        [(PortName::RulesOwner, FailureCause::InvalidAnswer)]
    );
}
