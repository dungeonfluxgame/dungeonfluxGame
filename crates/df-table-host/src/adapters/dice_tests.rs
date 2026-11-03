use std::collections::VecDeque;
use std::num::NonZeroU16;

use df_table::event::FailureCause;
use df_table::ports::{Dice, DrawRequest, PortName};
use df_table::testkit::correlation;

use super::dice::{
    DrawRecord, EntropySource, RecordedDice, ReplayDice, SeededEntropy, SystemEntropy,
};

fn die(sides: u16) -> DrawRequest {
    DrawRequest {
        sides: NonZeroU16::new(sides).expect("a die has sides"),
    }
}

/// A source that gives the numbers it was given, then repeats the last one.
#[derive(Debug)]
struct Scripted {
    numbers: VecDeque<u64>,
    last: u64,
}

impl Scripted {
    fn giving(numbers: impl IntoIterator<Item = u64>) -> Self {
        Self {
            numbers: numbers.into_iter().collect(),
            last: 0,
        }
    }
}

impl EntropySource for Scripted {
    fn next_u64(&mut self) -> u64 {
        if let Some(next) = self.numbers.pop_front() {
            self.last = next;
        }
        self.last
    }
}

fn values(dice: &RecordedDice, sides: u16, count: usize) -> Vec<u16> {
    (0..count)
        .map(|index| {
            dice.draw(correlation(index as u64 + 1), die(sides))
                .expect("a draw")
                .value()
        })
        .collect()
}

#[test]
fn the_seeded_source_gives_the_splitmix64_sequence_of_its_seed() {
    let mut source = SeededEntropy::new(0);

    assert_eq!(source.next_u64(), 0xe220_a839_7b1d_cdaf);
    assert_eq!(source.next_u64(), 0x6e78_9e6a_a1b9_65f4);
}

#[test]
fn the_same_seed_gives_the_same_dice_and_another_seed_gives_others() {
    let first = values(&RecordedDice::seeded(20_261_102), 20, 64);
    let again = values(&RecordedDice::seeded(20_261_102), 20, 64);
    let other = values(&RecordedDice::seeded(20_261_103), 20, 64);

    assert_eq!(first, again);
    assert_ne!(first, other);
}

#[test]
fn every_face_is_on_the_die_whatever_its_size() {
    for sides in [1, 2, 4, 6, 8, 10, 12, 20, 100, u16::MAX] {
        let rolled = values(&RecordedDice::seeded(7), sides, 200);

        assert!(
            rolled.iter().all(|value| (1..=sides).contains(value)),
            "a d{sides} rolled off the die: {rolled:?}"
        );
    }
}

#[test]
fn a_d20_reaches_both_its_lowest_and_highest_face_over_many_draws() {
    let rolled = values(&RecordedDice::seeded(1), 20, 2_000);

    assert!(rolled.contains(&1) && rolled.contains(&20));
}

#[test]
fn a_number_from_the_uneven_last_block_is_thrown_away_and_another_is_taken() {
    let accepted_below = (u64::MAX / 20) * 20;
    let dice = RecordedDice::new(Box::new(Scripted::giving([
        u64::MAX,
        accepted_below,
        accepted_below - 1,
    ])));

    let drawn = dice.draw(correlation(1), die(20)).expect("a draw");

    assert_eq!(u64::from(drawn.value()), (accepted_below - 1) % 20 + 1);
}

#[test]
fn a_source_that_only_gives_rejected_numbers_is_unavailable_and_records_nothing() {
    let dice = RecordedDice::new(Box::new(Scripted::giving([u64::MAX])));

    let error = dice.draw(correlation(1), die(20)).expect_err("no face");

    assert_eq!(
        (error.port, error.cause),
        (PortName::Dice, FailureCause::Unavailable)
    );
    assert!(dice.records().is_empty());
}

#[test]
fn every_draw_is_recorded_in_order_with_the_input_it_served() {
    let dice = RecordedDice::seeded(3);
    let first = dice.draw(correlation(1), die(20)).expect("a draw");
    let second = dice.draw(correlation(1), die(6)).expect("a draw");
    let third = dice.draw(correlation(2), die(20)).expect("a draw");

    let records = dice.records();

    assert_eq!(
        records,
        [
            DrawRecord {
                sequence: 1,
                correlation: correlation(1),
                draw: first
            },
            DrawRecord {
                sequence: 2,
                correlation: correlation(1),
                draw: second
            },
            DrawRecord {
                sequence: 3,
                correlation: correlation(2),
                draw: third
            },
        ]
    );
    assert_eq!(
        dice.records_of(correlation(1).operation_id),
        records[..2].to_vec()
    );
}

