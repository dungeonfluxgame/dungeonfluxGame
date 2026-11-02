//! A small test machine with a migration and resource requests, so the parts of the contract the
//! lobby and explore do not use (a schema migration, a keyed request to the broker and its
//! result) have a machine that keeps them.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::conformance::{self, ConformanceFixture, Expectation, ScriptedStep};
use crate::effects::TableState;
use crate::identity::MachineId;
use crate::lobby_tests::{action, lobby, seat};
use crate::machine::{
    Entry, HandOff, InspectReport, Machine, MachineError, MachineInput, MachineReadout,
    MachineSpec, RequestKey, ResourceOutcome, ResourceRequest, Scope, StateName, StepInput,
    StepOutcome, StepResult, SuspendPolicy,
};
use crate::machine_slot::{ErrorClass, LoadError, MachineSlot};
use crate::outcome::{Decline, DeclineCode, Outcome};
use crate::ports::{Audience, PortName, PurposeName};
use crate::testkit::{QueuedDice, ScriptedBroker, ScriptedRulesOwner};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TickState {
    ticks: u32,
    outstanding: Vec<u64>,
}

/// Counts ticks, asks the broker for a picture on each under a key it stores, and takes the keyed
/// results back. Its schema is version 2: version 1 called `ticks` `count` and kept no keys.
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
            outstanding: Vec::new(),
        })
    }

    fn accepts(&self, table: &TableState, input: &MachineInput, state: &MachineSlot) -> bool {
        let Ok(state) = self.state_of(state) else {
            return false;
        };
        match input {
            MachineInput::Intent { seat, action } => {
                action.as_str() == "tick" && table.seats().contains(seat)
            }
            MachineInput::ResourceResult { key, .. } => state.outstanding.contains(&key.get()),
            _ => false,
        }
    }

    fn step(&self, slot: &MachineSlot, input: &StepInput<'_>) -> Result<StepResult, MachineError> {
        let mut state = self.state_of(slot)?;
        if !self.accepts(input.table, &input.input, slot) {
            return Ok(StepResult::declined(Decline::with_default_reason(
                DeclineCode::NotViable,
            )));
        }
        match &input.input {
            MachineInput::Intent { .. } => {
                state.ticks += 1;
                let key = RequestKey::new(u64::from(state.ticks));
                state.outstanding.push(key.get());
                let mut result = StepResult::applied(self.encode(&state)?, Vec::new());
                result.resource_requests.push(ResourceRequest {
                    key,
                    purpose: PurposeName::new("tick_art").expect("valid purpose"),
                });
                Ok(result)
            }
            MachineInput::ResourceResult { key, .. } => {
                state.outstanding.retain(|open| *open != key.get());
                Ok(StepResult::applied(self.encode(&state)?, Vec::new()))
            }
            _ => unreachable!("accepts is false for every other input"),
        }
    }

    fn wants_hand_off(&self, slot: &MachineSlot) -> Result<bool, MachineError> {
        self.state_of(slot)?;
        Ok(false)
    }

    fn exit(&self, _table: &TableState, slot: &MachineSlot) -> Result<HandOff, MachineError> {
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

    fn validate(&self, slot: &MachineSlot) -> Result<(), MachineError> {
        self.state_of(slot).map(|_| ())
    }

    fn project(
        &self,
        _table: &TableState,
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
            class: ErrorClass::Data,
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
            json!({ "ticks": ticks, "outstanding": [] }),
        ))
    }
}

pub(crate) fn tick_fixture() -> ConformanceFixture {
    let table = TableState::seated(2, &[seat(1)]).expect("valid table");
    ConformanceFixture {
        machine: Arc::new(TickMachine::new()),
        neighbours: vec![Arc::new(lobby())],
        wired: vec![PortName::Broker],
        table,
        entry: Entry::empty(),
        refused_entry: None,
        script: vec![
            ScriptedStep::player(seat(1), action("tick"), Expectation::applied()),
            ScriptedStep::player(seat(1), action("tick"), Expectation::applied()),
            ScriptedStep::raw(
                MachineInput::ResourceResult {
                    key: RequestKey::new(77),
                    outcome: ResourceOutcome::Ready,
                },
                Expectation::refused(),
            ),
            ScriptedStep::raw(
                MachineInput::ResourceResult {
                    key: RequestKey::new(1),
                    outcome: ResourceOutcome::Ready,
                },
                Expectation::applied(),
            ),
            ScriptedStep::raw(
                MachineInput::ResourceResult {
                    key: RequestKey::new(1),
                    outcome: ResourceOutcome::Ready,
                },
                Expectation::refused(),
            ),
        ],
        rules: ScriptedRulesOwner::default(),
        dice: QueuedDice::default(),
        broker: ScriptedBroker::default(),
    }
}

