use std::num::NonZeroU16;
use std::sync::Arc;

use serde_json::json;

use crate::conformance::{self, ConformanceFixture, Expectation, ScriptedStep};
use crate::dialogue::{Conversation, ConversationError, DialogueMachine};
use crate::effects::{
    ApplyOutcome, ConversationOutcome, EffectCause, EffectRecord, TableEffect, TableState,
};
use crate::envelope::{PendingId, TimerName};
use crate::explore::{AuthoredBeat, ExploreMachine};
use crate::explore_tests::{entry_to_the_first_scene, table_in_the_first_scene};
use crate::identity::{MachineId, SceneId, TableId, TableScope};
use crate::lobby_tests::{FIRST_SCENE, action, lobby, offered, seat};
use crate::machine::slot::MachineSlot;
use crate::machine::{
    CarryItem, Decision, Entry, EntryError, Machine, MachineError, MachineFailureCause,
    MachineInput, ResultCause, StepInput, StepOutcome, StepResult,
};
use crate::outcome::{Decline, DeclineCode, Outcome};
use crate::ports::{Audience, Draw, DrawRequest, PortName, Preparation, Resolution, Resolved};
use crate::testkit::{QueuedDice, ScriptedBroker, ScriptedRulesOwner, correlation};

const URGE: &str = "persuade_guard";
const URGE_LABEL: &str = "persuade the guard";

