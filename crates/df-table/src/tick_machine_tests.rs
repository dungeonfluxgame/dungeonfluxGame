//! A small test machine with a migration and resource requests, so the parts of the contract the
//! lobby and explore do not use (a schema migration, a request to the broker and its result) have
//! a machine that keeps them.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::conformance::{self, ConformanceFixture, Expectation, ScriptedStep};
use crate::effects::TableState;
use crate::identity::MachineId;
use crate::lobby_tests::{action, lobby, seat};
use crate::machine::{
    Entry, HandOff, InspectReport, Machine, MachineError, MachineInput, MachineReadout,
    MachineSpec, ResourceRequest, Scope, StateName, StepInput, StepOutcome, StepResult,
    SuspendPolicy,
};
use crate::machine_slot::{LoadError, MachineSlot};
use crate::outcome::{Decline, DeclineCode, Outcome};
use crate::ports::{Audience, PortName, PurposeName, Readiness, ResourceHandle};
use crate::testkit::{QueuedDice, ScriptedBroker, ScriptedRulesOwner};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TickState {
    ticks: u32,
    in_flight: u32,
}

/// Counts ticks, asks the broker for a picture on each, and takes the results back. Its schema
/// is version 2: version 1 called `ticks` `count` and had no request in flight.
#[derive(Debug, Clone)]
pub(crate) struct TickMachine {
    spec: MachineSpec,
}

impl TickMachine {
    pub(crate) fn new() -> Self {
        let spec = MachineSpec::new(
            "ticker",
            Scope::Group,
            2,
            &["counting"],
            SuspendPolicy::CancelInFlight,
        )
        .expect("valid spec")
        .handing_off_to("lobby")
        .expect("valid target")
        .requiring_port(PortName::Broker);
        Self { spec }
    }

    fn state_of(&self, slot: &MachineSlot) -> Result<TickState, MachineError> {
        Ok(slot.decode(&self.spec)?)
    }

    fn encode(&self, state: &TickState) -> Result<MachineSlot, MachineError> {
        Ok(MachineSlot::encode(&self.spec, state)?)
    }

    /// A saved record as version 1 wrote it.
    pub(crate) fn version_one_record(count: u64) -> MachineSlot {
        let id = MachineId::new("ticker").expect("valid id");
        MachineSlot::from_parts(id, 1, json!({ "count": count }))
    }
}

impl Machine for TickMachine {
    fn spec(&self) -> &MachineSpec {
        &self.spec
    }

    fn enter(&self, _table: &TableState, _entry: &Entry) -> Result<MachineSlot, MachineError> {
        self.encode(&TickState {
            ticks: 0,
            in_flight: 0,
        })
    }

    fn accepts(&self, input: &MachineInput, state: &MachineSlot) -> bool {
        let Ok(state) = self.state_of(state) else {
            return false;
        };
        match input {
            MachineInput::Intent { action, .. } => action.as_str() == "tick",
            MachineInput::ResourceResult { .. } => state.in_flight > 0,
            _ => false,
        }
    }

    fn step(&self, slot: &MachineSlot, input: &StepInput) -> Result<StepResult, MachineError> {
        let state = self.state_of(slot)?;
        if !self.accepts(&input.input, slot) {
            return Ok(StepResult::declined(
                slot,
                Decline::with_default_reason(DeclineCode::NotViable),
            ));
        }
        match &input.input {
            MachineInput::Intent { .. } => {
                let next = self.encode(&TickState {
                    ticks: state.ticks + 1,
                    in_flight: state.in_flight + 1,
                })?;
                let mut result = StepResult::applied(next, Vec::new());
                result.resource_requests.push(ResourceRequest {
                    purpose: PurposeName::new("tick_art").expect("valid purpose"),
                });
                Ok(result)
            }
            _ => {
                let next = self.encode(&TickState {
                    ticks: state.ticks,
                    in_flight: state.in_flight - 1,
                })?;
                Ok(StepResult::applied(next, Vec::new()))
            }
        }
    }

    fn exit(&self, slot: &MachineSlot) -> Result<HandOff, MachineError> {
        self.state_of(slot)?;
        Ok(HandOff::new(
            MachineId::new("lobby").expect("valid id"),
            Vec::new(),
        )?)
    }

    fn state_name(&self, slot: &MachineSlot) -> Result<StateName, MachineError> {
        self.state_of(slot)?;
        Ok(StateName::new("counting")?)
    }

