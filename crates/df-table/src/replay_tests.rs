//! Recording a script against a scripted world and replaying it: the same records and state hash
//! on a fresh world, the same result twice, and a refusal for every way a recording can be
//! tampered with. The format and the recorder are in `replay_session_tests`.

use crate::envelope::{EngineControl, EngineInput, InputOrigin, PendingId, PlayerInput};
use crate::envelope::{TimerName, UtteranceId};
use crate::golden::{
    CountingDice, Ending, Expect, GoldenInput, GoldenStep, StateChange, World, WorldError,
};
use crate::identity::{SeatId, TurnSeq};
use crate::outcome::{DeclineCode, OutcomeKind};
use crate::pipeline_fixtures_tests::{action, allows, d20, refuses, seat, window_of, world_of};
use crate::ports::{
    Awaiting, AwaitingKind, Interpretation, NonActionKind, Preparation, Resolution,
};
use crate::private_data::PlayerText;
use crate::replay::{
    LineTextError, RecordedSession, Replay, ReplayError, SessionError, TurnField, golden_script,
    record,
};
use crate::testkit::{ScriptedInterpreter, ScriptedRulesOwner};

pub(crate) const LOOK_LINE: &str = "I look around the room.";

pub(crate) fn scripted_world() -> Result<World, WorldError> {
    let strike = ScriptedRulesOwner::default()
        .with_action(
            action("strike"),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        )
        .with_resolutions(
            action("strike"),
            vec![Resolution::Await(Awaiting {
                kind: AwaitingKind::Choice,
                ..window_of(1, &["use_inspiration", "keep"])
            })],
        )
        .with_resumptions(action("strike"), vec![Resolution::applied()]);
    let rules = refuses(allows(strike, "look"), "dance", DeclineCode::NotViable)
        .with_choices(seat(1), vec![action("strike"), action("look")])
        .with_choices(seat(2), vec![action("look"), action("dance")]);
    let mut world = world_of(&["strike", "look", "dance"], 2, rules);
    world.interpreter = ScriptedInterpreter::default().with_line(
        LOOK_LINE,
        Interpretation::NonAction(NonActionKind::Question),
    );
    world.dice = CountingDice::new([19, 7]);
    Ok(world)
}

fn player(seat_number: u64, input: PlayerInput) -> InputOrigin {
    InputOrigin::Player {
        seat: seat(seat_number),
        input,
    }
}

fn tap(seat_number: u64, name: &str) -> InputOrigin {
    player(
        seat_number,
        PlayerInput::Tap {
            action: action(name),
        },
    )
}

/// A roll that waits for the roller's answer, the answer, a typed line, an engine timer, a decline,
/// an applied action, the safety card and the resume that follows it.
pub(crate) fn session_script() -> Vec<InputOrigin> {
    vec![
        tap(1, "strike"),
        player(
            1,
            PlayerInput::PendingAnswer {
                pending: PendingId::new(1),
                choice: action("keep"),
            },
        ),
        player(
            2,
            PlayerInput::SubmitText {
                utterance: UtteranceId::new(3),
                text: PlayerText::new(LOOK_LINE),
            },
        ),
        InputOrigin::Engine(EngineInput::Timer {
            timer: TimerName::new("idle_nudge").expect("valid timer"),
        }),
        tap(2, "dance"),
        tap(1, "look"),
        player(2, PlayerInput::SafetyCard),
        InputOrigin::Engine(EngineInput::Control(EngineControl::ResumeTable)),
    ]
}

fn recorded() -> crate::replay::Recording {
    let mut world = scripted_world().expect("the world builds");
    record(&mut world, &session_script()).expect("the script records")
}

/// The recording as JSON, edited by `edit`, and read back.
fn tampered(
    session: &RecordedSession,
    edit: impl FnOnce(&mut serde_json::Value),
) -> Result<RecordedSession, SessionError> {
    let mut json: serde_json::Value =
        serde_json::from_str(&session.to_json().expect("writes")).expect("is JSON");
    edit(&mut json);
    RecordedSession::from_json(&json.to_string())
}

