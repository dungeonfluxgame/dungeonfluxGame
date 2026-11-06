use std::num::NonZeroU16;
use std::sync::Arc;

use crate::conformance::{self, ConformanceFixture, Expectation, ScriptedStep};
use crate::effects::{BeatName, TableEffect, TableState};
use crate::envelope::{PendingId, TimerName};
use crate::explore::{AuthoredBeat, ExploreMachine};
use crate::identity::{SceneId, TableId, TableScope};
use crate::lobby_tests::{FIRST_SCENE, action, explore_for_neighbour, lobby, offered, seat};
use crate::machine::slot::MachineSlot;
use crate::machine::{
    CarryItem, Decision, Entry, EntryError, Machine, MachineError, MachineInput, StepInput,
    StepOutcome, StepResult,
};
use crate::outcome::{Decline, DeclineCode, Outcome};
use crate::ports::{Audience, Draw, DrawRequest, PortName, Preparation, Resolution};
use crate::testkit::{QueuedDice, ScriptedBroker, ScriptedRulesOwner, correlation};

fn d20() -> DrawRequest {
    DrawRequest {
        sides: NonZeroU16::new(20).expect("a die has sides"),
    }
}

/// A table whose active scene is the lobby's first scene, with seats 1 and 2 at it.
pub(crate) fn table_in_the_first_scene() -> TableState {
    let mut table = TableState::seated(4, &[seat(1), seat(2)]).expect("valid table");
    let scope = TableScope::of_table(TableId::new(1));
    table
        .apply(
            &scope,
            correlation(100),
            &[TableEffect::SceneEntered { scene: FIRST_SCENE }],
        )
        .expect("the scene is entered");
    table
}

pub(crate) fn entry_to_the_first_scene() -> Entry {
    Entry::carrying(vec![
        CarryItem::Scene(FIRST_SCENE),
        CarryItem::Seat(seat(2)),
        CarryItem::Seat(seat(1)),
    ])
}

pub(crate) fn explore_fixture() -> ConformanceFixture {
    let intent =
        |who: u64, name: &str, expect| ScriptedStep::player(seat(who), action(name), expect);
    let rules = ScriptedRulesOwner::default().with_action(
        action("investigate"),
        Preparation::Ready { draws: vec![d20()] },
        Resolution::applied(),
    );
    ConformanceFixture {
        machine: Arc::new(explore_for_neighbour()),
        neighbours: vec![Arc::new(lobby())],
        wired: vec![PortName::RulesOwner],
        table: table_in_the_first_scene(),
        entry: entry_to_the_first_scene(),
        refused_entry: Some(Entry::empty()),
        script: vec![
            intent(9, "investigate", Expectation::refused()),
            intent(
                1,
                "investigate",
                Expectation::waiting().moving("presenting", "awaiting_roll"),
            ),
            ScriptedStep::raw(
                MachineInput::Timer {
                    timer: TimerName::new("idle_nudge").expect("valid timer"),
                },
                Expectation::refused(),
            ),
            intent(2, "investigate", Expectation::refused()),
            ScriptedStep::answering_open_roll(
                Expectation::applied()
                    .moving("awaiting_roll", "resolved")
                    .handing_off(),
            ),
            intent(1, "investigate", Expectation::refused().handing_off()),
        ],
        rules,
        dice: QueuedDice::new([14]),
        broker: ScriptedBroker::default(),
    }
}

fn explore() -> ExploreMachine {
    explore_for_neighbour()
}

fn entered() -> (ExploreMachine, MachineSlot, TableState) {
    let machine = explore();
    let table = table_in_the_first_scene();
    let slot = machine
        .enter(&table, &entry_to_the_first_scene())
        .expect("enters");
    (machine, slot, table)
}

fn investigate(who: u64) -> MachineInput {
    MachineInput::Intent {
        seat: seat(who),
        action: action("investigate"),
    }
}

fn dice_for(pending: u64) -> MachineInput {
    MachineInput::Dice {
        pending: PendingId::new(pending),
        draws: vec![Draw::new(NonZeroU16::new(20).expect("sides"), 14).expect("on the die")],
    }
}

fn with(table: &TableState, input: MachineInput, decision: Option<Decision>) -> StepInput<'_> {
    StepInput {
        correlation: correlation(1),
        now_ms: 0,
        table,
        input,
        decision,
    }
}

fn declined_code(result: &StepResult) -> Option<DeclineCode> {
    match &result.outcome {
        StepOutcome::Settled(Outcome::Declined(decline)) => Some(decline.code()),
        _ => None,
    }
}

/// The state after a roll was opened for pending 5.
fn waiting_state(machine: &ExploreMachine, slot: &MachineSlot, table: &TableState) -> MachineSlot {
    machine
        .step(
            slot,
            &with(
                table,
                investigate(1),
                Some(Decision::RollPending(PendingId::new(5))),
            ),
        )
        .expect("steps")
        .state
        .expect("waiting records the pending")
}

