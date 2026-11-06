//! Standing offers across the three scene machines: every button offered is one the machine
//! accepts, each machine reads its state once for all the seats, and the offered set of a machine
//! whose buttons are its rules actions is exactly the accepted set (H3).

use crate::dialogue::{Conversation, DialogueMachine};
use crate::effects::{TableEffect, TableState};
use crate::envelope::PendingId;
use crate::explore::{AuthoredBeat, ExploreMachine};
use crate::identity::{SeatId, TableId, TableScope};
use crate::lobby::LobbyMachine;
use crate::machine::slot::{MachineSlot, decodes_on_this_thread};
use crate::machine::{CarryItem, Decision, Entry, Machine, MachineError, MachineInput, StepInput};
use crate::ports::{ActionId, Resolution, Resolved};
use crate::testkit::correlation;

use crate::lobby_tests::{FIRST_SCENE, action, seat};

const SEATS: u64 = 6;

fn seats() -> Vec<SeatId> {
    (1..=SEATS).map(seat).collect()
}

/// A table of six seats in the first scene.
fn table() -> TableState {
    let mut table = TableState::seated(8, &seats()).expect("a valid table");
    table
        .apply(
            &TableScope::of_table(TableId::new(1)),
            correlation(100),
            &[TableEffect::SceneEntered { scene: FIRST_SCENE }],
        )
        .expect("the scene is entered");
    table
}

fn entry() -> Entry {
    let seated = seats().into_iter().map(CarryItem::Seat);
    Entry::carrying(
        std::iter::once(CarryItem::Scene(FIRST_SCENE))
            .chain(seated)
            .collect(),
    )
}

fn intent(who: u64, name: &str) -> MachineInput {
    MachineInput::Intent {
        seat: seat(who),
        action: action(name),
    }
}

/// The state after `input` was stepped with `decision`; a decline leaves the state as it was.
fn advanced(
    machine: &dyn Machine,
    table: &TableState,
    slot: &MachineSlot,
    input: MachineInput,
    decision: Option<Decision>,
) -> MachineSlot {
    let result = machine
        .step(
            slot,
            &StepInput {
                correlation: correlation(1),
                now_ms: 0,
                table,
                input,
                decision,
            },
        )
        .expect("steps");
    result.state_after(slot).clone()
}

fn applied() -> Option<Decision> {
    Some(Decision::Resolved(Resolution::Applied(
        Resolved::default()
            .with_fact("succeeded", 1)
            .expect("a valid label"),
    )))
}

fn waiting() -> Option<Decision> {
    Some(Decision::RollPending(PendingId::new(5)))
}

