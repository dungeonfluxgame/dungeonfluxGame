//! The checks that do not need a script: registration, entry, the saved record, damage, strays
//! and exit. Each reports a `Finding` against its named check and never panics.

use serde_json::Value;

use crate::effects::{ApplyOutcome, TableEffect, TableState};
use crate::envelope::{PendingId, TimerName};
use crate::event::TableEvent;
use crate::identity::{MachineId, SeatId, TableId, TableScope};
use crate::machine::{CarryItem, Machine, MachineError, MachineInput, StepInput, StepOutcome};
use crate::machine_slot::MachineSlot;
use crate::outcome::Outcome;
use crate::ports::{Audience, PortName, Readiness, ResourceHandle};
use crate::registry::RegistryBuilder;
use crate::testkit::correlation;

use super::{ConformanceCheck, ConformanceFixture, ConformanceReport};

/// The table every probe event is filed under.
pub(super) const PROBE_TABLE: TableId = TableId::new(1);
/// A turn number far from any scripted step, so a probe's operation id never repeats a step's.
const PROBE_TURN: u64 = u64::MAX / 2;

pub(super) fn registration(fixture: &ConformanceFixture, report: &mut ConformanceReport) {
    let id = fixture.machine.spec().id().clone();
    let mut builder = RegistryBuilder::new().register_shared(fixture.machine.clone());
    for neighbour in &fixture.neighbours {
        builder = builder.register_shared(neighbour.clone());
    }
    if let Err(errors) = builder.build(PortName::ALL) {
        for error in errors.iter().filter(|error| error.machine() == Some(&id)) {
            report.fail(ConformanceCheck::Registration, error.to_string());
        }
    }
}

pub(super) fn entry_refusal(fixture: &ConformanceFixture, report: &mut ConformanceReport) {
    let Some(entry) = &fixture.refused_entry else {
        return;
    };
    match fixture.machine.enter(&fixture.table, entry) {
        Err(MachineError::Entry(_)) => {}
        Err(other) => report.fail(
            ConformanceCheck::EntryRefusal,
            format!("an entry the table disagrees with failed as {other}, not as an entry refusal"),
        ),
        Ok(_) => report.fail(
            ConformanceCheck::EntryRefusal,
            "an entry the table disagrees with was accepted",
        ),
    }
}

fn audiences(table: &TableState) -> Vec<Audience> {
    let mut audiences = vec![Audience::Public];
    audiences.extend(table.seats().iter().copied().map(Audience::Seat));
    audiences.push(Audience::Seat(SeatId::new(u64::MAX)));
    audiences
}

/// What every state the machine is in must satisfy. A result for a request can only be for one
/// the machine has outstanding, so the stray result is tried only when there is none.
pub(super) fn check_slot(
    machine: &dyn Machine,
    table: &TableState,
    slot: &MachineSlot,
    outstanding_requests: usize,
    label: &str,
    report: &mut ConformanceReport,
) {
    let spec = machine.spec();
    if slot.machine() != spec.id() || slot.schema_version() != spec.schema_version() {
        report.fail(
            ConformanceCheck::SavedState,
            format!(
                "{label}: the record is stamped {} version {}, the spec says {} version {}",
                slot.machine(),
                slot.schema_version(),
                spec.id(),
                spec.schema_version()
            ),
        );
    }
    match machine.load(slot) {
        Ok(loaded) if &loaded == slot => {}
        Ok(_) => report.fail(
            ConformanceCheck::SavedState,
            format!("{label}: loading the record changed it"),
        ),
        Err(error) => report.fail(
            ConformanceCheck::SavedState,
            format!("{label}: the machine's own record does not load: {error}"),
        ),
    }
    let name = match machine.state_name(slot) {
        Ok(name) => {
            if !spec.states().contains(&name) {
                report.fail(
                    ConformanceCheck::StateNames,
                    format!("{label}: the state {name} is not in the spec"),
                );
            }
            Some(name)
        }
        Err(error) => {
            report.fail(
                ConformanceCheck::StateNames,
                format!("{label}: the state has no name: {error}"),
            );
            None
        }
    };
    for audience in audiences(table) {
        if let Err(error) = machine.project(slot, audience) {
            report.fail(
                ConformanceCheck::Projection,
                format!("{label}: nothing to show {audience:?}: {error}"),
            );
        }
    }
    match machine.inspect(slot) {
        Ok(inspected) => {
            if &inspected.machine != spec.id()
                || inspected.schema_version != slot.schema_version()
                || Some(&inspected.state) != name.as_ref()
            {
                report.fail(
                    ConformanceCheck::InspectReport,
                    format!("{label}: the report does not describe the machine's own state"),
                );
            }
        }
        Err(error) => report.fail(
            ConformanceCheck::InspectReport,
            format!("{label}: no report: {error}"),
        ),
    }
    let stray_roll = MachineInput::Dice {
        pending: PendingId::new(u64::MAX),
        draws: Vec::new(),
    };
    let stray_result = MachineInput::ResourceResult {
        handle: ResourceHandle::new(u64::MAX),
        readiness: Readiness::Ready,
    };
    refuses(
        machine,
        slot,
        stray_roll,
        ConformanceCheck::DiceAndWaiting,
        label,
        report,
    );
    if outstanding_requests == 0 {
        refuses(
            machine,
            slot,
            stray_result,
            ConformanceCheck::ResourceResults,
            label,
            report,
        );
    }
}