#[test]
fn a_full_record_refuses_the_next_draw_instead_of_forgetting_one() {
    let dice = RecordedDice::seeded(3).recording_at_most(2);
    dice.draw(correlation(1), die(20)).expect("a draw");
    dice.draw(correlation(1), die(20)).expect("a draw");

    let error = dice.draw(correlation(1), die(20)).expect_err("full");

    assert_eq!(error.cause, FailureCause::Unavailable);
    assert_eq!(dice.records().len(), 2);
}

#[test]
fn replayed_dice_hand_back_the_recorded_draws_in_order_and_never_roll() {
    let recorded = RecordedDice::seeded(11);
    let rolled: Vec<_> = [(1, 20), (1, 6), (2, 20)]
        .into_iter()
        .map(|(turn, sides)| {
            recorded
                .draw(correlation(turn), die(sides))
                .expect("a draw")
        })
        .collect();
    let replay = ReplayDice::new(recorded.records());

    let replayed: Vec<_> = [(1, 20), (1, 6), (2, 20)]
        .into_iter()
        .map(|(turn, sides)| replay.draw(correlation(turn), die(sides)).expect("a draw"))
        .collect();

    assert_eq!(replayed, rolled);
    assert_eq!(replay.remaining(), 0);
    let exhausted = replay.draw(correlation(2), die(20)).expect_err("none left");
    assert_eq!(exhausted.cause, FailureCause::Unavailable);
}

#[test]
fn a_replayed_question_that_does_not_match_the_record_is_invalid_and_keeps_the_record() {
    let recorded = RecordedDice::seeded(11);
    recorded.draw(correlation(1), die(20)).expect("a draw");
    let replay = ReplayDice::new(recorded.records());

    let wrong_die = replay.draw(correlation(1), die(6)).expect_err("wrong die");
    let wrong_input = replay
        .draw(correlation(2), die(20))
        .expect_err("wrong input");

    assert_eq!(wrong_die.cause, FailureCause::InvalidAnswer);
    assert_eq!(wrong_input.cause, FailureCause::InvalidAnswer);
    assert_eq!(replay.remaining(), 1);
}

#[test]
fn the_system_source_keeps_its_faces_on_the_die_and_differs_between_instances() {
    let rolled = values(&RecordedDice::from_system_entropy(), 20, 500);
    let mut first = SystemEntropy::new();
    let mut second = SystemEntropy::new();

    assert!(rolled.iter().all(|value| (1..=20).contains(value)));
    assert_ne!(first.next_u64(), second.next_u64());
    assert_ne!(first.next_u64(), first.next_u64());
}

/// Draws three dice for two turns, the way a batch of two inputs does, then gives them back the
/// way a refused batch does (newest turn first) and draws again.
fn refused_batch_meets_the_same_dice(dice: &dyn Dice) {
    let (first, second) = (correlation(1), correlation(2));
    let turn_one: Vec<_> = (0..2)
        .map(|_| dice.draw(first, die(20)).expect("a draw"))
        .collect();
    let turn_two: Vec<_> = (0..1)
        .map(|_| dice.draw(second, die(20)).expect("a draw"))
        .collect();

    dice.give_back(second, &turn_two);
    dice.give_back(first, &turn_one);

    let again_one: Vec<_> = (0..2)
        .map(|_| dice.draw(first, die(20)).expect("a draw"))
        .collect();
    let again_two: Vec<_> = (0..1)
        .map(|_| dice.draw(second, die(20)).expect("a draw"))
        .collect();
    assert_eq!(
        again_one, turn_one,
        "the retried first input meets its own dice"
    );
    assert_eq!(again_two, turn_two, "and the second meets its own");
}

#[test]
fn recorded_dice_serve_what_a_refused_batch_gave_back_before_they_roll_again() {
    let dice = RecordedDice::seeded(11);

    refused_batch_meets_the_same_dice(&dice);

    let records = dice.records();
    assert_eq!(
        records.len(),
        3,
        "the refused batch's records went with its draws and the retry recorded its own"
    );
}

#[test]
fn replay_dice_serve_what_a_refused_batch_gave_back_in_order() {
    let source = RecordedDice::seeded(5);
    let (first, second) = (correlation(1), correlation(2));
    for correlation in [first, first, second] {
        source.draw(correlation, die(20)).expect("a draw");
    }
    let dice = ReplayDice::new(source.records());

    refused_batch_meets_the_same_dice(&dice);

    assert_eq!(dice.remaining(), 0);
}
