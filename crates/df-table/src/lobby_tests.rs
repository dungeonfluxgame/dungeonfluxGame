use std::sync::Arc;

use crate::conformance::{self, ConformanceFixture, Expectation, ScriptedStep};
use crate::effects::{ApplyOutcome, TableEffect, TableState};
use crate::envelope::TimerName;
use crate::explore::{AuthoredBeat, ExploreMachine};
use crate::identity::{SceneId, SeatId, TableId, TableScope};
use crate::lobby::{LobbyAction, LobbyMachine};
use crate::machine::slot::{MachineSlot, decodes_on_this_thread};
use crate::machine::{
    CarryItem, Entry, Machine, MachineInput, MachineSpec, Scope, StateName, StepInput, StepOutcome,
    StepResult,
};
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

/// The buttons one seat holds, from one call of the machine that answers for all of them.
pub(crate) fn offered(
    machine: &impl Machine,
    table: &TableState,
    slot: &MachineSlot,
    who: SeatId,
) -> Vec<ActionId> {
    machine
        .standing_offers(table, slot, &[who])
        .expect("the state reads")
        .into_iter()
        .flat_map(|(_, actions)| actions)
        .collect()
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
        wired: Vec::new(),
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
            intent(3, "ready", Expectation::applied()),
            ScriptedStep::table_change(TableEffect::SeatLeft { seat: seat(3) }),
            intent(2, "ready", Expectation::applied()),
            intent(
                1,
                "start",
                Expectation::applied()
                    .moving("gathering", "started")
                    .handing_off(),
            ),
            intent(1, "join", Expectation::refused().handing_off()),
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

fn step_input(table: &TableState, input: MachineInput) -> StepInput<'_> {
    StepInput {
        correlation: correlation(1),
        now_ms: 0,
        table,
        input,
        decision: None,
    }
}

fn entered(table: &TableState) -> (LobbyMachine, MachineSlot) {
    let machine = lobby();
    let slot = machine.enter(table, &Entry::empty()).expect("enters");
    (machine, slot)
}

fn run(
    machine: &LobbyMachine,
    table: &TableState,
    slot: &MachineSlot,
    input: MachineInput,
) -> StepResult {
    machine
        .step(slot, &step_input(table, input))
        .expect("steps")
}

/// Plays a result forward: the new state, and the table after its effects.
fn commit(
    table: &mut TableState,
    slot: &MachineSlot,
    result: StepResult,
    turn: u64,
) -> MachineSlot {
    let scope = TableScope::of_table(TableId::new(1));
    table
        .apply(&scope, correlation(turn), &result.table_effects)
        .expect("the effects apply");
    result.state.unwrap_or_else(|| slot.clone())
}

fn declined_code(result: &StepResult) -> Option<DeclineCode> {
    match &result.outcome {
        StepOutcome::Settled(Outcome::Declined(decline)) => Some(decline.code()),
        _ => None,
    }
}

fn host_removes(table: &mut TableState, who: u64, turn: u64) {
    let scope = TableScope::of_table(TableId::new(1));
    table
        .apply(
            &scope,
            correlation(turn),
            &[TableEffect::SeatLeft { seat: seat(who) }],
        )
        .expect("the seat leaves");
}

#[test]
fn the_lobby_passes_the_whole_conformance_suite() {
    let report = conformance::run(lobby_fixture());
    assert!(report.is_clean(), "{:#?}", report.findings());
    assert_eq!(
        report.skipped(),
        [conformance::ConformanceCheck::EntryRefusal],
        "the lobby reads nothing from its entry, so that check is listed as skipped"
    );
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
    assert!(has("kind=machine target=explore"));
    assert!(has("INPUT declined"));
    assert!(has("MACHINE failed"));
    assert!(has(
        "LOAD refused table=1 record=machine_slot machine=lobby"
    ));
}

#[test]
fn entering_the_lobby_is_ready_for_nobody_whoever_sits_at_the_table() {
    let table = table_with_two_seated();
    let (machine, slot) = entered(&table);
    let readout = machine
        .project(&table, &slot, Audience::Public)
        .expect("projects");
    assert_eq!(readout.as_str(), "0 of 2 seats are ready.");
}