/// An input that is for nothing the machine holds must not be accepted, and `step` must decline
/// it and change nothing.
fn refuses(
    machine: &dyn Machine,
    slot: &MachineSlot,
    input: MachineInput,
    check: ConformanceCheck,
    label: &str,
    report: &mut ConformanceReport,
) {
    if machine.accepts(&input, slot) {
        report.fail(
            check,
            format!("{label}: accepted {input:?}, which names nothing it holds"),
        );
        return;
    }
    let step_input = StepInput {
        correlation: correlation(PROBE_TURN),
        now_ms: 0,
        input,
        decision: None,
    };
    match machine.step(slot, &step_input) {
        Ok(result) => {
            let declined = matches!(result.outcome, StepOutcome::Settled(Outcome::Declined(_)));
            let unchanged = result.state == *slot
                && result.table_effects.is_empty()
                && result.resource_requests.is_empty();
            if !declined || !unchanged {
                report.fail(
                    check,
                    format!("{label}: an input for nothing it holds was not declined cleanly"),
                );
            }
        }
        Err(error) => report.fail(
            check,
            format!(
                "{label}: an input for nothing it holds failed instead of being declined: {error}"
            ),
        ),
    }
}

fn other_machine_id(id: &MachineId) -> Option<MachineId> {
    ["another_machine", "some_other_machine"]
        .into_iter()
        .filter_map(|name| MachineId::new(name).ok())
        .find(|candidate| candidate != id)
}

/// The record must be refused when it is for another machine, newer than the engine, versionless
/// or damaged.
pub(super) fn load_refusal(
    machine: &dyn Machine,
    slot: &MachineSlot,
    report: &mut ConformanceReport,
) {
    let spec = machine.spec();
    let id = spec.id().clone();
    let mut cases = vec![
        (
            "a record with version zero",
            MachineSlot::from_parts(id.clone(), 0, slot.payload().clone()),
        ),
        (
            "a record whose state is damaged",
            damaged(spec.id(), spec.schema_version()),
        ),
    ];
    if let Some(other) = other_machine_id(&id) {
        cases.push((
            "a record for another machine",
            MachineSlot::from_parts(other, slot.schema_version(), slot.payload().clone()),
        ));
    }
    if let Some(newer) = spec.schema_version().checked_add(1) {
        cases.push((
            "a record newer than the engine",
            MachineSlot::from_parts(id, newer, slot.payload().clone()),
        ));
    }
    for (what, record) in cases {
        match machine.load(&record) {
            Ok(_) => report.fail(ConformanceCheck::LoadRefusal, format!("loaded {what}")),
            Err(error) => report.events.push(error.refusal_event(spec, &record)),
        }
    }
}

fn damaged(id: &MachineId, version: u32) -> MachineSlot {
    MachineSlot::from_parts(id.clone(), version, Value::String("damaged".to_owned()))
}