#[test]
fn explore_passes_the_whole_conformance_suite() {
    let report = conformance::run(explore_fixture());
    assert!(report.is_clean(), "{:#?}", report.findings());
    assert!(report.skipped().is_empty());
}

#[test]
fn the_conformance_run_shows_explore_waiting_then_resolving() {
    let report = conformance::run(explore_fixture());
    let lines: Vec<String> = report
        .events()
        .iter()
        .map(|event| event.render_audit_line())
        .collect();
    let has = |needle: &str| lines.iter().any(|line| line.contains(needle));
    assert!(has("from=presenting to=awaiting_roll"), "{lines:#?}");
    assert!(has("from=awaiting_roll to=resolved"));
    assert!(has("effect=beat_resolved"));
    assert!(has("kind=machine target=lobby"));
}

#[test]
fn entering_needs_a_scene_the_table_is_in_and_seats_it_holds() {
    let machine = explore();
    let table = table_in_the_first_scene();
    let refusals = [
        (Entry::empty(), EntryError::MissingScene),
        (
            Entry::carrying(vec![CarryItem::Seat(seat(1))]),
            EntryError::MissingScene,
        ),
        (
            Entry::carrying(vec![
                CarryItem::Scene(SceneId::new(99)),
                CarryItem::Seat(seat(1)),
            ]),
            EntryError::SceneNotActive {
                scene: SceneId::new(99),
            },
        ),
        (
            Entry::carrying(vec![CarryItem::Scene(FIRST_SCENE)]),
            EntryError::NoSeats,
        ),
        (
            Entry::carrying(vec![
                CarryItem::Scene(FIRST_SCENE),
                CarryItem::Seat(seat(9)),
            ]),
            EntryError::SeatNotAtTable { seat: seat(9) },
        ),
    ];
    for (entry, expected) in refusals {
        assert_eq!(
            machine.enter(&table, &entry),
            Err(MachineError::Entry(expected)),
            "{entry:?}"
        );
    }
}

#[test]
fn only_a_seat_at_the_table_can_make_the_check() {
    let (machine, slot, table) = entered();
    assert!(machine.accepts(&table, &investigate(1), &slot));
    assert!(!machine.accepts(&table, &investigate(9), &slot));
    let other_action = MachineInput::Intent {
        seat: seat(1),
        action: action("attack"),
    };
    assert!(!machine.accepts(&table, &other_action, &slot));
}

#[test]
fn a_seat_that_left_the_table_can_no_longer_act() {
    let (machine, slot, mut table) = entered();
    let scope = TableScope::of_table(TableId::new(1));
    table
        .apply(
            &scope,
            correlation(101),
            &[TableEffect::SeatLeft { seat: seat(1) }],
        )
        .expect("the seat leaves");
    assert!(!machine.accepts(&table, &investigate(1), &slot));
    assert!(machine.accepts(&table, &investigate(2), &slot));
    let hand_off = machine.exit(&table, &slot).expect("exits");
    assert_eq!(
        hand_off.carry(),
        [CarryItem::Scene(FIRST_SCENE), CarryItem::Seat(seat(2))]
    );
}

#[test]
fn a_roll_decision_makes_the_machine_wait_on_that_pending_and_hold_only_its_id() {
    let (machine, slot, table) = entered();
    let pending = PendingId::new(5);
    let waiting = machine
        .step(
            &slot,
            &with(&table, investigate(1), Some(Decision::RollPending(pending))),
        )
        .expect("steps");
    assert_eq!(waiting.outcome, StepOutcome::Waiting(pending));
    assert!(waiting.table_effects.is_empty());
    assert!(waiting.resource_requests.is_empty());
    let state = waiting.state.expect("the pending is recorded");
    assert!(!machine.wants_hand_off(&state).expect("asks"));
    assert!(!machine.accepts(&table, &investigate(1), &state));
    assert!(machine.accepts(&table, &dice_for(5), &state));
    assert!(!machine.accepts(&table, &dice_for(6), &state));
}

#[test]
fn the_dice_of_the_open_roll_resolve_the_beat_and_the_machine_then_asks_to_hand_back() {
    let (machine, slot, table) = entered();
    let waiting = waiting_state(&machine, &slot, &table);
    let resolved = machine
        .step(
            &waiting,
            &with(
                &table,
                dice_for(5),
                Some(Decision::Resolved(Resolution::applied())),
            ),
        )
        .expect("steps");
    assert_eq!(resolved.outcome, StepOutcome::Settled(Outcome::Applied));
    assert_eq!(
        resolved.table_effects,
        vec![TableEffect::BeatResolved {
            scene: FIRST_SCENE,
            beat: BeatName::new("locked_gate").expect("valid beat"),
        }]
    );
    let done = resolved.state.expect("resolved is a new state");
    assert!(machine.wants_hand_off(&done).expect("asks"));
    assert!(
        machine
            .wants_hand_off(&done)
            .expect("asks again after a refusal")
    );
    let hand_off = machine.exit(&table, &done).expect("exits");
    assert_eq!(
        hand_off.machine_target().map(|id| id.as_str()),
        Some("lobby")
    );
    assert_eq!(
        hand_off.carry(),
        [
            CarryItem::Scene(FIRST_SCENE),
            CarryItem::Seat(seat(1)),
            CarryItem::Seat(seat(2))
        ]
    );
}

