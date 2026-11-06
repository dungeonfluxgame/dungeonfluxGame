//! The checks that do not need a script: registration, entry, the saved record and every way to
//! damage it, input addressed to nothing, the hand-off ask and exit. Each reports a `Finding`
//! against its named check and never panics. The per-state probes run in every state the script
//! reaches, so what a fixture leaves out is probed anyway.

use std::sync::Arc;

use serde_json::Value;

use crate::effects::TableState;
use crate::envelope::{PendingId, ProposalOrigin, ProposerRef, TimerName};
use crate::identity::{MachineId, SeatId, TableId, TableScope};
use crate::machine::slot::MachineSlot;
use crate::machine::{
    CarryItem, InputClaim, Machine, MachineError, MachineInput, RequestKey, ResourceOutcome,
    StepInput, StepOutcome, exited_event,
};
use crate::outcome::Outcome;
use crate::ports::ActionId;
use crate::registry::{Enablement, RegistryBuilder, check_hand_off_route};
use crate::testkit::correlation;

use super::{ConformanceCheck, ConformanceFixture, ConformanceReport};

/// The table every probe event is filed under.
pub(super) const PROBE_TABLE: TableId = TableId::new(1);
/// A turn number far from any scripted step, so a probe's operation id never repeats a step's.
const PROBE_TURN: u64 = u64::MAX / 2;
/// The action no machine is meant to know.
const UNKNOWN_ACTION: &str = "conformance_unknown_action";
const PROBE_NAME: &str = "conformance_probe";

/// What the per-state probes need to know besides the state.
pub(super) struct Context<'a> {
    pub machine: &'a dyn Machine,
    pub neighbours: &'a [Arc<dyn Machine>],
    /// The actions the script sends, which a seat that is not at the table must not be able to use.
    pub actions: &'a [ActionId],
}

pub(super) fn registration(fixture: &ConformanceFixture, report: &mut ConformanceReport) {
    let id = fixture.machine.spec().id().clone();
    let mut builder = RegistryBuilder::new().register_shared(fixture.machine.clone());
    for neighbour in &fixture.neighbours {
        builder = builder.register_shared(neighbour.clone());
    }
    if let Err(errors) = builder.build(&fixture.wired) {
        for error in errors.iter().filter(|error| error.machine() == Some(&id)) {
            report.fail(ConformanceCheck::Registration, error.to_string());
        }
    }
}

