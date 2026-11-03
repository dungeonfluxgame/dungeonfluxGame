use std::num::NonZeroU16;
use std::sync::Arc;

use crate::conformance::{self, ConformanceFixture, Expectation, ScriptedStep};
use crate::dialogue::{Conversation, ConversationError, DialogueMachine};
use crate::effects::TableState;
use crate::envelope::{PendingId, TimerName};
use crate::explore::{AuthoredBeat, ExploreMachine};
use crate::explore_tests::{entry_to_the_first_scene, table_in_the_first_scene};
use crate::lobby_tests::{FIRST_SCENE, action, lobby, seat};
use crate::machine::{
    CarryItem, Decision, Entry, EntryRefusal, Machine, MachineError, MachineInput, StepInput,
    StepOutcome, StepResult,
};
use crate::machine_slot::MachineSlot;
use crate::outcome::{Decline, DeclineCode, Outcome};
use crate::ports::{Audience, Draw, DrawRequest, PortName, Preparation, Resolution, Resolved};
use crate::testkit::{QueuedDice, ScriptedBroker, ScriptedRulesOwner, correlation};

const URGE: &str = "persuade_guard";

fn conversation() -> Conversation {
    Conversation::new(
        "The gate guard",
        "Halt. Who goes there?",
        URGE,
        "Fine. Go through.",
        "Not a chance.",
    )
    .expect("the conversation is valid")
}

pub(crate) fn dialogue() -> DialogueMachine {
    DialogueMachine::new(conversation()).expect("the dialogue spec is valid")
}

fn d20() -> DrawRequest {
    DrawRequest {
        sides: NonZeroU16::new(20).expect("a die has sides"),
    }
}

fn resolved_with(label: &str, amount: u64) -> Resolution {
    Resolution::Applied(
        Resolved::default()
            .with_fact(label, amount)
            .expect("a valid label"),
    )
}

/// Explore handing on to the dialogue, which is how the first playable path chains them.
fn explore_then_dialogue() -> ExploreMachine {
    let beat = AuthoredBeat::new("locked_gate", "investigate").expect("valid beat");
    ExploreMachine::followed_by(beat, "dialogue").expect("the explore spec is valid")
}

fn dialogue_fixture() -> ConformanceFixture {
    let intent =
        |who: u64, name: &str, expect| ScriptedStep::player(seat(who), action(name), expect);
    let rules = ScriptedRulesOwner::default().with_action(
        action(URGE),
        Preparation::Ready { draws: vec![d20()] },
        resolved_with("succeeded", 1),
    );
    ConformanceFixture {
        machine: Arc::new(dialogue()),
        neighbours: vec![Arc::new(lobby()), Arc::new(explore_then_dialogue())],
        wired: vec![PortName::RulesOwner],
        table: table_in_the_first_scene(),
        entry: entry_to_the_first_scene(),
        refused_entry: Some(Entry::empty()),
        script: vec![
            intent(9, URGE, Expectation::refused()),
            intent(
                1,
                URGE,
                Expectation::waiting().moving("listening", "awaiting_roll"),
            ),
            ScriptedStep::raw(
                MachineInput::Timer {
                    timer: TimerName::new("idle_nudge").expect("valid timer"),
                },
                Expectation::refused(),
            ),
            intent(2, URGE, Expectation::refused()),
            ScriptedStep::answering_open_roll(
                Expectation::applied()
                    .moving("awaiting_roll", "concluded")
                    .handing_off(),
            ),
            intent(1, URGE, Expectation::refused().handing_off()),
        ],
        rules,
        dice: QueuedDice::new([14]),
        broker: ScriptedBroker::default(),
    }
}

fn entered() -> (DialogueMachine, MachineSlot, TableState) {
    let machine = dialogue();
    let table = table_in_the_first_scene();
    let slot = machine
        .enter(&table, &entry_to_the_first_scene())
        .expect("enters");
    (machine, slot, table)
}

