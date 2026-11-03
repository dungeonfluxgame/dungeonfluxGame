//! An operation id means something only together with its sender (#1735): the result books and the
//! table's memory of applied operations are keyed by the sender and the membership of that sender,
//! so a seat that reuses another seat's id (or a seat's earlier occupant's) never reads that seat's
//! outcome and is never blocked by it, while a duplicate from the same sender still gets the
//! original answer without a port asked or a die drawn.

use crate::effects::TableEffect;
use crate::envelope::{InputEnvelope, InputKind, PlayerInput, Sender, ViewStamp};
use crate::golden::{CountingDice, TABLE, World};
use crate::identity::TableScope;
use crate::outcome::{DeclineCode, Outcome};
use crate::pipeline::{
    BookLoadError, PipelineLimits, PipelineLoadError, PipelineRecord, PipelineTable, ResultBook,
    ResultKey, ResultRecord, TurnResult,
};
use crate::pipeline_fixtures_tests::{action, d20, seat, world_of};
use crate::ports::{Clock, Preparation, Resolution};
use crate::testkit::{ScriptedRulesOwner, correlation};

/// Every seat may wave (no die) or roll (one d20).
fn rules() -> ScriptedRulesOwner {
    let choices = vec![action("wave"), action("roll")];
    ScriptedRulesOwner::default()
        .with_choices(seat(1), choices.clone())
        .with_choices(seat(2), choices.clone())
        .with_choices(seat(3), choices)
        .with_action(
            action("wave"),
            Preparation::Ready { draws: Vec::new() },
            Resolution::applied(),
        )
        .with_action(
            action("roll"),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        )
}

fn world(dice: &[u16]) -> World {
    let mut world = world_of(&["wave", "roll"], 3, rules());
    world.dice = CountingDice::new(dice.iter().copied());
    world
}

/// A tap by `seat_number` under the operation id of input number `operation`.
fn tap(operation: u64, seat_number: u64, name: &str) -> InputEnvelope {
    let probe = correlation(operation);
    InputEnvelope::from_player(
        probe.operation_id,
        seat(seat_number),
        ViewStamp::new(probe.revision),
        PlayerInput::Tap {
            action: action(name),
        },
    )
}

fn send(world: &mut World, envelope: &InputEnvelope, operation: u64) -> TurnResult {
    world
        .run_with(envelope, correlation(operation))
        .expect("the input runs")
}

fn key_of(world: &World, envelope: &InputEnvelope) -> ResultKey {
    ResultKey::new(
        envelope.sender(),
        world.table.facts().membership_of(envelope.sender()),
        envelope.operation_id(),
    )
}

/// Changes the table's facts the way the host's other owners would, keeping everything the pipeline
/// holds (the result books included).
fn change_table(world: &mut World, step: u64, effects: &[TableEffect]) {
    let mut facts = world.table.facts().clone();
    facts
        .apply(
            &TableScope::of_table(TABLE),
            correlation(4_000_000 + step),
            effects,
        )
        .expect("the change applies");
    let now_ms = world.clock.now_ms();
    let record = world.table.to_record(now_ms);
    world.table =
        PipelineTable::from_record(TABLE, facts, record, now_ms, &PipelineLimits::default())
            .expect("the table loads again");
}