pub(super) fn entry_refusal(fixture: &ConformanceFixture, report: &mut ConformanceReport) {
    let Some(entry) = &fixture.refused_entry else {
        if fixture.machine.spec().reads_entry() {
            report.fail(
                ConformanceCheck::EntryRefusal,
                "the spec says the machine reads its entry, but the fixture has no refused entry",
            );
        } else {
            report.skipped.push(ConformanceCheck::EntryRefusal);
        }
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

fn audiences(table: &TableState) -> Vec<crate::ports::Audience> {
    let mut audiences = vec![crate::ports::Audience::Public];
    audiences.extend(
        table
            .seats()
            .iter()
            .copied()
            .map(crate::ports::Audience::Seat),
    );
    audiences.push(crate::ports::Audience::Seat(SeatId::new(u64::MAX)));
    audiences
}

/// What every state the machine is in must satisfy, whatever the script did to get there. It
/// returns the machine's ask for a hand-off, the first time it was asked.
pub(super) fn check_slot(
    context: &Context<'_>,
    table: &TableState,
    slot: &MachineSlot,
    label: &str,
    report: &mut ConformanceReport,
) -> Option<bool> {
    let machine = context.machine;
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
            if machine.state_name(slot).as_ref() != Ok(&name) {
                report.fail(
                    ConformanceCheck::Determinism,
                    format!("{label}: state_name gave two different names"),
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
        match machine.project(table, slot, audience) {
            Ok(readout) => {
                if machine.project(table, slot, audience).as_ref() != Ok(&readout) {
                    report.fail(
                        ConformanceCheck::Determinism,
                        format!("{label}: project gave two different readouts for {audience:?}"),
                    );
                }
            }
            Err(error) => report.fail(
                ConformanceCheck::Projection,
                format!("{label}: nothing to show {audience:?}: {error}"),
            ),
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
            if machine.inspect(slot).as_ref() != Ok(&inspected) {
                report.fail(
                    ConformanceCheck::Determinism,
                    format!("{label}: inspect gave two different reports"),
                );
            }
        }
        Err(error) => report.fail(
            ConformanceCheck::InspectReport,
            format!("{label}: no report: {error}"),
        ),
    }
    load_damage(machine, slot, label, report);
    strays(context, table, slot, label, report);
    let asks = hand_off_ask(machine, slot, label, report);
    exit_hand_off(context, table, slot, label, report);
    asks
}

fn claims(machine: &dyn Machine, claim: InputClaim) -> bool {
    machine.spec().claimed_inputs().contains(&claim)
}

/// Every input that is for nothing the machine holds, in this state.
fn strays(
    context: &Context<'_>,
    table: &TableState,
    slot: &MachineSlot,
    label: &str,
    report: &mut ConformanceReport,
) {
    let machine = context.machine;
    let seated = table
        .seats()
        .iter()
        .next()
        .copied()
        .unwrap_or(SeatId::new(1));
    let absent = SeatId::new(u64::MAX);
    let mut probes = vec![
        (
            MachineInput::Dice {
                pending: PendingId::new(u64::MAX),
                draws: Vec::new(),
            },
            ConformanceCheck::DiceAndWaiting,
        ),
        (
            MachineInput::ResourceResult {
                key: RequestKey::new(u64::MAX),
                outcome: ResourceOutcome::Ready,
            },
            ConformanceCheck::ResourceResults,
        ),
    ];
    if let Ok(action) = ActionId::new(UNKNOWN_ACTION) {
        probes.push((
            MachineInput::Intent {
                seat: seated,
                action,
            },
            ConformanceCheck::Accepts,
        ));
    }
    if !claims(machine, InputClaim::Timer)
        && let Ok(timer) = TimerName::new(PROBE_NAME)
    {
        probes.push((MachineInput::Timer { timer }, ConformanceCheck::Accepts));
    }
    if !claims(machine, InputClaim::EngineProposal)
        && let Ok(proposer) = ProposerRef::new(PROBE_NAME)
    {
        probes.push((
            MachineInput::Proposal {
                origin: ProposalOrigin::World,
                proposer,
            },
            ConformanceCheck::Accepts,
        ));
    }
    for action in context.actions {
        if !machine.spec().unseated_actions().contains(action) {
            probes.push((
                MachineInput::Intent {
                    seat: absent,
                    action: action.clone(),
                },
                ConformanceCheck::Accepts,
            ));
        }
    }
    for (input, check) in probes {
        refuses(machine, table, slot, input, check, label, report);
    }
}

/// An input that is for nothing the machine holds must not be accepted, and `step` must decline
/// it and change nothing.
fn refuses(
    machine: &dyn Machine,
    table: &TableState,
    slot: &MachineSlot,
    input: MachineInput,
    check: ConformanceCheck,
    label: &str,
    report: &mut ConformanceReport,
) {
    if machine.accepts(table, &input, slot) {
        report.fail(
            check,
            format!("{label}: accepted {input:?}, which is addressed to nothing it holds"),
        );
        return;
    }
    let step_input = StepInput {
        correlation: correlation(PROBE_TURN),
        now_ms: 0,
        table,
        input: input.clone(),
        decision: None,
    };
    match machine.step(slot, &step_input) {
        Ok(result) => {
            let declined = matches!(result.outcome, StepOutcome::Settled(Outcome::Declined(_)));
            let unchanged = result.state.is_none()
                && result.table_effects.is_empty()
                && result.resource_requests.is_empty();
            if !declined || !unchanged {
                report.fail(
                    check,
                    format!("{label}: {input:?} was not declined cleanly"),
                );
            }
        }
        Err(error) => report.fail(
            check,
            format!("{label}: {input:?} failed instead of being declined: {error}"),
        ),
    }
}

/// The ask for a hand-off is read from the state, so asking again, after a refused hand-off or
/// after other input, gives the same answer. The first answer is returned for the script.
fn hand_off_ask(
    machine: &dyn Machine,
    slot: &MachineSlot,
    label: &str,
    report: &mut ConformanceReport,
) -> Option<bool> {
    let answers = [
        machine.wants_hand_off(slot),
        machine.wants_hand_off(slot),
        machine.wants_hand_off(slot),
    ];
    match answers {
        [Ok(first), Ok(second), Ok(third)] if first == second && second == third => Some(first),
        [Ok(first), Ok(_), Ok(_)] => {
            report.fail(
                ConformanceCheck::HandOffAsk,
                format!(
                    "{label}: the ask for a hand-off changed between calls, so a refusal strands the table"
                ),
            );
            Some(first)
        }
        _ => {
            report.fail(
                ConformanceCheck::HandOffAsk,
                format!("{label}: wants_hand_off failed for the machine's own state"),
            );
            None
        }
    }
}

fn other_machine_id(id: &MachineId) -> Option<MachineId> {
    ["another_machine", "some_other_machine"]
        .into_iter()
        .filter_map(|name| MachineId::new(name).ok())
        .find(|candidate| candidate != id)
}

/// A record damaged from the machine's own: each field removed, each field nulled, and the
/// previous schema version, must all be refused at load.
fn load_damage(
    machine: &dyn Machine,
    slot: &MachineSlot,
    label: &str,
    report: &mut ConformanceReport,
) {
    let spec = machine.spec();
    let id = spec.id();
    let version = slot.schema_version();
    let mut cases: Vec<(String, MachineSlot)> = Vec::new();
    if let Some(fields) = slot.payload().as_object() {
        for key in fields.keys() {
            let mut removed = fields.clone();
            removed.remove(key);
            cases.push((
                format!("the field {key} removed"),
                MachineSlot::from_parts(id.clone(), version, Value::Object(removed)),
            ));
            let mut nulled = fields.clone();
            nulled.insert(key.clone(), Value::Null);
            cases.push((
                format!("the field {key} null"),
                MachineSlot::from_parts(id.clone(), version, Value::Object(nulled)),
            ));
        }
    }
    if let Some(older) = spec
        .schema_version()
        .checked_sub(1)
        .filter(|older| *older > 0)
    {
        cases.push((
            format!("the previous schema version {older}"),
            MachineSlot::from_parts(id.clone(), older, slot.payload().clone()),
        ));
    }
    for (what, record) in cases {
        match machine.load(&record) {
            Ok(_) => report.fail(
                ConformanceCheck::LoadRefusal,
                format!("{label}: loaded a record with {what}"),
            ),
            Err(error) => report
                .events
                .push(error.refusal_event(PROBE_TABLE, spec, &record)),
        }
    }
}

/// The record must be refused when it is for another machine, newer than the engine, versionless
/// or not a state at all.
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
            Err(error) => report
                .events
                .push(error.refusal_event(PROBE_TABLE, spec, &record)),
        }
    }
}