#[test]
fn a_seat_joins_once_and_the_join_leaves_as_a_table_effect() {
    let table = table_with_two_seated();
    let (machine, slot) = entered(&table);
    let joined = run(&machine, &table, &slot, intent(3, "join"));
    assert_eq!(
        joined.table_effects,
        vec![TableEffect::SeatJoined { seat: seat(3) }]
    );
    assert_eq!(
        joined.state, None,
        "a join changes no fact of the lobby's own"
    );
    let mut table = table;
    commit(&mut table, &slot, joined, 1);
    let again = run(&machine, &table, &slot, intent(3, "join"));
    assert_eq!(declined_code(&again), Some(DeclineCode::NotViable));
    assert_eq!(again.state, None);
    assert!(again.table_effects.is_empty());
}

#[test]
fn a_full_table_declines_a_join() {
    let table = TableState::seated(2, &[seat(1), seat(2)]).expect("valid table");
    let (machine, slot) = entered(&table);
    let refused = run(&machine, &table, &slot, intent(3, "join"));
    assert_eq!(declined_code(&refused), Some(DeclineCode::NotViable));
    assert_eq!(refused.state, None);
}

#[test]
fn a_seat_that_leaves_frees_its_place_and_a_stranger_cannot_leave_or_ready() {
    let table = table_with_two_seated();
    let (machine, slot) = entered(&table);
    let left = run(&machine, &table, &slot, intent(2, "leave"));
    assert_eq!(
        left.table_effects,
        vec![TableEffect::SeatLeft { seat: seat(2) }]
    );
    for name in ["leave", "ready", "unready", "start"] {
        let refused = run(&machine, &table, &slot, intent(9, name));
        assert_eq!(
            declined_code(&refused),
            Some(DeclineCode::NotViable),
            "{name}"
        );
        assert_eq!(refused.state, None, "{name}");
        assert!(!machine.accepts(&table, &intent(9, name), &slot), "{name}");
    }
    assert!(machine.accepts(&table, &intent(9, "join"), &slot));
}

#[test]
fn ready_and_unready_change_only_the_lobbys_own_record() {
    let table = table_with_two_seated();
    let (machine, slot) = entered(&table);
    let ready = run(&machine, &table, &slot, intent(1, "ready"));
    assert!(ready.table_effects.is_empty());
    let next = ready.state.expect("readiness is the lobby's own fact");
    let shown = machine
        .project(&table, &next, Audience::Seat(seat(1)))
        .expect("projects");
    assert!(
        shown.as_str().starts_with("You are ready."),
        "{}",
        shown.as_str()
    );
    let again = run(&machine, &table, &next, intent(1, "ready"));
    assert_eq!(again.state, None, "saying ready twice changes nothing");
    let undone = run(&machine, &table, &next, intent(1, "unready"));
    assert_eq!(undone.state, Some(slot));
}

#[test]
fn starting_needs_every_seat_ready_and_names_what_is_missing() {
    let table = table_with_two_seated();
    let (machine, slot) = entered(&table);
    let one_ready = run(&machine, &table, &slot, intent(1, "ready"));
    let next = one_ready.state.expect("state");
    let blocked = run(&machine, &table, &next, intent(1, "start"));
    assert_eq!(declined_code(&blocked), Some(DeclineCode::NotViable));
    assert_eq!(blocked.state, None);
    let StepOutcome::Settled(Outcome::Declined(decline)) = &blocked.outcome else {
        panic!("a start with a seat not ready is declined");
    };
    assert_eq!(decline.missing_piece(), Some("every seat ready"));
}