#[test]
fn two_seats_using_the_same_operation_id_each_get_their_own_outcome() {
    let mut world = world(&[14]);
    let waved = tap(1, 1, "wave");
    let rolled = tap(1, 2, "roll");

    let first = send(&mut world, &waved, 1);
    assert_eq!(world.dice.drawn(), 0);
    let second = send(&mut world, &rolled, 1);

    assert!(!first.record.replayed);
    assert!(
        !second.record.replayed,
        "seat 2's input ran: it is not seat 1's duplicate"
    );
    assert_eq!(second.outcome, Outcome::Applied);
    assert_ne!(
        second.readout, first.readout,
        "seat 2 is not told what seat 1 was told"
    );
    assert_eq!(world.dice.drawn(), 1, "seat 2's own roll drew its die");
    assert_eq!(second.draws().len(), 1);
    let first_stored = world
        .table
        .results()
        .find_stored(key_of(&world, &waved))
        .expect("seat 1's result is kept");
    let second_stored = world
        .table
        .results()
        .find_stored(key_of(&world, &rolled))
        .expect("seat 2's result is kept beside it");
    assert_eq!(first_stored.kind(), Some(InputKind::Tap));
    assert_eq!(second_stored.kind(), Some(InputKind::Tap));
    assert_eq!(world.table.results().len(), 2);
}

#[test]
fn no_seat_is_given_the_stored_outcome_of_another_seat() {
    let mut world = world(&[14, 3]);
    let first = send(&mut world, &tap(1, 1, "wave"), 1);
    let first_readout = first.readout.clone();

    let other = send(&mut world, &tap(1, 2, "roll"), 1);
    let third = send(&mut world, &tap(1, 3, "roll"), 1);

    for result in [&other, &third] {
        assert!(!result.record.replayed);
        assert_ne!(result.readout, first_readout);
    }
    assert_eq!(world.dice.drawn(), 2, "each of the two rolls drew once");
}

#[test]
fn a_duplicate_from_the_same_sender_gets_the_original_without_drawing_again() {
    let mut world = world(&[14, 3]);
    let rolled = tap(1, 2, "roll");
    let first = send(&mut world, &rolled, 1);
    assert_eq!(world.dice.drawn(), 1);
    let calls = world.rules.calls().len();
    // Another seat uses the same id in between.
    send(&mut world, &tap(1, 1, "wave"), 1);

    let again = send(&mut world, &rolled, 1);

    assert!(again.record.replayed);
    assert_eq!(again.outcome, first.outcome);
    assert_eq!(again.readout, first.readout, "the original answer");
    assert_eq!(world.dice.drawn(), 1, "no die is drawn again");
    assert_eq!(
        world.rules.calls().len(),
        calls + 2,
        "only seat 1's own wave asked the rules owner; the duplicate asked nothing"
    );
}

#[test]
fn a_seat_that_never_sat_is_declined_and_nothing_is_kept_for_it() {
    let mut world = world(&[]);
    let refused = tap(1, 9, "wave");
    assert_eq!(world.table.facts().membership_of(refused.sender()), 0);
    let first = send(&mut world, &refused, 1);
    assert!(matches!(first.outcome, Outcome::Declined(_)));
    assert_eq!(first.record.declined, Some(DeclineCode::NotViable));
    assert!(world.table.results().is_empty());
    assert!(world.table.ends().is_empty());

    let waved = send(&mut world, &tap(1, 1, "wave"), 1);

    assert!(
        !waved.record.replayed,
        "seat 1 does not meet the decline of an unseated sender that used its id"
    );
    assert_eq!(waved.outcome, Outcome::Applied);
    let again = send(&mut world, &refused, 1);
    assert!(
        !again.record.replayed,
        "nothing was kept for the sender, so nothing is replayed"
    );
    assert_eq!(again.readout, first.readout);
    assert_eq!(again.record.declined, Some(DeclineCode::NotViable));
    let zero = ResultKey::new(refused.sender(), 0, refused.operation_id());
    assert!(world.table.ends().find_stored(zero).is_none());
    assert!(world.table.results().find_stored(zero).is_none());
}