fn damaged(id: &MachineId, version: u32) -> MachineSlot {
    MachineSlot::from_parts(id.clone(), version, Value::String("damaged".to_owned()))
}

/// A damaged record is a typed failure everywhere: never a default state, never a projection.
pub(super) fn failure_events(
    machine: &dyn Machine,
    table: &TableState,
    report: &mut ConformanceReport,
) {
    let spec = machine.spec();
    let record = damaged(spec.id(), spec.schema_version());
    let Ok(timer) = TimerName::new(PROBE_NAME) else {
        return;
    };
    let probe = correlation(PROBE_TURN);
    let input = MachineInput::Timer { timer };
    let scope = TableScope::of_table(PROBE_TABLE).in_machine(spec.id().clone());
    let step_input = StepInput {
        correlation: probe,
        now_ms: 0,
        table,
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
        ("accepts", machine.accepts(table, &input, &record)),
        ("state_name", machine.state_name(&record).is_ok()),
        ("validate", machine.validate(&record).is_ok()),
        ("wants_hand_off", machine.wants_hand_off(&record).is_ok()),
        ("exit", machine.exit(table, &record).is_ok()),
        (
            "project",
            machine
                .project(table, &record, crate::ports::Audience::Public)
                .is_ok(),
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

/// Exit, in every state, names a hand-off the registry rules accept, twice the same, carrying
/// only what the table backs.
fn exit_hand_off(
    context: &Context<'_>,
    table: &TableState,
    slot: &MachineSlot,
    label: &str,
    report: &mut ConformanceReport,
) {
    let machine = context.machine;
    let spec = machine.spec();
    let hand_off = match machine.exit(table, slot) {
        Ok(hand_off) => hand_off,
        Err(error) => {
            report.fail(
                ConformanceCheck::ExitHandOff,
                format!("{label}: exit failed: {error}"),
            );
            return;
        }
    };
    if machine.exit(table, slot).as_ref() != Ok(&hand_off) {
        report.fail(
            ConformanceCheck::Determinism,
            format!("{label}: exit gave two different hand-offs"),
        );
    }
    let neighbour = |id: &MachineId| {
        context
            .neighbours
            .iter()
            .find(|candidate| candidate.spec().id() == id)
            .map(|candidate| (candidate.spec().scope(), Enablement::Enabled))
    };
    if let Err(refusal) = check_hand_off_route(spec, &hand_off, neighbour) {
        report.fail(
            ConformanceCheck::ExitHandOff,
            format!("{label}: the hand-off is refused: {refusal}"),
        );
    }
    for item in hand_off.carry() {
        match item {
            CarryItem::Seat(seat) if !table.seats().contains(seat) => report.fail(
                ConformanceCheck::ExitHandOff,
                format!("{label}: exit carries seat {seat}, which is not at the table"),
            ),
            CarryItem::Scene(scene) if table.active_scene() != Some(*scene) => report.fail(
                ConformanceCheck::ExitHandOff,
                format!("{label}: exit carries scene {scene}, which is not the active scene"),
            ),
            CarryItem::Seat(_) | CarryItem::Scene(_) => {}
        }
    }
    let scope = TableScope::of_table(PROBE_TABLE).in_machine(spec.id().clone());
    report
        .events
        .push(exited_event(&scope, correlation(PROBE_TURN), &hand_off));
}