fn urge(who: u64) -> MachineInput {
    MachineInput::Intent {
        seat: seat(who),
        action: action(URGE),
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

fn concluded_by(resolution: Resolution) -> (DialogueMachine, MachineSlot, TableState) {
    let (machine, slot, table) = entered();
    let result = machine
        .step(
            &slot,
            &with(&table, urge(1), Some(Decision::Resolved(resolution))),
        )
        .expect("steps");
    assert_eq!(result.outcome, StepOutcome::Settled(Outcome::Applied));
    let done = result.state.expect("a conclusion is a new state");
    (machine, done, table)
}

fn readout(machine: &DialogueMachine, slot: &MachineSlot, table: &TableState) -> String {
    machine
        .project(table, slot, Audience::Public)
        .expect("projects")
        .as_str()
        .to_owned()
}

fn declined_code(result: &StepResult) -> Option<DeclineCode> {
    match &result.outcome {
        StepOutcome::Settled(Outcome::Declined(decline)) => Some(decline.code()),
        _ => None,
    }
}

#[test]
fn dialogue_passes_the_whole_conformance_suite() {
    let report = conformance::run(dialogue_fixture());
    assert!(report.is_clean(), "{:#?}", report.findings());
    assert!(report.skipped().is_empty());
}

#[test]
fn a_conversation_with_a_blank_line_or_a_bad_urging_is_refused_with_the_field_named() {
    let build = |speaker, opening, urge, moved, unmoved| {
        Conversation::new(speaker, opening, urge, moved, unmoved)
    };
    assert_eq!(
        build(" ", "Hi.", URGE, "Yes.", "No."),
        Err(ConversationError::Blank("speaker"))
    );
    assert_eq!(
        build("Guard", "", URGE, "Yes.", "No."),
        Err(ConversationError::Blank("opening line"))
    );
    assert_eq!(
        build("Guard", "Hi.", URGE, "  ", "No."),
        Err(ConversationError::Blank("line when moved"))
    );
    assert_eq!(
        build("Guard", "Hi.", URGE, "Yes.", ""),
        Err(ConversationError::Blank("line when unmoved"))
    );
    assert!(matches!(
        build("Guard", "Hi.", "Not A Name!", "Yes.", "No."),
        Err(ConversationError::InvalidAction(_))
    ));
}

#[test]
fn entering_needs_a_scene_the_table_is_in_and_seats_it_holds() {
    let machine = dialogue();
    let table = table_in_the_first_scene();
    assert_eq!(
        machine.enter(&table, &Entry::empty()),
        Err(MachineError::Entry(EntryRefusal::MissingScene))
    );
    assert_eq!(
        machine.enter(
            &table,
            &Entry::carrying(vec![CarryItem::Scene(FIRST_SCENE)])
        ),
        Err(MachineError::Entry(EntryRefusal::NoSeats))
    );
    assert_eq!(
        machine.enter(
            &table,
            &Entry::carrying(vec![
                CarryItem::Scene(FIRST_SCENE),
                CarryItem::Seat(seat(9))
            ])
        ),
        Err(MachineError::Entry(EntryRefusal::SeatNotAtTable {
            seat: seat(9)
        }))
    );
}

#[test]
fn only_a_seat_at_the_table_can_urge_and_only_with_the_authored_action() {
    let (machine, slot, table) = entered();
    assert!(machine.accepts(&table, &urge(1), &slot));
    assert!(!machine.accepts(&table, &urge(9), &slot));
    let other = MachineInput::Intent {
        seat: seat(1),
        action: action("attack"),
    };
    assert!(!machine.accepts(&table, &other, &slot));
}

#[test]
fn a_roll_decision_makes_the_machine_wait_on_that_pending_and_hold_only_its_id() {
    let (machine, slot, table) = entered();
    let pending = PendingId::new(5);
    let waiting = machine
        .step(
            &slot,
            &with(&table, urge(1), Some(Decision::RollPending(pending))),
        )
        .expect("steps");
    assert_eq!(waiting.outcome, StepOutcome::Waiting(pending));
    let state = waiting.state.expect("the pending is recorded");
    let dice = MachineInput::Dice {
        pending,
        draws: vec![Draw::new(NonZeroU16::new(20).expect("sides"), 14).expect("on the die")],
    };
    assert!(machine.accepts(&table, &dice, &state));
    assert!(!machine.accepts(&table, &urge(1), &state));
    let cancelled = machine
        .step(
            &state,
            &with(&table, MachineInput::PendingCancelled { pending }, None),
        )
        .expect("steps");
    assert_eq!(cancelled.outcome, StepOutcome::Settled(Outcome::Applied));
    assert_eq!(
        machine
            .state_name(&cancelled.state.expect("back to listening"))
            .expect("named")
            .as_str(),
        "listening"
    );
}

#[test]
fn the_typed_facts_of_the_rules_result_decide_how_the_conversation_ends() {
    let cases = [
        (resolved_with("succeeded", 1), "Fine. Go through."),
        (resolved_with("succeeded", 0), "Not a chance."),
        (resolved_with("complied", 1), "Fine. Go through."),
        (resolved_with("complied", 0), "Not a chance."),
        (Resolution::applied(), "has heard you out"),
    ];
    for (resolution, expected) in cases {
        let (machine, done, table) = concluded_by(resolution.clone());
        let line = readout(&machine, &done, &table);
        assert!(line.contains(expected), "{resolution:?} gave {line}");
        assert!(line.contains("The gate guard"), "{line}");
    }
}

#[test]
fn a_concluded_conversation_asks_to_hand_back_to_the_lobby_with_the_scene_and_the_seats() {
    let (machine, done, table) = concluded_by(resolved_with("succeeded", 1));
    assert!(machine.wants_hand_off(&done).expect("asks"));
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
    assert!(!machine.accepts(&table, &urge(1), &done));
}

#[test]
fn a_decline_from_the_rules_owner_changes_nothing() {
    let (machine, slot, table) = entered();
    let decline = Decline::with_default_reason(DeclineCode::MissingRequirement);
    let refused = machine
        .step(
            &slot,
            &with(
                &table,
                urge(1),
                Some(Decision::Resolved(Resolution::Declined(decline))),
            ),
        )
        .expect("steps");
    assert_eq!(
        declined_code(&refused),
        Some(DeclineCode::MissingRequirement)
    );
    assert!(refused.state.is_none() && refused.table_effects.is_empty());
}

#[test]
fn standing_offers_name_the_urging_only_while_listening_and_only_for_a_seated_seat() {
    let (machine, slot, table) = entered();
    assert_eq!(
        machine.standing_offers(&table, &slot, seat(1)),
        [action(URGE)]
    );
    assert!(machine.standing_offers(&table, &slot, seat(9)).is_empty());
    let (machine, done, table) = concluded_by(resolved_with("succeeded", 1));
    assert!(machine.standing_offers(&table, &done, seat(1)).is_empty());
}

#[test]
fn the_phone_reads_the_opening_line_and_the_urging_and_the_tv_reads_the_line_alone() {
    let (machine, slot, table) = entered();
    let phone = machine
        .project(&table, &slot, Audience::Seat(seat(1)))
        .expect("projects");
    assert_eq!(
        phone.as_str(),
        "The gate guard: \"Halt. Who goes there?\" You can try to persuade guard."
    );
    assert_eq!(
        readout(&machine, &slot, &table),
        "The gate guard: \"Halt. Who goes there?\""
    );
}