/// One machine with the states a script reaches, each named.
struct Scripted {
    machine: Box<dyn Machine>,
    states: Vec<(&'static str, MachineSlot)>,
}

fn lobby() -> Scripted {
    let table = table();
    let machine = LobbyMachine::new(FIRST_SCENE).expect("the lobby builds");
    let gathering = machine.enter(&table, &Entry::empty()).expect("enters");
    let one_ready = advanced(&machine, &table, &gathering, intent(1, "ready"), None);
    let mut all_ready = one_ready.clone();
    for who in 2..=SEATS {
        all_ready = advanced(&machine, &table, &all_ready, intent(who, "ready"), None);
    }
    let started = advanced(&machine, &table, &all_ready, intent(1, "start"), None);
    Scripted {
        machine: Box::new(machine),
        states: vec![
            ("gathering", gathering),
            ("one ready", one_ready),
            ("all ready", all_ready),
            ("started", started),
        ],
    }
}

fn explore() -> Scripted {
    let table = table();
    let beat = AuthoredBeat::new("locked_gate", "investigate").expect("a valid beat");
    let machine = ExploreMachine::new(beat).expect("the explore builds");
    let presenting = machine.enter(&table, &entry()).expect("enters");
    let check = || intent(1, "investigate");
    let awaiting = advanced(&machine, &table, &presenting, check(), waiting());
    let resolved = advanced(&machine, &table, &presenting, check(), applied());
    Scripted {
        machine: Box::new(machine),
        states: vec![
            ("presenting", presenting),
            ("awaiting roll", awaiting),
            ("resolved", resolved),
        ],
    }
}

fn dialogue() -> Scripted {
    let table = table();
    let conversation = Conversation::new(
        "The gate guard",
        "Halt.",
        "persuade_guard",
        "persuade the guard",
        "Fine.",
        "No.",
    )
    .expect("the conversation is valid");
    let machine = DialogueMachine::new(conversation).expect("the dialogue builds");
    let listening = machine.enter(&table, &entry()).expect("enters");
    let urge = || intent(1, "persuade_guard");
    let awaiting = advanced(&machine, &table, &listening, urge(), waiting());
    let concluded = advanced(&machine, &table, &listening, urge(), applied());
    Scripted {
        machine: Box::new(machine),
        states: vec![
            ("listening", listening),
            ("awaiting roll", awaiting),
            ("concluded", concluded),
        ],
    }
}

fn every_machine() -> Vec<(&'static str, Scripted)> {
    vec![
        ("lobby", lobby()),
        ("explore", explore()),
        ("dialogue", dialogue()),
    ]
}

fn accepts(
    machine: &dyn Machine,
    table: &TableState,
    slot: &MachineSlot,
    who: SeatId,
    action: &ActionId,
) -> bool {
    let input = MachineInput::Intent {
        seat: who,
        action: action.clone(),
    };
    machine.accepts(table, &input, slot)
}

#[test]
fn every_standing_offer_of_every_seat_in_every_scripted_state_is_accepted_by_the_machine() {
    let table = table();
    let mut asked = seats();
    asked.push(seat(9));
    for (name, scripted) in every_machine() {
        for (state, slot) in &scripted.states {
            let offers = scripted
                .machine
                .standing_offers(&table, slot, &asked)
                .unwrap_or_else(|error| panic!("{name} {state}: {error}"));
            let named: Vec<SeatId> = offers.iter().map(|(who, _)| *who).collect();
            assert_eq!(
                named, asked,
                "{name} {state}: one answer per seat asked, in order"
            );
            for (who, actions) in &offers {
                for offered in actions {
                    assert!(
                        accepts(scripted.machine.as_ref(), &table, slot, *who, offered),
                        "{name} {state}: seat {who} is offered {offered:?} and the machine declines it"
                    );
                }
            }
            let stranger = offers.last().map(|(_, actions)| actions.len());
            assert_eq!(
                stranger,
                Some(0),
                "{name} {state}: a seat not at the table is offered nothing"
            );
        }
    }
}

#[test]
fn a_machine_whose_buttons_are_its_rules_actions_offers_exactly_the_ones_it_accepts() {
    let table = table();
    for (name, scripted) in every_machine()
        .into_iter()
        .filter(|(name, _)| *name != "lobby")
    {
        let rules = scripted.machine.spec().rules_actions().to_vec();
        for (state, slot) in &scripted.states {
            let offers = scripted
                .machine
                .standing_offers(&table, slot, &seats())
                .expect("the state reads");
            for (who, actions) in offers {
                let accepted: Vec<ActionId> = rules
                    .iter()
                    .filter(|rule| accepts(scripted.machine.as_ref(), &table, slot, who, rule))
                    .cloned()
                    .collect();
                assert_eq!(actions, accepted, "{name} {state}: seat {who}");
            }
        }
    }
}

#[test]
fn a_machine_reads_its_state_once_for_all_six_seats() {
    let table = table();
    for (name, scripted) in every_machine() {
        for (state, slot) in &scripted.states {
            let before = decodes_on_this_thread();
            let offers = scripted.machine.standing_offers(&table, slot, &seats());
            let decodes = decodes_on_this_thread() - before;
            assert!(offers.is_ok(), "{name} {state}");
            assert_eq!(decodes, 1, "{name} {state}");
        }
    }
}

#[test]
fn the_default_derives_the_offers_from_the_spec_and_refuses_a_slot_it_cannot_read() {
    // A machine that overrides nothing: the golden scene machine offers each rules action it
    // accepts, and a corrupt record is the typed error and not an empty list.
    let table = table();
    let scene = crate::golden::SceneMachine::new(&[
        ("wave", crate::golden::Leaves::Nothing),
        ("bow", crate::golden::Leaves::Nothing),
    ])
    .expect("the scene builds");
    let slot = scene.enter(&table, &Entry::empty()).expect("enters");

    let offers = scene
        .standing_offers(&table, &slot, &[seat(1), seat(9)])
        .expect("the state reads");

    assert_eq!(
        offers,
        [
            (seat(1), vec![action("wave"), action("bow")]),
            (seat(9), Vec::new())
        ]
    );
    let corrupt = MachineSlot::from_parts(
        slot.machine().clone(),
        slot.schema_version(),
        serde_json::json!({ "phase": "bogus" }),
    );
    assert!(matches!(
        scene.standing_offers(&table, &corrupt, &[seat(1)]),
        Err(MachineError::Load(_))
    ));
}
