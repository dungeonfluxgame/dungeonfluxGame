//! Shared fixtures for the pipeline and pending tests: a world with a scene machine, a rules
//! owner scripted per action, and readers for the events a run recorded.

use std::num::NonZeroU16;

use crate::event::{FailureCause, PipelineStep, TableEvent};
use crate::golden::{Leaves, World, compact};
use crate::identity::SeatId;
use crate::outcome::{Decline, DeclineCode};
use crate::pipeline::TurnResult;
use crate::ports::{
    ActionId, Awaiting, AwaitingKind, DrawRequest, Offer, PortName, Preparation, Resolution,
    RulingAnswer, RulingKey, RulingOption, RulingRequest, RulingScope, RulingTier,
};
use crate::testkit::ScriptedRulesOwner;

pub(crate) fn seat(number: u64) -> SeatId {
    SeatId::new(number)
}

pub(crate) fn action(name: &str) -> ActionId {
    ActionId::new(name).expect("the test action name is valid")
}

pub(crate) fn d20() -> DrawRequest {
    DrawRequest {
        sides: NonZeroU16::new(20).expect("20 is not zero"),
    }
}

pub(crate) fn d8() -> DrawRequest {
    DrawRequest {
        sides: NonZeroU16::new(8).expect("8 is not zero"),
    }
}

/// An action the rules owner allows at once, with no dice.
pub(crate) fn allows(rules: ScriptedRulesOwner, name: &str) -> ScriptedRulesOwner {
    rules.with_action(
        action(name),
        Preparation::Ready { draws: Vec::new() },
        Resolution::applied(),
    )
}

/// An action the rules owner declines with the code and its default reason.
pub(crate) fn refuses(
    rules: ScriptedRulesOwner,
    name: &str,
    code: DeclineCode,
) -> ScriptedRulesOwner {
    let decline = Decline::with_default_reason(code);
    rules.with_action(
        action(name),
        Preparation::Declined(decline.clone()),
        Resolution::Declined(decline),
    )
}

/// A world with a scene that takes these committing actions for `seats` seats.
pub(crate) fn world_of(names: &[&str], seats: u64, rules: ScriptedRulesOwner) -> World {
    let actions: Vec<(&str, Leaves)> = names.iter().map(|name| (*name, Leaves::Effect)).collect();
    let mut world = World::with_scene(&actions, seats, None).expect("the world builds");
    world.rules = rules;
    world
}

/// A world whose scene asks for a hand-off after `turns` turns.
pub(crate) fn finishing_world(
    names: &[&str],
    seats: u64,
    turns: u64,
    rules: ScriptedRulesOwner,
) -> World {
    let actions: Vec<(&str, Leaves)> = names.iter().map(|name| (*name, Leaves::Effect)).collect();
    let mut world = World::with_scene(&actions, seats, Some(turns)).expect("the world builds");
    world.rules = rules;
    world
}

/// The compact line of every event recorded so far.
pub(crate) fn lines(world: &World) -> Vec<String> {
    world.events().iter().map(compact).collect()
}

/// The compact lines of the events recorded after the first `seen`.
pub(crate) fn lines_after(world: &World, seen: usize) -> Vec<String> {
    world.events().iter().skip(seen).map(compact).collect()
}

pub(crate) fn steps_of(events: &[TableEvent]) -> Vec<PipelineStep> {
    events
        .iter()
        .filter_map(|event| match event {
            TableEvent::StepFinished { step, .. } => Some(*step),
            _ => None,
        })
        .collect()
}

pub(crate) fn failures_of(events: &[TableEvent]) -> Vec<(PortName, FailureCause)> {
    events
        .iter()
        .filter_map(|event| match event {
            TableEvent::BoundaryFailed { port, cause, .. } => Some((*port, *cause)),
            _ => None,
        })
        .collect()
}

pub(crate) fn ran(world: &mut World, seat_number: u64, name: &str) -> TurnResult {
    world.tap(seat_number, name).expect("the tap runs")
}

/// A reaction window of one seat that may take `choice` or pass.
pub(crate) fn window_of(seat_number: u64, choices: &[&str]) -> Awaiting {
    Awaiting {
        kind: AwaitingKind::Reaction,
        offers: vec![Offer {
            seat: seat(seat_number),
            choices: choices.iter().map(|name| action(name)).collect(),
        }],
        draws: Vec::new(),
    }
}

pub(crate) fn ruling_request(key: &str, suggested: &str, tier: RulingTier) -> RulingRequest {
    RulingRequest {
        key: RulingKey::new(key).expect("the key is valid"),
        scope: RulingScope::Scene,
        suggested: RulingOption {
            answer: RulingAnswer::new(suggested).expect("the answer is valid"),
            tier,
        },
        alternatives: Vec::new(),
        reviewed_automatic: false,
    }
}

/// The phase the running machine saved.
pub(crate) fn phase_of(world: &World) -> String {
    world
        .table
        .facts()
        .snapshot()
        .focus_machine()
        .and_then(|active| active.slot().payload()["phase"].as_str().map(str::to_owned))
        .expect("the machine has a phase")
}

/// The saved state of the running machine.
pub(crate) fn slot_of(world: &World) -> Option<crate::machine_slot::MachineSlot> {
    world
        .table
        .facts()
        .snapshot()
        .focus_machine()
        .map(|active| active.slot().clone())
}

/// The effects a turn committed other than the update of the running machine's saved state.
pub(crate) fn changes(result: &TurnResult) -> Vec<&crate::effects::TableEffect> {
    result
        .record
        .effects
        .iter()
        .filter(|effect| {
            !matches!(
                effect,
                crate::effects::TableEffect::ActiveSlotUpdated { .. }
            )
        })
        .collect()
}

/// Runs the narration job of a result and says the narration that was attached.
pub(crate) fn narrated(world: &mut World, result: &TurnResult) -> Option<String> {
    world
        .narrate(result)
        .expect("the narration runs")
        .map(|narration| narration.expose().to_owned())
}