fn tick_input(table: &TableState) -> StepInput<'_> {
    StepInput {
        correlation: crate::testkit::correlation(1),
        now_ms: 0,
        table,
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
fn a_step_that_wants_a_picture_returns_a_keyed_request_and_never_calls_the_broker() {
    let machine = TickMachine::new();
    let table = TableState::seated(2, &[seat(1)]).expect("valid table");
    let slot = machine.enter(&table, &Entry::empty()).expect("enters");
    let result = machine.step(&slot, &tick_input(&table)).expect("steps");
    assert_eq!(result.outcome, StepOutcome::Settled(Outcome::Applied));
    assert_eq!(result.resource_requests.len(), 1);
    assert_eq!(result.resource_requests[0].purpose.as_str(), "tick_art");
    assert_eq!(result.resource_requests[0].key, RequestKey::new(1));
}

#[test]
fn a_result_is_matched_by_its_key_and_a_key_no_request_used_is_declined_with_requests_open() {
    let machine = TickMachine::new();
    let table = TableState::seated(2, &[seat(1)]).expect("valid table");
    let slot = machine.enter(&table, &Entry::empty()).expect("enters");
    let two_requests = {
        let first = machine.step(&slot, &tick_input(&table)).expect("steps");
        let state = first.state.expect("state");
        machine
            .step(&state, &tick_input(&table))
            .expect("steps")
            .state
            .expect("state")
    };
    let result = |key: u64| MachineInput::ResourceResult {
        key: RequestKey::new(key),
        outcome: ResourceOutcome::Ready,
    };
    assert!(!machine.accepts(&table, &result(77), &two_requests));
    assert!(machine.accepts(&table, &result(1), &two_requests));
    assert!(machine.accepts(&table, &result(2), &two_requests));
}

#[test]
fn a_version_one_record_migrates_to_the_current_version_when_it_is_loaded() {
    let machine = TickMachine::new();
    let loaded = machine
        .load(&TickMachine::version_one_record(41))
        .expect("version one migrates");
    assert_eq!(loaded.schema_version(), 2);
    assert_eq!(loaded.payload(), &json!({ "ticks": 41, "outstanding": [] }));
    let table = TableState::new(2).expect("valid table");
    let readout = machine
        .project(&table, &loaded, Audience::Public)
        .expect("projects");
    assert_eq!(readout.as_str(), "Ticked 41 times.");
}

#[test]
fn a_record_that_fails_its_migration_is_refused_with_its_cause_and_never_loaded_wrongly() {
    let machine = TickMachine::new();
    let id = MachineId::new("ticker").expect("valid id");
    let missing = MachineSlot::from_parts(id.clone(), 1, json!({ "other": 1 }));
    let too_big = MachineSlot::from_parts(id, 1, json!({ "count": u64::MAX }));
    for record in [missing, too_big] {
        assert!(matches!(
            machine.load(&record),
            Err(LoadError::Malformed {
                version: 1,
                class: ErrorClass::Data
            })
        ));
    }
}

#[test]
fn a_migration_that_does_not_advance_the_version_is_refused() {
    struct Stuck(TickMachine);
    impl Machine for Stuck {
        fn spec(&self) -> &MachineSpec {
            self.0.spec()
        }
        fn enter(&self, table: &TableState, entry: &Entry) -> Result<MachineSlot, MachineError> {
            self.0.enter(table, entry)
        }
        fn accepts(&self, table: &TableState, input: &MachineInput, state: &MachineSlot) -> bool {
            self.0.accepts(table, input, state)
        }
        fn step(
            &self,
            state: &MachineSlot,
            input: &StepInput<'_>,
        ) -> Result<StepResult, MachineError> {
            self.0.step(state, input)
        }
        fn wants_hand_off(&self, state: &MachineSlot) -> Result<bool, MachineError> {
            self.0.wants_hand_off(state)
        }
        fn exit(&self, table: &TableState, state: &MachineSlot) -> Result<HandOff, MachineError> {
            self.0.exit(table, state)
        }
        fn state_name(&self, state: &MachineSlot) -> Result<StateName, MachineError> {
            self.0.state_name(state)
        }
        fn validate(&self, state: &MachineSlot) -> Result<(), MachineError> {
            self.0.validate(state)
        }
        fn project(
            &self,
            table: &TableState,
            state: &MachineSlot,
            audience: Audience,
        ) -> Result<MachineReadout, MachineError> {
            self.0.project(table, state, audience)
        }
        fn inspect(&self, state: &MachineSlot) -> Result<InspectReport, MachineError> {
            self.0.inspect(state)
        }
        fn migrate(&self, from: &MachineSlot) -> Result<MachineSlot, LoadError> {
            Ok(from.clone())
        }
    }
    let record = TickMachine::version_one_record(3);
    assert_eq!(
        Stuck(TickMachine::new()).load(&record),
        Err(LoadError::UnsupportedVersion {
            found: 1,
            expected: 2
        })
    );
}
