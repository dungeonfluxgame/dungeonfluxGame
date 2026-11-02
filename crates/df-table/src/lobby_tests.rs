use std::sync::Arc;

use crate::conformance::{self, ConformanceFixture, Expectation, ScriptedStep};
use crate::effects::{ApplyOutcome, TableEffect, TableState};
use crate::envelope::TimerName;
use crate::explore::{AuthoredBeat, ExploreMachine};
use crate::identity::{SceneId, SeatId};
use crate::lobby::{LobbyAction, LobbyMachine};
use crate::machine::{
    CarryItem, Entry, Machine, MachineInput, MachineSpec, Scope, StateName, StepInput, StepOutcome,
    StepResult,
};
use crate::machine_slot::MachineSlot;
use crate::outcome::{DeclineCode, Outcome};
use crate::ports::{ActionId, Audience};
use crate::testkit::{QueuedDice, ScriptedBroker, ScriptedRulesOwner, correlation};

pub(crate) const FIRST_SCENE: SceneId = SceneId::new(7);

pub(crate) fn seat(number: u64) -> SeatId {
    SeatId::new(number)
}

pub(crate) fn action(name: &str) -> ActionId {
    ActionId::new(name).expect("valid action")
}

pub(crate) fn lobby() -> LobbyMachine {
    LobbyMachine::new(FIRST_SCENE).expect("the lobby spec is valid")
}

pub(crate) fn explore_for_neighbour() -> ExploreMachine {
    let beat = AuthoredBeat::new("locked_gate", "investigate").expect("valid beat");
    ExploreMachine::new(beat).expect("the explore spec is valid")
}

/// A table of four seats where seats 1 and 2 are already taken.
pub(crate) fn table_with_two_seated() -> TableState {
    TableState::seated(4, &[seat(1), seat(2)]).expect("a valid table")
}

pub(crate) fn lobby_fixture() -> ConformanceFixture {
    let intent =
        |who: u64, name: &str, expect| ScriptedStep::player(seat(who), action(name), expect);
    ConformanceFixture {
        machine: Arc::new(lobby()),
        neighbours: vec![Arc::new(explore_for_neighbour())],
        table: table_with_two_seated(),
        entry: Entry::empty(),
        refused_entry: None,
        script: vec![
            intent(3, "join", Expectation::applied()),
            intent(1, "ready", Expectation::applied()),
            intent(1, "start", Expectation::declined()),
            intent(1, "attack", Expectation::refused()),
            ScriptedStep::raw(
                MachineInput::Timer {
                    timer: TimerName::new("idle_nudge").expect("valid timer"),
                },
                Expectation::refused(),
            ),
            intent(2, "ready", Expectation::applied()),
            intent(3, "ready", Expectation::applied()),
            intent(
                1,
                "start",
                Expectation::applied()
                    .moving("gathering", "started")
                    .handing_off(),
            ),
            intent(1, "join", Expectation::refused()),
        ],
        rules: ScriptedRulesOwner::default(),
        dice: QueuedDice::default(),
        broker: ScriptedBroker::default(),
    }
}

fn intent(who: u64, name: &str) -> MachineInput {
    MachineInput::Intent {
        seat: seat(who),
        action: action(name),
    }
}

fn step_input(input: MachineInput) -> StepInput {
    StepInput {
        correlation: correlation(1),
        now_ms: 0,
        input,
        decision: None,
    }
}

fn entered(table: &TableState) -> (LobbyMachine, MachineSlot) {
    let machine = lobby();
    let slot = machine.enter(table, &Entry::empty()).expect("enters");
    (machine, slot)
}

fn run(machine: &LobbyMachine, slot: &MachineSlot, input: MachineInput) -> StepResult {
    machine.step(slot, &step_input(input)).expect("steps")
}

fn declined_code(result: &StepResult) -> Option<DeclineCode> {
    match &result.outcome {
        StepOutcome::Settled(Outcome::Declined(decline)) => Some(decline.code()),
        _ => None,
    }
}

#[test]
fn the_lobby_passes_the_whole_conformance_suite() {
    let report = conformance::run(lobby_fixture());
    assert!(report.is_clean(), "{:#?}", report.findings());
}

#[test]
fn the_conformance_run_records_the_lobbys_transition_its_effects_and_its_hand_off() {
    let report = conformance::run(lobby_fixture());
    let lines: Vec<String> = report
        .events()
        .iter()
        .map(|event| event.render_audit_line())
        .collect();
    let has = |needle: &str| lines.iter().any(|line| line.contains(needle));
    assert!(has("MACHINE entered"), "{lines:#?}");
    assert!(has("MACHINE transitioned"));
    assert!(has("from=gathering to=started"));
    assert!(has("EFFECT applied"));
    assert!(has("effect=seat_joined"));
    assert!(has("effect=scene_entered"));
    assert!(has("MACHINE exited"));
    assert!(has("target=explore"));
    assert!(has("INPUT declined"));
    assert!(has("MACHINE failed"));
    assert!(has("LOAD refused"));
}

#[test]
fn entering_the_lobby_seats_everyone_the_table_already_holds_and_ready_for_nobody() {
    let (machine, slot) = entered(&table_with_two_seated());
    let readout = machine.project(&slot, Audience::Public).expect("projects");
    assert_eq!(readout.as_str(), "0 of 2 seats are ready.");
}