fn conversation() -> Conversation {
    Conversation::new(
        "The gate guard",
        "Halt. Who goes there?",
        URGE,
        URGE_LABEL,
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
        Conversation::new(speaker, opening, urge, URGE_LABEL, moved, unmoved)
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
fn a_blank_urging_label_is_refused_with_the_field_named() {
    for blank in ["", "   "] {
        assert_eq!(
            Conversation::new("Guard", "Hi.", URGE, blank, "Yes.", "No."),
            Err(ConversationError::Blank("urging label"))
        );
    }
}

#[test]
fn entering_needs_a_scene_the_table_is_in_and_seats_it_holds() {
    let machine = dialogue();
    let table = table_in_the_first_scene();
    assert_eq!(
        machine.enter(&table, &Entry::empty()),
        Err(MachineError::Entry(EntryError::MissingScene))
    );
    assert_eq!(
        machine.enter(
            &table,
            &Entry::carrying(vec![CarryItem::Scene(FIRST_SCENE)])
        ),
        Err(MachineError::Entry(EntryError::NoSeats))
    );
    assert_eq!(
        machine.enter(
            &table,
            &Entry::carrying(vec![
                CarryItem::Scene(FIRST_SCENE),
                CarryItem::Seat(seat(9))
            ])
        ),
        Err(MachineError::Entry(EntryError::SeatNotAtTable {
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
    assert_eq!(offered(&machine, &table, &slot, seat(1)), [action(URGE)]);
    assert!(offered(&machine, &table, &slot, seat(9)).is_empty());
    let (machine, done, table) = concluded_by(resolved_with("succeeded", 1));
    assert!(offered(&machine, &table, &done, seat(1)).is_empty());
}

#[test]
fn the_phone_reads_the_opening_line_and_the_urging_and_the_tv_reads_the_line_alone() {
    let (machine, slot, table) = entered();
    let phone = machine
        .project(&table, &slot, Audience::Seat(seat(1)))
        .expect("projects");
    assert_eq!(
        phone.as_str(),
        "The gate guard: \"Halt. Who goes there?\" You can try to persuade the guard."
    );
    assert_eq!(
        readout(&machine, &slot, &table),
        "The gate guard: \"Halt. Who goes there?\""
    );
}

fn waiting_on(pending: u64) -> (DialogueMachine, MachineSlot, TableState) {
    let (machine, slot, table) = entered();
    let waiting = machine
        .step(
            &slot,
            &with(
                &table,
                urge(1),
                Some(Decision::RollPending(PendingId::new(pending))),
            ),
        )
        .expect("steps");
    let state = waiting.state.expect("the pending is recorded");
    (machine, state, table)
}

fn dice_for(pending: u64) -> MachineInput {
    MachineInput::Dice {
        pending: PendingId::new(pending),
        draws: vec![Draw::new(NonZeroU16::new(20).expect("sides"), 14).expect("on the die")],
    }
}

fn scope() -> TableScope {
    TableScope::of_table(TableId::new(1))
}

fn resolved_facts(facts: &[(&str, u64)]) -> Resolution {
    let mut resolved = Resolved::default();
    for (label, amount) in facts {
        resolved = resolved.with_fact(label, *amount).expect("a valid label");
    }
    Resolution::Applied(resolved)
}

fn urged(
    machine: &DialogueMachine,
    slot: &MachineSlot,
    table: &TableState,
    resolution: Resolution,
) -> Result<StepResult, MachineError> {
    machine.step(
        slot,
        &with(table, urge(1), Some(Decision::Resolved(resolution))),
    )
}

#[test]
fn a_dice_answer_or_a_cancel_for_another_pending_is_declined_and_leaves_the_slot_alone() {
    let (machine, state, table) = waiting_on(5);
    let answers = [
        dice_for(6),
        MachineInput::PendingCancelled {
            pending: PendingId::new(6),
        },
    ];
    for (which, input) in answers.into_iter().enumerate() {
        assert!(!machine.accepts(&table, &input, &state), "answer {which}");
        let decision = Some(Decision::Resolved(resolved_with("succeeded", 1)));
        let result = machine
            .step(&state, &with(&table, input, decision))
            .expect("steps");
        assert_eq!(
            declined_code(&result),
            Some(DeclineCode::NotViable),
            "answer {which}"
        );
        assert!(result.state.is_none() && result.table_effects.is_empty());
    }
}

#[test]
fn a_second_roll_decision_while_a_roll_is_awaited_is_declined_and_never_re_arms_the_pending() {
    let (machine, state, table) = waiting_on(5);
    assert!(!machine.accepts(&table, &urge(1), &state));
    let again = machine
        .step(
            &state,
            &with(
                &table,
                urge(1),
                Some(Decision::RollPending(PendingId::new(6))),
            ),
        )
        .expect("steps");
    assert_eq!(declined_code(&again), Some(DeclineCode::NotViable));
    assert!(again.state.is_none() && again.table_effects.is_empty());
    assert_eq!(state.payload()["pending"], 5);
}

#[test]
fn a_declined_resolution_on_the_dice_answer_leaves_the_roll_awaited_on_the_same_pending() {
    let (machine, state, table) = waiting_on(5);
    let decline = Decline::with_default_reason(DeclineCode::MissingRequirement);
    let refused = machine
        .step(
            &state,
            &with(
                &table,
                dice_for(5),
                Some(Decision::Resolved(Resolution::Declined(decline))),
            ),
        )
        .expect("steps");
    assert_eq!(
        declined_code(&refused),
        Some(DeclineCode::MissingRequirement)
    );
    assert!(refused.state.is_none() && refused.table_effects.is_empty());
    assert!(machine.accepts(&table, &dice_for(5), refused.state_after(&state)));
    assert_eq!(
        machine
            .state_name(refused.state_after(&state))
            .expect("named")
            .as_str(),
        "awaiting_roll"
    );
}

#[test]
fn a_seat_that_left_the_table_after_entry_cannot_urge_is_offered_nothing_and_is_not_carried_out() {
    let (machine, slot, table) = entered();
    let mut smaller = table.clone();
    smaller
        .apply(
            &scope(),
            correlation(200),
            &[TableEffect::SeatLeft { seat: seat(2) }],
        )
        .expect("the seat leaves");
    assert!(machine.accepts(&smaller, &urge(1), &slot));
    assert!(!machine.accepts(&smaller, &urge(2), &slot));
    assert_eq!(offered(&machine, &smaller, &slot, seat(1)), [action(URGE)]);
    assert!(offered(&machine, &smaller, &slot, seat(2)).is_empty());
    let (machine, done, _) = concluded_by(resolved_with("succeeded", 1));
    let hand_off = machine.exit(&smaller, &done).expect("exits");
    assert_eq!(
        hand_off.carry(),
        [CarryItem::Scene(FIRST_SCENE), CarryItem::Seat(seat(1))]
    );
}

#[test]
fn a_corrupt_slot_gives_the_typed_error_or_no_button_and_never_a_panic() {
    let (machine, _, table) = entered();
    let dialogue_id = MachineId::new("dialogue").expect("a machine id");
    let corrupt = [
        (
            MachineSlot::from_parts(dialogue_id.clone(), 1, json!({ "phase": "listening" })),
            MachineFailureCause::SlotMalformed,
        ),
        (
            MachineSlot::from_parts(dialogue_id.clone(), 1, json!({ "phase": "bogus" })),
            MachineFailureCause::SlotMalformed,
        ),
        (
            MachineSlot::from_parts(dialogue_id, 9, json!({ "phase": "listening" })),
            MachineFailureCause::SlotUnsupportedVersion,
        ),
        (
            MachineSlot::from_parts(
                MachineId::new("lobby").expect("a machine id"),
                1,
                json!({ "phase": "gathering", "ready": [] }),
            ),
            MachineFailureCause::SlotWrongMachine,
        ),
    ];
    for (slot, cause) in corrupt {
        let projected = machine.project(&table, &slot, Audience::Public);
        assert_eq!(projected.map_err(|error| error.cause()), Err(cause));
        assert!(!machine.accepts(&table, &urge(1), &slot));
        let offers = machine.standing_offers(&table, &slot, &[seat(1), seat(2)]);
        assert_eq!(offers.map_err(|error| error.cause()), Err(cause));
    }
}

#[test]
fn the_waiting_result_carries_exactly_the_pending_it_was_given() {
    let (machine, slot, table) = entered();
    for given in [1, 5, u64::MAX] {
        let pending = PendingId::new(given);
        let result = machine
            .step(
                &slot,
                &with(&table, urge(1), Some(Decision::RollPending(pending))),
            )
            .expect("steps");
        assert_eq!(result.outcome, StepOutcome::Waiting(pending));
        let state = result.state.expect("the pending is recorded");
        assert_eq!(state.payload()["phase"], "awaiting_roll");
        assert_eq!(state.payload()["pending"], given);
    }
}

type Facts<'a> = &'a [(&'a str, u64)];

#[test]
fn a_rules_result_is_read_as_a_verdict_or_refused_with_a_typed_reason() {
    let (machine, slot, table) = entered();
    let moved = Some(ConversationOutcome::Moved);
    let unmoved = Some(ConversationOutcome::Unmoved);
    let cases: [(Facts, Result<Option<ConversationOutcome>, ResultCause>); 12] = [
        (&[("succeeded", 1)], Ok(moved)),
        (&[("succeeded", 0)], Ok(unmoved)),
        (&[("complied", 1)], Ok(moved)),
        (&[("complied", 0)], Ok(unmoved)),
        (&[("succeeded", 1), ("complied", 1)], Ok(moved)),
        (&[("succeeded", 0), ("complied", 0)], Ok(unmoved)),
        (&[], Ok(None)),
        (&[("damage", 7)], Ok(None)),
        (&[("succeeded", 2)], Err(ResultCause::AmountNotZeroOrOne)),
        (
            &[("complied", u64::MAX)],
            Err(ResultCause::AmountNotZeroOrOne),
        ),
        (
            &[("succeeded", 0), ("complied", 1)],
            Err(ResultCause::FactsConflict),
        ),
        (
            &[("succeeded", 1), ("succeeded", 1)],
            Err(ResultCause::DuplicateFact),
        ),
    ];
    for (facts, wanted) in cases {
        let result = urged(&machine, &slot, &table, resolved_facts(facts));
        match wanted {
            Ok(Some(outcome)) => {
                let applied = result.unwrap_or_else(|error| panic!("{facts:?} gave {error}"));
                assert_eq!(
                    applied.table_effects,
                    [TableEffect::ConversationEnded {
                        scene: FIRST_SCENE,
                        outcome
                    }],
                    "{facts:?}"
                );
            }
            Ok(None) => {
                let declined = result.expect("a result with no verdict is declined, not failed");
                assert_eq!(
                    declined_code(&declined),
                    Some(DeclineCode::NotSupportedYet),
                    "{facts:?}"
                );
                assert!(declined.state.is_none() && declined.table_effects.is_empty());
            }
            Err(fault) => assert_eq!(result, Err(MachineError::Result(fault)), "{facts:?}"),
        }
    }
}

#[test]
fn a_result_with_no_verdict_leaves_the_urging_open_to_be_tried_again() {
    let (machine, slot, table) = entered();
    let declined = urged(&machine, &slot, &table, Resolution::applied()).expect("steps");
    assert!(declined.state.is_none());
    assert!(machine.accepts(&table, &urge(1), &slot));
    assert!(!machine.wants_hand_off(&slot).expect("asks"));
    let retried = urged(&machine, &slot, &table, resolved_with("succeeded", 1)).expect("steps");
    assert_eq!(retried.outcome, StepOutcome::Settled(Outcome::Applied));
}

#[test]
fn the_end_of_the_conversation_is_a_table_effect_the_events_and_a_checkpoint_keep() {
    for (amount, outcome) in [
        (1, ConversationOutcome::Moved),
        (0, ConversationOutcome::Unmoved),
    ] {
        let (machine, slot, table) = entered();
        let result =
            urged(&machine, &slot, &table, resolved_with("succeeded", amount)).expect("steps");
        let ended = TableEffect::ConversationEnded {
            scene: FIRST_SCENE,
            outcome,
        };
        assert_eq!(result.table_effects, std::slice::from_ref(&ended));

        let mut live = table.clone();
        assert_eq!(live.active_conversation(), None);
        let applied = live
            .apply(&scope(), correlation(300), &result.table_effects)
            .expect("applies");
        let ApplyOutcome::Applied(events) = applied else {
            unreachable!("a new operation is applied");
        };
        let lines: Vec<String> = events
            .iter()
            .map(|event| event.render_audit_line())
            .collect();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("effect=conversation_ended")),
            "{lines:?}"
        );
        assert_eq!(live.active_conversation(), Some(outcome));
        assert_ne!(live.digest_without_stamps(), table.digest_without_stamps());

        let saved =
            serde_json::to_string(&EffectRecord::stamp(&ended).expect("stamps")).expect("saves");
        let record: EffectRecord = serde_json::from_str(&saved).expect("loads");
        let mut restored = table_in_the_first_scene();
        restored
            .apply(
                &scope(),
                correlation(300),
                &[record.open().expect("a record this engine reads")],
            )
            .expect("replays");
        assert_eq!(restored.active_conversation(), Some(outcome));
        assert_eq!(
            restored.digest_without_stamps(),
            live.digest_without_stamps()
        );
    }
}

#[test]
fn a_conversation_that_ended_in_a_scene_that_is_not_open_is_refused() {
    let mut table = table_in_the_first_scene();
    let refused = table.apply(
        &scope(),
        correlation(301),
        &[TableEffect::ConversationEnded {
            scene: SceneId::new(99),
            outcome: ConversationOutcome::Moved,
        }],
    );
    assert_eq!(
        refused.expect_err("refused").cause,
        EffectCause::SceneNotActive
    );
}