#[test]
fn a_recorded_session_replays_to_the_same_records_and_the_same_state_hash() {
    let recording = recorded();
    let session = &recording.session;

    let report = Replay::of(session)
        .with_lines(&recording.lines)
        .run(scripted_world)
        .expect("the session replays");

    assert_eq!(report.turns_replayed, session.turns().len());
    assert_eq!(report.turns_replayed, 8);
    assert!(report.divergences.is_empty());
    assert_eq!(report.final_state_hash, session.final_state_hash());
}

#[test]
fn the_recorded_turns_say_how_each_input_ended_what_was_drawn_and_what_was_committed() {
    let session = recorded().session;
    let turns = session.turns();

    assert_eq!(turns[0].outcome, OutcomeKind::Pending);
    assert_eq!(
        turns[0]
            .draws
            .iter()
            .map(|draw| draw.value())
            .collect::<Vec<_>>(),
        [19]
    );
    assert_eq!(turns[0].pending, Some(PendingId::new(1)));
    assert_eq!(turns[1].outcome, OutcomeKind::Applied);
    assert!(!turns[1].effects.is_empty());
    assert_eq!(turns[4].outcome, OutcomeKind::Declined);
    assert_eq!(turns[4].declined, Some(DeclineCode::NotViable));
    assert!(
        turns
            .windows(2)
            .all(|pair| pair[1].state_hash_before == pair[0].state_hash_settled),
        "each turn starts from the hash the one before it settled at"
    );
    assert_eq!(session.initial_state_hash(), turns[0].state_hash_before);
    assert_ne!(session.initial_state_hash(), session.final_state_hash());
}

#[test]
fn replaying_twice_gives_the_same_report_and_a_second_recording_is_identical() {
    let recording = recorded();

    let first = Replay::of(&recording.session)
        .with_lines(&recording.lines)
        .run(scripted_world)
        .expect("replays");
    let second = Replay::of(&recording.session)
        .with_lines(&recording.lines)
        .run(scripted_world)
        .expect("replays");

    assert_eq!(first, second);
    assert_eq!(recorded().session, recording.session);
}

#[test]
fn a_session_written_as_json_reads_back_equal_and_still_replays_and_holds_no_typed_text() {
    let recording = recorded();

    let json = recording.session.to_json().expect("writes");
    let read = RecordedSession::from_json(&json).expect("reads back");

    assert!(!json.contains(LOOK_LINE) && !json.contains("look around"));
    assert!(json.contains("\"length\":\"medium\"") || json.contains("\"length\":\"short\""));
    assert_eq!(read, recording.session);
    Replay::of(&read)
        .with_lines(&recording.lines)
        .run(scripted_world)
        .expect("the parsed session replays");
}

#[test]
fn a_changed_die_is_refused_as_a_divergence_in_the_draws() {
    let recording = recorded();
    let session = tampered(&recording.session, |json| {
        json["turns"][0]["draws"][0]["value"] = serde_json::json!(2);
    })
    .expect("the chain still holds");

    let error = Replay::of(&session)
        .with_lines(&recording.lines)
        .run(scripted_world)
        .expect_err("a changed die is refused");

    match error {
        ReplayError::Diverged(divergence) => {
            assert_eq!(divergence.turn, 0);
            assert_eq!(divergence.fields, [TurnField::Draws]);
        }
        other => panic!("expected a divergence, got {other:?}"),
    }
}

#[test]
fn a_dropped_effect_or_a_flipped_outcome_is_refused_in_that_turn() {
    let recording = recorded();
    let dropped = tampered(&recording.session, |json| {
        json["turns"][1]["effects"] = serde_json::json!([]);
    })
    .expect("the chain still holds");
    let flipped = tampered(&recording.session, |json| {
        json["turns"][5]["outcome"] = serde_json::json!("declined");
    })
    .expect("the chain still holds");

    for (session, turn, field) in [
        (&dropped, 1, TurnField::Effects),
        (&flipped, 5, TurnField::Outcome),
    ] {
        let error = Replay::of(session)
            .with_lines(&recording.lines)
            .run(scripted_world)
            .expect_err("tampering is refused");
        assert!(
            matches!(&error, ReplayError::Diverged(found)
                if found.turn == turn && found.fields.contains(&field)),
            "{error:?}"
        );
    }
}