#[test]
fn a_seat_joins_once_and_the_join_leaves_as_a_table_effect() {
    let (machine, slot) = entered(&table_with_two_seated());
    let joined = run(&machine, &slot, intent(3, "join"));
    assert_eq!(
        joined.table_effects,
        vec![TableEffect::SeatJoined { seat: seat(3) }]
    );
    let again = run(&machine, &joined.state, intent(3, "join"));
    assert_eq!(declined_code(&again), Some(DeclineCode::NotViable));
    assert_eq!(again.state, joined.state);
    assert!(again.table_effects.is_empty());
}

#[test]
fn a_full_table_declines_a_join() {
    let table = TableState::seated(2, &[seat(1), seat(2)]).expect("valid table");
    let (machine, slot) = entered(&table);
    let refused = run(&machine, &slot, intent(3, "join"));
    assert_eq!(declined_code(&refused), Some(DeclineCode::NotViable));
    assert_eq!(refused.state, slot);
}

#[test]
fn a_seat_that_leaves_frees_its_place_and_a_stranger_cannot_leave_or_ready() {
    let (machine, slot) = entered(&table_with_two_seated());
    let left = run(&machine, &slot, intent(2, "leave"));
    assert_eq!(
        left.table_effects,
        vec![TableEffect::SeatLeft { seat: seat(2) }]
    );
    for name in ["leave", "ready", "unready", "start"] {
        let refused = run(&machine, &slot, intent(9, name));
        assert_eq!(
            declined_code(&refused),
            Some(DeclineCode::NotViable),
            "{name}"
        );
        assert_eq!(refused.state, slot, "{name}");
    }
}

#[test]
fn ready_and_unready_change_only_the_lobbys_own_record() {
    let (machine, slot) = entered(&table_with_two_seated());
    let ready = run(&machine, &slot, intent(1, "ready"));
    assert!(ready.table_effects.is_empty());
    let shown = machine
        .project(&ready.state, Audience::Seat(seat(1)))
        .expect("projects");
    assert!(
        shown.as_str().starts_with("You are ready."),
        "{}",
        shown.as_str()
    );
    let undone = run(&machine, &ready.state, intent(1, "unready"));
    assert_eq!(undone.state, slot);
}

#[test]
fn starting_needs_every_seat_ready_and_names_what_is_missing() {
    let (machine, slot) = entered(&table_with_two_seated());
    let one_ready = run(&machine, &slot, intent(1, "ready"));
    let blocked = run(&machine, &one_ready.state, intent(1, "start"));
    assert_eq!(declined_code(&blocked), Some(DeclineCode::NotViable));
    assert!(!blocked.asks_for_hand_off);
    let StepOutcome::Settled(Outcome::Declined(decline)) = &blocked.outcome else {
        panic!("a start with a seat not ready is declined");
    };
    assert_eq!(decline.missing_piece(), Some("every seat ready"));
}

#[test]
fn starting_enters_the_first_scene_and_asks_to_hand_off_with_the_roster() {
    let (machine, slot) = entered(&table_with_two_seated());
    let first = run(&machine, &slot, intent(1, "ready"));
    let both = run(&machine, &first.state, intent(2, "ready"));
    let started = run(&machine, &both.state, intent(1, "start"));
    assert_eq!(
        started.table_effects,
        vec![TableEffect::SceneEntered { scene: FIRST_SCENE }]
    );
    assert!(started.asks_for_hand_off);
    let hand_off = machine.exit(&started.state).expect("exits");
    assert_eq!(hand_off.target().as_str(), "explore");
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
fn a_started_lobby_accepts_nothing_and_a_start_with_nobody_seated_is_declined() {
    let (machine, slot) = entered(&table_with_two_seated());
    let first = run(&machine, &slot, intent(1, "ready"));
    let both = run(&machine, &first.state, intent(2, "ready"));
    let started = run(&machine, &both.state, intent(1, "start"));
    assert!(!machine.accepts(&intent(1, "join"), &started.state));
    let empty = TableState::new(4).expect("valid table");
    let (machine, slot) = entered(&empty);
    let nobody = run(&machine, &slot, intent(1, "start"));
    assert_eq!(declined_code(&nobody), Some(DeclineCode::NotViable));
}

#[test]
fn the_lobbys_effects_apply_to_the_table_it_ran_against() {
    let table = table_with_two_seated();
    let (machine, slot) = entered(&table);
    let joined = run(&machine, &slot, intent(3, "join"));
    let mut table = table;
    let scope = crate::identity::TableScope::of_table(crate::identity::TableId::new(1));
    let outcome = table
        .apply(&scope, correlation(1), &joined.table_effects)
        .expect("applies");
    assert!(matches!(outcome, ApplyOutcome::Applied(_)));
    assert!(table.seats().contains(&seat(3)));
}

#[test]
fn every_lobby_action_has_a_distinct_key_the_lobby_recognises() {
    let keys: std::collections::BTreeSet<_> =
        LobbyAction::ALL.iter().map(|action| action.key()).collect();
    assert_eq!(keys.len(), LobbyAction::ALL.len());
    for lobby_action in LobbyAction::ALL {
        assert_eq!(
            LobbyAction::of(&action(lobby_action.key())),
            Some(*lobby_action)
        );
    }
    assert_eq!(LobbyAction::of(&action("attack")), None);
}

#[test]
fn the_lobby_declares_a_table_scope_machine_that_hands_off_to_explore() {
    let machine = lobby();
    let spec: &MachineSpec = machine.spec();
    assert_eq!(spec.scope(), Scope::Table);
    assert_eq!(spec.hand_off_targets().len(), 1);
    assert!(spec.required_ports().is_empty());
    assert!(spec.rules_actions().is_empty());
    let names: Vec<&str> = spec.states().iter().map(StateName::as_str).collect();
    assert_eq!(names, ["gathering", "started"]);
}