#[test]
fn two_periods_a_seat_spent_away_never_share_a_result_key() {
    let mut world = world(&[]);
    let probe = tap(1, 2, "wave");

    change_table(&mut world, 1, &[TableEffect::SeatLeft { seat: seat(2) }]);
    let first_key = key_of(&world, &probe);
    let first = send(&mut world, &probe, 1);
    assert!(matches!(first.outcome, Outcome::Declined(_)));
    assert!(
        world.table.ends().find_stored(first_key).is_some(),
        "the first period away keeps its decline under its own membership"
    );

    change_table(&mut world, 2, &[TableEffect::SeatJoined { seat: seat(2) }]);
    change_table(&mut world, 3, &[TableEffect::SeatLeft { seat: seat(2) }]);
    let second_key = key_of(&world, &probe);
    assert_ne!(first_key, second_key);
    assert_ne!(
        world.table.facts().membership_of(probe.sender()),
        0,
        "a seat that sat holds a membership of its own while away"
    );
    assert!(
        world.table.ends().find_stored(second_key).is_none(),
        "the second period away does not read the first one's decline"
    );
    let second = send(&mut world, &probe, 1);
    assert!(
        !second.record.replayed,
        "the second period's input ran; it is not the first period's duplicate"
    );
    assert!(world.table.ends().find_stored(second_key).is_some());
}

#[test]
fn a_rejoined_seat_does_not_see_the_previous_occupants_stored_outcome() {
    let mut world = world(&[14]);
    let waved = tap(1, 2, "wave");
    let first = send(&mut world, &waved, 1);
    let before = world.table.facts().membership_of(Sender::Seat(seat(2)));
    assert_ne!(before, 0);

    change_table(&mut world, 1, &[TableEffect::SeatLeft { seat: seat(2) }]);
    let away = world.table.facts().membership_of(Sender::Seat(seat(2)));
    assert_ne!(away, 0, "the time away is a period of its own");
    assert_ne!(away, before, "the time away is not the seated period");
    change_table(&mut world, 2, &[TableEffect::SeatJoined { seat: seat(2) }]);
    let after = world.table.facts().membership_of(Sender::Seat(seat(2)));
    assert!(after > before, "a join is a new membership");
    assert!(after > away, "a join is a new membership");

    // The new occupant sends the id the old one used, for a different action.
    let rolled = tap(1, 2, "roll");
    let second = send(&mut world, &rolled, 1);

    assert!(
        !second.record.replayed,
        "the new occupant's input runs; it is not the old one's duplicate"
    );
    assert_ne!(second.readout, first.readout);
    assert_eq!(world.dice.drawn(), 1);
    assert!(
        world
            .table
            .facts()
            .has_applied(Sender::Seat(seat(2)), &rolled.operation_id()),
        "the new membership's own operation is remembered"
    );
    let own =
        |membership: u64| ResultKey::new(Sender::Seat(seat(2)), membership, waved.operation_id());
    assert!(
        world.table.results().find_stored(own(before)).is_some(),
        "the earlier membership's result is still its own, under its own key"
    );
    assert_ne!(
        world.table.results().find_stored(own(after)),
        world.table.results().find_stored(own(before)),
        "the rejoined seat's key holds its own result, not what was stored before the leave"
    );
}

#[test]
fn another_seats_operation_id_no_longer_blocks_an_input_the_results_have_forgotten() {
    let mut world = world(&[]);
    world.limits = PipelineLimits {
        maximum_remembered_results: 1,
        ..PipelineLimits::default()
    };
    world.table = PipelineTable::with_limits(TABLE, world.table.facts().clone(), &world.limits);
    let first = tap(1, 1, "wave");
    send(&mut world, &first, 1);
    send(&mut world, &tap(2, 1, "wave"), 2);
    assert!(
        world.table.results().find(key_of(&world, &first)).is_none(),
        "the result book forgot seat 1's first operation"
    );
    let facts = world.table.facts();
    assert!(facts.has_applied(Sender::Seat(seat(1)), &first.operation_id()));
    assert!(
        !facts.has_applied(Sender::Seat(seat(2)), &first.operation_id()),
        "the applied memory is seat 1's alone"
    );
    assert!(!facts.has_applied(Sender::Engine, &first.operation_id()));

    let other = send(&mut world, &tap(1, 2, "wave"), 1);
    assert_eq!(
        other.outcome,
        Outcome::Applied,
        "seat 2's input runs; seat 1's id does not block it"
    );
    assert!(!other.record.replayed);

    let again = send(&mut world, &first, 1);
    assert!(
        again.record.replayed,
        "seat 1's own forgotten duplicate is still not run again"
    );
}