#[test]
fn a_turn_that_does_not_start_where_the_one_before_it_settled_is_refused_before_anything_runs() {
    let recording = recorded();

    let refused = tampered(&recording.session, |json| {
        json["turns"][2]["state_hash_before"] = serde_json::json!(12345);
    })
    .expect_err("a broken chain is refused");

    assert!(matches!(
        refused,
        SessionError::ChainBroken {
            turn: 2,
            found: 12345,
            ..
        }
    ));
}

#[test]
fn a_hash_edited_on_both_sides_of_a_link_passes_the_chain_and_fails_the_replay() {
    let recording = recorded();
    let session = tampered(&recording.session, |json| {
        json["turns"][1]["state_hash_settled"] = serde_json::json!(777);
        json["turns"][2]["state_hash_before"] = serde_json::json!(777);
    })
    .expect("the chain is consistent");

    let error = Replay::of(&session)
        .with_lines(&recording.lines)
        .run(scripted_world)
        .expect_err("the replay disagrees");

    assert!(matches!(&error, ReplayError::Diverged(found)
        if found.turn == 1 && found.fields == [TurnField::StateHashSettled]));
}

#[test]
fn swapped_turns_are_refused_by_the_chain() {
    let recording = recorded();

    let refused = tampered(&recording.session, |json| {
        let turns = json["turns"].as_array_mut().expect("turns");
        turns.swap(3, 4);
    })
    .expect_err("swapped turns are refused");

    assert!(matches!(refused, SessionError::ChainBroken { turn: 3, .. }));
}

#[test]
fn a_recording_of_another_schema_version_or_an_unreadable_one_is_refused_with_a_typed_error() {
    let recording = recorded();

    let other_version = tampered(&recording.session, |json| {
        json["schema_version"] = serde_json::json!(2);
    });
    let extra_field = tampered(&recording.session, |json| {
        json["turns"][0]["note"] = serde_json::json!("hand written");
    });

    assert_eq!(
        other_version,
        Err(SessionError::UnsupportedSchema {
            found: 2,
            expected: crate::replay::RECORDING_SCHEMA_VERSION
        })
    );
    assert!(matches!(extra_field, Err(SessionError::Undecodable { .. })));
    assert!(matches!(
        RecordedSession::from_json("not json"),
        Err(SessionError::Undecodable { .. })
    ));
    assert!(matches!(
        RecordedSession::from_json("{}"),
        Err(SessionError::Undecodable { .. })
    ));
}

#[test]
fn diff_lists_every_divergence_where_run_stops_at_the_first() {
    let recording = recorded();
    let session = tampered(&recording.session, |json| {
        json["turns"][0]["draws"][0]["value"] = serde_json::json!(2);
        json["turns"][4]["outcome"] = serde_json::json!("applied");
    })
    .expect("the chain still holds");

    let report = Replay::of(&session)
        .with_lines(&recording.lines)
        .diff(scripted_world)
        .expect("a diff always completes");

    assert_eq!(report.turns_replayed, 8);
    let turns: Vec<usize> = report.divergences.iter().map(|found| found.turn).collect();
    assert_eq!(turns, [0, 4]);
    assert_eq!(report.divergences[0].fields, [TurnField::Draws]);
    assert!(report.divergences[1].fields.contains(&TurnField::Outcome));
}

#[test]
fn until_stops_after_the_last_turn_numbered_at_most_the_one_given() {
    let recording = recorded();
    let session = &recording.session;

    let report = Replay::of(session)
        .with_lines(&recording.lines)
        .until(TurnSeq::new(2))
        .run(scripted_world)
        .expect("replays");

    assert_eq!(report.turns_replayed, 2);
    assert_eq!(
        report.final_state_hash,
        session.turns()[1].state_hash_settled
    );
    let none = Replay::of(session)
        .with_lines(&recording.lines)
        .until(TurnSeq::new(0))
        .run(scripted_world)
        .expect("replays");
    assert_eq!(none.turns_replayed, 0);
    assert_eq!(none.final_state_hash, session.initial_state_hash());
}