/// A damaged record is a typed failure everywhere: never a default state, never a projection.
pub(super) fn failure_events(machine: &dyn Machine, report: &mut ConformanceReport) {
    let spec = machine.spec();
    let record = damaged(spec.id(), spec.schema_version());
    let Ok(timer) = TimerName::new("conformance_probe") else {
        return;
    };
    let probe = correlation(PROBE_TURN);
    let input = MachineInput::Timer { timer };
    let scope = TableScope::of_table(PROBE_TABLE).in_machine(spec.id().clone());
    let step_input = StepInput {
        correlation: probe,
        now_ms: 0,
        input: input.clone(),
        decision: None,
    };
    match machine.step(&record, &step_input) {
        Err(error) => report.events.push(error.event(&scope, probe)),
        Ok(_) => report.fail(
            ConformanceCheck::FailureEvents,
            "step accepted a damaged record instead of failing",
        ),
    }
    let silent = [
        ("accepts", machine.accepts(&input, &record)),
        ("state_name", machine.state_name(&record).is_ok()),
        ("exit", machine.exit(&record).is_ok()),
        (
            "project",
            machine.project(&record, Audience::Public).is_ok(),
        ),
        ("inspect", machine.inspect(&record).is_ok()),
    ];
    for (call, answered) in silent {
        if answered {
            report.fail(
                ConformanceCheck::FailureEvents,
                format!("{call} answered for a damaged record instead of failing"),
            );
        }
    }
}

/// Exit names a declared target and a typed carry list the table backs, twice the same, and
/// recording the hand-off leaves the seats, the scene and the beat as they were.
pub(super) fn exit_hand_off(
    machine: &dyn Machine,
    table: &TableState,
    slot: &MachineSlot,
    report: &mut ConformanceReport,
) {
    let spec = machine.spec();
    let hand_off = match machine.exit(slot) {
        Ok(hand_off) => hand_off,
        Err(error) => {
            report.fail(
                ConformanceCheck::ExitHandOff,
                format!("exit failed: {error}"),
            );
            return;
        }
    };
    if machine.exit(slot).as_ref() != Ok(&hand_off) {
        report.fail(
            ConformanceCheck::Determinism,
            "exit gave two different hand-offs",
        );
    }
    if !spec.hand_off_targets().contains(hand_off.target()) {
        report.fail(
            ConformanceCheck::ExitHandOff,
            format!(
                "exit names {}, which the spec does not declare",
                hand_off.target()
            ),
        );
    }
    for item in hand_off.carry() {
        match item {
            CarryItem::Seat(seat) if !table.seats().contains(seat) => report.fail(
                ConformanceCheck::ExitHandOff,
                format!("exit carries seat {seat}, which is not at the table"),
            ),
            CarryItem::Scene(scene) if table.active_scene() != Some(*scene) => report.fail(
                ConformanceCheck::ExitHandOff,
                format!("exit carries scene {scene}, which is not the active scene"),
            ),
            CarryItem::Seat(_) | CarryItem::Scene(_) => {}
        }
    }
    let probe = correlation(PROBE_TURN);
    let scope = TableScope::of_table(PROBE_TABLE).in_machine(spec.id().clone());
    report.events.push(TableEvent::MachineExited {
        scope: scope.clone(),
        correlation: probe,
        target: hand_off.target().clone(),
        carried: hand_off.carry().len(),
    });
    let mut after = table.clone();
    let record = TableEffect::HandOffRecorded {
        from: spec.id().clone(),
        to: hand_off.target().clone(),
        carried: hand_off.carry().len(),
    };
    match after.apply(&scope, probe, &[record]) {
        Ok(ApplyOutcome::Applied(_)) if after.same_world_as(table) => {}
        Ok(_) => report.fail(
            ConformanceCheck::ExitHandOff,
            "recording the hand-off changed the seats, the scene or the beat",
        ),
        Err(error) => report.fail(
            ConformanceCheck::ExitHandOff,
            format!("the hand-off could not be recorded: {error}"),
        ),
    }
}