#[test]
fn starting_enters_the_first_scene_and_the_lobby_then_asks_to_hand_off_with_the_roster() {
    let mut table = table_with_two_seated();
    let (machine, mut slot) = entered(&table);
    for (turn, who) in [(1, 1), (2, 2)] {
        let ready = run(&machine, &table, &slot, intent(who, "ready"));
        slot = commit(&mut table, &slot, ready, turn);
    }
    assert!(!machine.wants_hand_off(&slot).expect("asks"));
    let started = run(&machine, &table, &slot, intent(1, "start"));
    assert_eq!(
        started.table_effects,
        vec![TableEffect::SceneEntered { scene: FIRST_SCENE }]
    );
    let slot = commit(&mut table, &slot, started, 3);
    assert!(machine.wants_hand_off(&slot).expect("asks"));
    let hand_off = machine.exit(&table, &slot).expect("exits");
    assert_eq!(
        hand_off.machine_target().map(|id| id.as_str()),
        Some("explore")
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
fn a_refused_hand_off_leaves_the_lobby_asking_and_the_ask_survives_other_input() {
    let mut table = table_with_two_seated();
    let (machine, mut slot) = entered(&table);
    for (turn, who) in [(1, 1), (2, 2)] {
        let ready = run(&machine, &table, &slot, intent(who, "ready"));
        slot = commit(&mut table, &slot, ready, turn);
    }
    let started = run(&machine, &table, &slot, intent(1, "start"));
    let slot = commit(&mut table, &slot, started, 3);
    for _ in 0..3 {
        assert!(machine.wants_hand_off(&slot).expect("asks"));
    }
    let again = run(&machine, &table, &slot, intent(1, "start"));
    assert_eq!(declined_code(&again), Some(DeclineCode::NotViable));
    assert!(machine.wants_hand_off(&slot).expect("asks"));
    let restored = machine.load(&slot).expect("a saved lobby loads");
    assert!(machine.wants_hand_off(&restored).expect("asks"));
}

#[test]
fn exiting_before_the_game_starts_carries_no_scene_so_the_next_machine_would_refuse_it() {
    let table = table_with_two_seated();
    let (machine, slot) = entered(&table);
    assert!(!machine.wants_hand_off(&slot).expect("asks"));
    let hand_off = machine.exit(&table, &slot).expect("exits");
    assert_eq!(
        hand_off.carry(),
        [CarryItem::Seat(seat(1)), CarryItem::Seat(seat(2))]
    );
    let explore = explore_for_neighbour();
    assert!(
        explore
            .enter(&table, &Entry::from_hand_off(&hand_off))
            .is_err()
    );
}

#[test]
fn a_seat_the_host_removed_cannot_hold_up_a_start() {
    let mut table = table_with_two_seated();
    let (machine, mut slot) = entered(&table);
    host_removes(&mut table, 2, 50);
    let ready = run(&machine, &table, &slot, intent(1, "ready"));
    slot = commit(&mut table, &slot, ready, 1);
    let started = run(&machine, &table, &slot, intent(1, "start"));
    assert_eq!(
        declined_code(&started),
        None,
        "seat 2 is gone, so only seat 1 must be ready"
    );
    assert_eq!(
        started.table_effects,
        vec![TableEffect::SceneEntered { scene: FIRST_SCENE }]
    );
}

#[test]
fn a_ready_seat_that_left_is_not_counted_and_a_join_is_judged_against_the_seats_now() {
    let mut table = TableState::seated(2, &[seat(1), seat(2)]).expect("valid table");
    let (machine, mut slot) = entered(&table);
    let ready = run(&machine, &table, &slot, intent(2, "ready"));
    slot = commit(&mut table, &slot, ready, 1);
    host_removes(&mut table, 2, 50);
    let public = machine
        .project(&table, &slot, Audience::Public)
        .expect("projects");
    assert_eq!(public.as_str(), "0 of 1 seats are ready.");
    let joined = run(&machine, &table, &slot, intent(3, "join"));
    assert_eq!(
        joined.table_effects,
        vec![TableEffect::SeatJoined { seat: seat(3) }],
        "a seat is free now, though the table was full when the lobby was entered"
    );
}

#[test]
fn a_started_lobby_accepts_nothing_and_a_start_with_nobody_seated_is_declined() {
    let mut table = table_with_two_seated();
    let (machine, mut slot) = entered(&table);
    for (turn, who) in [(1, 1), (2, 2)] {
        let ready = run(&machine, &table, &slot, intent(who, "ready"));
        slot = commit(&mut table, &slot, ready, turn);
    }
    let started = run(&machine, &table, &slot, intent(1, "start"));
    let slot = commit(&mut table, &slot, started, 3);
    assert!(!machine.accepts(&table, &intent(1, "join"), &slot));
    let empty = TableState::new(4).expect("valid table");
    let (machine, slot) = entered(&empty);
    let nobody = run(&machine, &empty, &slot, intent(1, "start"));
    assert_eq!(declined_code(&nobody), Some(DeclineCode::NotViable));
}

#[test]
fn the_lobbys_effects_apply_to_the_table_it_ran_against() {
    let table = table_with_two_seated();
    let (machine, slot) = entered(&table);
    let joined = run(&machine, &table, &slot, intent(3, "join"));
    let mut table = table;
    let scope = TableScope::of_table(TableId::new(1));
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
    assert_eq!(spec.unseated_actions(), [action("join")]);
    assert!(!spec.reads_entry());
    let names: Vec<&str> = spec.states().iter().map(StateName::as_str).collect();
    assert_eq!(names, ["gathering", "started"]);
}

#[test]
fn each_call_decodes_the_state_at_most_once_and_the_calls_that_only_name_it_decode_nothing() {
    let table = table_with_two_seated();
    let (machine, slot) = entered(&table);
    let decodes = |call: &dyn Fn()| {
        let before = decodes_on_this_thread();
        call();
        decodes_on_this_thread() - before
    };
    assert_eq!(decodes(&|| drop(machine.state_name(&slot))), 0);
    assert_eq!(decodes(&|| drop(machine.wants_hand_off(&slot))), 0);
    assert_eq!(decodes(&|| drop(machine.inspect(&slot))), 0);
    assert_eq!(
        decodes(&|| {
            let _accepted = machine.accepts(&table, &intent(1, "ready"), &slot);
        }),
        1
    );
    assert_eq!(
        decodes(&|| drop(machine.step(&slot, &step_input(&table, intent(1, "ready"))))),
        1
    );
    assert_eq!(
        decodes(&|| drop(machine.step(&slot, &step_input(&table, intent(1, "start"))))),
        1
    );
    assert_eq!(
        decodes(&|| drop(machine.project(&table, &slot, Audience::Public))),
        1
    );
    assert_eq!(decodes(&|| drop(machine.exit(&table, &slot))), 1);
}

#[test]
fn a_decline_hands_back_no_copy_of_the_state() {
    let table = table_with_two_seated();
    let (machine, slot) = entered(&table);
    let refused = run(&machine, &table, &slot, intent(1, "start"));
    assert!(refused.state.is_none());
    assert_eq!(refused.state_after(&slot), &slot);
}

#[test]
fn a_seated_seat_in_the_gathering_lobby_is_offered_ready_start_and_leave_and_a_stranger_nothing() {
    let table = table_with_two_seated();
    let (machine, slot) = entered(&table);
    let names = |offers: Vec<ActionId>| -> Vec<String> {
        offers
            .iter()
            .map(|offer| offer.as_str().to_owned())
            .collect()
    };
    assert_eq!(
        names(offered(&machine, &table, &slot, seat(1))),
        ["ready", "start", "leave"]
    );
    assert!(offered(&machine, &table, &slot, seat(9)).is_empty());
    let ready = run(&machine, &table, &slot, intent(1, "ready"));
    let after = ready.state_after(&slot).clone();
    assert_eq!(
        names(offered(&machine, &table, &after, seat(1))),
        ["unready", "start", "leave"]
    );
    assert_eq!(
        names(offered(&machine, &table, &after, seat(2))),
        ["ready", "start", "leave"]
    );
}

#[test]
fn the_lobby_builds_the_name_of_every_action_once_and_offers_exactly_those_names() {
    assert!(LobbyMachine::new(FIRST_SCENE).is_ok());
    let table = table_with_two_seated();
    let (machine, slot) = entered(&table);
    let offers = offered(&machine, &table, &slot, seat(1));
    let named: Vec<&str> = offers.iter().map(|offer| offer.as_str()).collect();
    assert_eq!(
        named,
        [
            LobbyAction::Ready.key(),
            LobbyAction::Start.key(),
            LobbyAction::Leave.key()
        ]
    );
}