#[test]
fn a_world_that_starts_at_another_state_is_refused_before_any_input_runs() {
    let recording = recorded();
    let elsewhere = || {
        let mut world = scripted_world()?;
        world.table = world.table.clone().hosted_by(SeatId::new(2));
        Ok(world)
    };

    let error = Replay::of(&recording.session)
        .with_lines(&recording.lines)
        .run(elsewhere)
        .expect_err("a different start is refused");

    assert!(matches!(
        error,
        ReplayError::StartsElsewhere { expected, found } if expected != found
    ));
}

#[test]
fn a_world_that_runs_another_table_is_refused_before_anything_else() {
    let recording = recorded();
    let other_table = || {
        let mut world = scripted_world()?;
        world.table = crate::pipeline::PipelineTable::new(
            crate::identity::TableId::new(2),
            world.table.facts().clone(),
        );
        Ok(world)
    };

    let error = Replay::of(&recording.session)
        .with_lines(&recording.lines)
        .run(other_table)
        .expect_err("another table is refused");

    assert!(matches!(
        error,
        ReplayError::OtherTable { recorded, world }
            if recorded == crate::identity::TableId::new(1)
                && world == crate::identity::TableId::new(2)
    ));
}

#[test]
fn a_typed_line_needs_its_words_and_words_of_another_length_are_refused() {
    let recording = recorded();
    let long = crate::replay::LineTexts::new().with(
        recording.session.turns()[2].correlation.operation_id,
        &"x".repeat(500),
    );

    let missing = Replay::of(&recording.session)
        .run(scripted_world)
        .expect_err("no words");
    let changed = Replay::of(&recording.session)
        .with_lines(&long)
        .run(scripted_world)
        .expect_err("other words");

    assert!(matches!(
        missing,
        ReplayError::Line {
            turn: 2,
            error: LineTextError::Missing
        }
    ));
    assert!(matches!(
        changed,
        ReplayError::Line {
            turn: 2,
            error: LineTextError::LengthChanged { .. }
        }
    ));
}

#[test]
fn an_empty_script_records_a_session_with_no_turns_that_replays_to_its_starting_hash() {
    let mut world = scripted_world().expect("builds");
    let recording = record(&mut world, &[]).expect("records");

    let report = Replay::of(&recording.session)
        .run(scripted_world)
        .expect("replays");

    assert!(recording.session.turns().is_empty());
    assert_eq!(report.turns_replayed, 0);
    assert_eq!(
        report.final_state_hash,
        recording.session.initial_state_hash()
    );
    assert_eq!(
        recording.session.final_state_hash(),
        recording.session.initial_state_hash()
    );
}

fn golden_step(name: &'static str, input: GoldenInput) -> GoldenStep {
    GoldenStep {
        name,
        input,
        expect: Expect {
            ending: Ending::Applied,
            state: StateChange::Changed,
            stages: &[],
            released: &[],
            dice: &[],
            readout: "",
            options: &[],
            machine: "",
            pendings: &[],
            views: &[],
        },
    }
}

#[test]
fn the_inputs_of_a_golden_script_record_and_replay_like_any_other_script() {
    let steps = [
        golden_step(
            "strike",
            GoldenInput::Tap {
                seat: 1,
                action: "strike",
            },
        ),
        golden_step(
            "keep",
            GoldenInput::Answer {
                seat: 1,
                pending: 1,
                choice: "keep",
            },
        ),
        golden_step(
            "look",
            GoldenInput::Text {
                seat: 2,
                line: LOOK_LINE,
            },
        ),
        golden_step("nudge", GoldenInput::Timer("idle_nudge")),
        golden_step(
            "dance",
            GoldenInput::Tap {
                seat: 2,
                action: "dance",
            },
        ),
        golden_step("proposal", GoldenInput::MachineProposal),
    ];
    let script = golden_script(&steps).expect("the names are valid");
    let mut world = scripted_world().expect("builds");

    let recording = record(&mut world, &script).expect("records");
    let report = Replay::of(&recording.session)
        .with_lines(&recording.lines)
        .run(scripted_world)
        .expect("replays");

    assert_eq!(recording.session.turns().len(), steps.len());
    assert_eq!(
        report.final_state_hash,
        recording.session.final_state_hash()
    );
    let bad = golden_script(&[golden_step(
        "bad",
        GoldenInput::Tap {
            seat: 1,
            action: "Not A Name",
        },
    )]);
    assert!(matches!(bad, Err(ReplayError::BadName(_))));
}