#[test]
fn the_books_survive_a_save_and_a_load_with_their_senders() {
    let mut world = world(&[14]);
    let waved = tap(1, 1, "wave");
    let rolled = tap(1, 2, "roll");
    let first = send(&mut world, &waved, 1);
    let second = send(&mut world, &rolled, 1);
    let now_ms = world.clock.now_ms();
    let saved = serde_json::to_string(&world.table.to_record(now_ms)).expect("the table writes");

    let record: PipelineRecord = serde_json::from_str(&saved).expect("reads");
    let loaded = PipelineTable::from_record(
        TABLE,
        world.table.facts().clone(),
        record,
        now_ms,
        &PipelineLimits::default(),
    )
    .expect("the record loads");

    assert_eq!(loaded.results(), world.table.results());
    assert_eq!(loaded.results().digest(), world.table.results().digest());
    assert_eq!(
        loaded
            .results()
            .find_stored(key_of(&world, &rolled))
            .and_then(|stored| stored.kind()),
        Some(InputKind::Tap)
    );
    world.table = loaded;
    let drawn = world.dice.drawn();
    let again_first = send(&mut world, &waved, 1);
    let again_second = send(&mut world, &rolled, 1);
    assert!(again_first.record.replayed && again_second.record.replayed);
    assert_eq!(again_first.readout, first.readout);
    assert_eq!(again_second.readout, second.readout);
    assert_eq!(world.dice.drawn(), drawn, "a restart never draws again");
}

#[test]
fn a_result_record_saved_before_senders_were_kept_is_refused_with_a_typed_error() {
    let mut world = world(&[]);
    send(&mut world, &tap(1, 1, "wave"), 1);
    let now_ms = world.clock.now_ms();
    let saved = serde_json::to_value(world.table.to_record(now_ms)).expect("the table writes");
    let results = saved["results"].clone();

    // Version 1: the operation id alone, no origin.
    let mut old = results.clone();
    old["schema_version"] = 1.into();
    for entry in old["entries"].as_array_mut().expect("entries") {
        entry.as_object_mut().expect("an entry").remove("origin");
    }
    let old: ResultRecord = serde_json::from_value(old).expect("an old record still reads");
    assert_eq!(
        ResultBook::from_record(old),
        Err(BookLoadError::UnsupportedVersion {
            found: 1,
            expected: 2
        })
    );

    // A current record whose result does not say who sent it is refused, never guessed.
    let mut nameless = results;
    for entry in nameless["entries"].as_array_mut().expect("entries") {
        entry.as_object_mut().expect("an entry").remove("origin");
    }
    let nameless: ResultRecord = serde_json::from_value(nameless).expect("reads");
    assert_eq!(
        ResultBook::from_record(nameless),
        Err(BookLoadError::SenderMissing)
    );
}

#[test]
fn a_pipeline_record_of_the_older_version_is_refused_whole() {
    let mut world = world(&[]);
    send(&mut world, &tap(1, 1, "wave"), 1);
    let now_ms = world.clock.now_ms();
    let mut saved = serde_json::to_value(world.table.to_record(now_ms)).expect("the table writes");
    saved["schema_version"] = 2.into();
    let record: PipelineRecord = serde_json::from_value(saved).expect("reads");

    let loaded = PipelineTable::from_record(
        TABLE,
        world.table.facts().clone(),
        record,
        now_ms,
        &PipelineLimits::default(),
    );

    assert!(matches!(
        loaded,
        Err(PipelineLoadError::UnsupportedVersion {
            found: 2,
            expected: 3
        })
    ));
}
