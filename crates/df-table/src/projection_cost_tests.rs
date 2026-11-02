//! Projection cost is proved by counting what a build examines, never by timing it (O.9 C5): for a
//! table of a few hundred elements, a few thousand facts and seven audiences, the work grows in
//! step with the table and not with its square, and the story's length does not matter.

use crate::effects::TableEffect;
use crate::identity::FactId;
use crate::ports::Audience;
use crate::private_data::Secret;
use crate::projection::bounds::PHONE_NARRATION_LINES;
use crate::projection::{Attachments, ProjectionWork, project};
use crate::projection_tests::{public_line, text};
use crate::table_state::{CastEntry, CastKind, Controller, TableState, Vitals};
use crate::table_state_tests::{apply, display_name, element, hero, place, scene, seat};

const SEATS: u64 = 6;

fn audiences() -> Vec<Audience> {
    let mut all = vec![Audience::Public];
    all.extend((1..=SEATS).map(|number| Audience::Seat(seat(number))));
    all
}

/// Six seats with a hero each and `creatures` creatures on the stage in all.
fn table_of(creatures: usize) -> TableState {
    let mut table =
        TableState::seated(8, &(1..=SEATS).map(seat).collect::<Vec<_>>()).expect("valid table");
    apply(
        &mut table,
        1,
        &[TableEffect::SceneEntered { scene: scene(10) }],
    )
    .expect("enters");
    let mut effects: Vec<TableEffect> = (1..=SEATS)
        .map(|number| TableEffect::CastEntered {
            entry: hero(
                &format!("hero{number}"),
                &format!("Hero{number}"),
                number,
                10,
                10,
            ),
        })
        .collect();
    for number in 0..creatures {
        let entry = CastEntry::new(
            element(&format!("creature{number}")),
            CastKind::Npc,
            Controller::Engine,
            scene(10),
            display_name(&format!("Creature {number}")),
            Vitals::new(5, 5).expect("valid vitals"),
        )
        .at(place(u32::try_from(number % 100).expect("small"), 20));
        effects.push(TableEffect::CastEntered { entry });
    }
    apply(&mut table, 2, &effects).expect("the cast enters");
    table
}

fn fact_id(number: usize) -> FactId {
    let value = u128::try_from(number).expect("small") + 1;
    FactId::from_nonzero_u128(std::num::NonZeroU128::new(value).expect("nonzero"))
}

/// Public story entries and `facts` private facts spread over the six seats.
fn attachments_of(facts: usize, story_entries: usize) -> Attachments {
    let mut attachments = Attachments::new();
    attachments
        .set_place(scene(10), text("The cellar"))
        .unwrap();
    for number in 0..story_entries {
        attachments
            .push_public(public_line(&format!("public line {number}")))
            .unwrap();
        attachments
            .push_private(
                seat(1 + u64::try_from(number).unwrap() % SEATS),
                public_line("private"),
            )
            .unwrap();
    }
    for number in 0..facts {
        let owner = seat(1 + u64::try_from(number).unwrap() % SEATS);
        attachments
            .set_private_fact(
                owner,
                fact_id(number),
                Secret::new(format!("fact {number}")),
            )
            .unwrap();
    }
    attachments
}

/// What a build examines for all seven audiences together.
fn work_of(table: &TableState, attachments: &Attachments) -> ProjectionWork {
    let snapshot = table.snapshot();
    let mut total = ProjectionWork::default();
    for audience in audiences() {
        let work = project(&snapshot, attachments, audience)
            .expect("projects")
            .record()
            .work;
        total.cast_visited += work.cast_visited;
        total.story_entries_visited += work.story_entries_visited;
        total.facts_visited += work.facts_visited;
    }
    total
}

#[test]
fn a_representative_table_costs_one_visit_per_element_on_the_shared_screen_and_one_per_fact() {
    let creatures = 400;
    let facts = 3_000;
    let table = table_of(creatures);
    let attachments = attachments_of(facts, 50);
    let work = work_of(&table, &attachments);
    let cast = creatures + usize::try_from(SEATS).unwrap();
    assert_eq!(table.snapshot().cast_len(), cast);
    assert_eq!(
        work.cast_visited, cast,
        "only the shared screen scans the cast"
    );
    assert_eq!(
        work.facts_visited, facts,
        "each fact is read by its own seat only"
    );
}

#[test]
fn doubling_the_elements_and_the_facts_exactly_doubles_the_work_it_never_squares_it() {
    let small = work_of(&table_of(200), &attachments_of(1_500, 50));
    let large = work_of(&table_of(400), &attachments_of(3_000, 50));
    let cast_small = 200 + usize::try_from(SEATS).unwrap();
    let cast_large = 400 + usize::try_from(SEATS).unwrap();
    assert_eq!(small.cast_visited, cast_small);
    assert_eq!(large.cast_visited, cast_large);
    assert_eq!(large.facts_visited, 2 * small.facts_visited);
    assert!(large.total() < 2 * small.total() + audiences().len() * 400);
}

#[test]
fn the_work_does_not_depend_on_how_long_the_story_is() {
    let table = table_of(20);
    let short = work_of(&table, &attachments_of(0, 30));
    let long = work_of(&table, &attachments_of(0, 30_000));
    assert_eq!(short.story_entries_visited, long.story_entries_visited);
    let seats = usize::try_from(SEATS).unwrap();
    assert!(
        long.story_entries_visited <= 1 + seats * (PHONE_NARRATION_LINES + 1),
        "{} entries examined",
        long.story_entries_visited
    );
}

#[test]
fn a_seats_build_never_reads_another_seats_facts_or_story() {
    let table = table_of(10);
    let attachments = attachments_of(600, 40);
    let snapshot = table.snapshot();
    for number in 1..=SEATS {
        let work = project(&snapshot, &attachments, Audience::Seat(seat(number)))
            .expect("projects")
            .record()
            .work;
        assert_eq!(
            work.facts_visited, 100,
            "seat {number} has a hundred facts of its own"
        );
    }
    let shared = project(&snapshot, &attachments, Audience::Public)
        .expect("projects")
        .record()
        .work;
    assert_eq!(
        shared.facts_visited, 0,
        "the shared screen reads no private fact"
    );
    assert_eq!(
        shared.story_entries_visited, 1,
        "only the newest public line"
    );
}

#[test]
fn the_per_audience_record_total_adds_up_what_was_counted() {
    let table = table_of(5);
    let attachments = attachments_of(12, 3);
    let work = project(&table.snapshot(), &attachments, Audience::Public)
        .expect("projects")
        .record()
        .work;
    assert_eq!(
        work.total(),
        work.cast_visited + work.story_entries_visited + work.facts_visited
    );
}

#[test]
fn a_seats_work_is_bounded_by_its_note_window_however_many_facts_it_has_learned() {
    use crate::projection::bounds::NOTE_BUDGET_BYTES;
    let table = table_of(10);
    let attachments = attachments_of(60_000, 0);
    let snapshot = table.snapshot();
    let shortest_fact = "fact 0".len();
    for number in 1..=SEATS {
        let work = project(&snapshot, &attachments, Audience::Seat(seat(number)))
            .expect("projects")
            .record()
            .work;
        assert!(
            work.facts_visited <= NOTE_BUDGET_BYTES / shortest_fact + 1,
            "seat {number} examined {} of its 10,000 facts",
            work.facts_visited
        );
    }
}