#[test]
fn a_check_the_rules_resolve_at_once_resolves_the_beat_without_a_pending() {
    let (machine, slot, table) = entered();
    let resolved = machine
        .step(
            &slot,
            &with(
                &table,
                investigate(1),
                Some(Decision::Resolved(Resolution::applied())),
            ),
        )
        .expect("steps");
    assert_eq!(resolved.outcome, StepOutcome::Settled(Outcome::Applied));
    let done = resolved.state.expect("state");
    assert!(machine.wants_hand_off(&done).expect("asks"));
}

#[test]
fn a_decline_from_the_rules_owner_changes_nothing_even_for_a_roll_answer() {
    let (machine, slot, table) = entered();
    let decline = Decline::with_default_reason(DeclineCode::MissingRequirement);
    let refused = machine
        .step(
            &slot,
            &with(
                &table,
                investigate(1),
                Some(Decision::Resolved(Resolution::Declined(decline.clone()))),
            ),
        )
        .expect("steps");
    assert_eq!(
        declined_code(&refused),
        Some(DeclineCode::MissingRequirement)
    );
    assert!(refused.state.is_none() && refused.table_effects.is_empty());
    let waiting = waiting_state(&machine, &slot, &table);
    let declined_roll = machine
        .step(
            &waiting,
            &with(
                &table,
                dice_for(5),
                Some(Decision::Resolved(Resolution::Declined(decline))),
            ),
        )
        .expect("steps");
    assert_eq!(
        declined_code(&declined_roll),
        Some(DeclineCode::MissingRequirement)
    );
    assert!(
        declined_roll.state.is_none(),
        "the pending stays open for C4 to cancel or re-ask"
    );
    assert!(machine.accepts(&table, &dice_for(5), &waiting));
}

#[test]
fn a_check_with_no_decision_is_declined_and_never_applied() {
    let (machine, slot, table) = entered();
    let result = machine
        .step(&slot, &with(&table, investigate(1), None))
        .expect("steps");
    assert_eq!(declined_code(&result), Some(DeclineCode::NotViable));
    assert!(result.state.is_none());
}

#[test]
fn a_roll_decision_on_a_dice_answer_is_declined_and_keeps_the_wait() {
    let (machine, slot, table) = entered();
    let waiting = waiting_state(&machine, &slot, &table);
    let result = machine
        .step(
            &waiting,
            &with(
                &table,
                dice_for(5),
                Some(Decision::RollPending(PendingId::new(6))),
            ),
        )
        .expect("steps");
    assert_eq!(declined_code(&result), Some(DeclineCode::NotViable));
    assert!(result.state.is_none());
}

#[test]
fn explore_shows_each_audience_something_true_in_every_state() {
    let (machine, slot, table) = entered();
    let public = machine
        .project(&table, &slot, Audience::Public)
        .expect("projects");
    let seated = machine
        .project(&table, &slot, Audience::Seat(seat(1)))
        .expect("projects");
    let stranger = machine
        .project(&table, &slot, Audience::Seat(seat(9)))
        .expect("projects");
    assert_eq!(public, stranger);
    assert_ne!(public, seated);
    assert!(seated.as_str().contains("investigate"));
}

#[test]
fn an_authored_beat_needs_valid_names() {
    assert!(AuthoredBeat::new("Locked Gate", "investigate").is_err());
    assert!(AuthoredBeat::new("locked_gate", "").is_err());
}

#[test]
fn explore_declares_the_rules_action_the_port_and_the_entry_it_needs() {
    let machine = explore();
    let spec = machine.spec();
    assert_eq!(spec.rules_actions(), [action("investigate")]);
    assert_eq!(spec.required_ports(), [PortName::RulesOwner]);
    assert_eq!(spec.scope(), crate::machine::Scope::Group);
    assert!(spec.reads_entry());
}

#[test]
fn a_fixture_without_a_refused_entry_fails_for_a_machine_that_reads_its_entry() {
    let mut fixture = explore_fixture();
    fixture.refused_entry = None;
    let report = conformance::run(fixture);
    assert!(
        report
            .failed_checks()
            .contains(&conformance::ConformanceCheck::EntryRefusal)
    );
}

#[test]
fn the_check_is_a_standing_offer_to_a_seat_at_the_table_while_the_beat_is_presented() {
    let (machine, slot, table) = entered();
    assert_eq!(
        offered(&machine, &table, &slot, seat(1)),
        [action("investigate")]
    );
    assert!(offered(&machine, &table, &slot, seat(9)).is_empty());
    let waiting = waiting_state(&machine, &slot, &table);
    assert!(offered(&machine, &table, &waiting, seat(1)).is_empty());
}