    fn project(
        &self,
        slot: &MachineSlot,
        _audience: Audience,
    ) -> Result<MachineReadout, MachineError> {
        let state = self.state_of(slot)?;
        Ok(MachineReadout::new(&format!(
            "Ticked {} times.",
            state.ticks
        ))?)
    }

    fn inspect(&self, slot: &MachineSlot) -> Result<InspectReport, MachineError> {
        Ok(InspectReport {
            machine: self.spec.id().clone(),
            state: self.state_name(slot)?,
            schema_version: slot.schema_version(),
        })
    }

    fn migrate(&self, from: &MachineSlot) -> Result<MachineSlot, LoadError> {
        let malformed = LoadError::Malformed {
            version: from.schema_version(),
        };
        if from.schema_version() != 1 {
            return Err(LoadError::UnsupportedVersion {
                found: from.schema_version(),
                expected: self.spec.schema_version(),
            });
        }
        let count = from
            .payload()
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .ok_or(malformed.clone())?;
        let ticks = u32::try_from(count).map_err(|_| malformed)?;
        Ok(MachineSlot::from_parts(
            self.spec.id().clone(),
            2,
            json!({ "ticks": ticks, "in_flight": 0 }),
        ))
    }
}

pub(crate) fn tick_fixture() -> ConformanceFixture {
    let table = TableState::new(2).expect("valid table");
    ConformanceFixture {
        machine: Arc::new(TickMachine::new()),
        neighbours: vec![Arc::new(lobby())],
        table,
        entry: Entry::empty(),
        refused_entry: None,
        script: vec![
            ScriptedStep::player(seat(1), action("tick"), Expectation::applied()),
            ScriptedStep::raw(
                MachineInput::ResourceResult {
                    handle: ResourceHandle::new(1),
                    readiness: Readiness::Ready,
                },
                Expectation::applied(),
            ),
            ScriptedStep::raw(
                MachineInput::ResourceResult {
                    handle: ResourceHandle::new(2),
                    readiness: Readiness::Ready,
                },
                Expectation::refused(),
            ),
        ],
        rules: ScriptedRulesOwner::default(),
        dice: QueuedDice::default(),
        broker: ScriptedBroker::default(),
    }
}

fn tick_input() -> StepInput {
    StepInput {
        correlation: crate::testkit::correlation(1),
        now_ms: 0,
        input: MachineInput::Intent {
            seat: seat(1),
            action: action("tick"),
        },
        decision: None,
    }
}

#[test]
fn a_machine_that_requests_a_resource_passes_the_whole_conformance_suite() {
    let report = conformance::run(tick_fixture());
    assert!(report.is_clean(), "{:#?}", report.findings());
}

#[test]
fn a_step_that_wants_a_picture_returns_the_request_and_never_calls_the_broker() {
    let machine = TickMachine::new();
    let slot = machine
        .enter(&TableState::new(2).expect("valid table"), &Entry::empty())
        .expect("enters");
    let result = machine.step(&slot, &tick_input()).expect("steps");
    assert_eq!(result.outcome, StepOutcome::Settled(Outcome::Applied));
    assert_eq!(result.resource_requests.len(), 1);
    assert_eq!(result.resource_requests[0].purpose.as_str(), "tick_art");
}

#[test]
fn a_version_one_record_migrates_to_the_current_version_when_it_is_loaded() {
    let machine = TickMachine::new();
    let loaded = machine
        .load(&TickMachine::version_one_record(41))
        .expect("version one migrates");
    assert_eq!(loaded.schema_version(), 2);
    assert_eq!(loaded.payload(), &json!({ "ticks": 41, "in_flight": 0 }));
    let readout = machine
        .project(&loaded, Audience::Public)
        .expect("projects");
    assert_eq!(readout.as_str(), "Ticked 41 times.");
}

#[test]
fn a_record_that_fails_its_migration_is_refused_and_never_loaded_wrongly() {
    let machine = TickMachine::new();
    let id = MachineId::new("ticker").expect("valid id");
    let missing = MachineSlot::from_parts(id.clone(), 1, json!({ "other": 1 }));
    let too_big = MachineSlot::from_parts(id, 1, json!({ "count": u64::MAX }));
    for record in [missing, too_big] {
        assert!(matches!(
            machine.load(&record),
            Err(LoadError::Malformed { version: 1 })
        ));
    }
}
