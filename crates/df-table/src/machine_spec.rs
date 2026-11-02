//! What a machine declares about itself at registration: its id, scope, states and neighbours.
//!
//! It owns `MachineSpec`, the one value the registry validates and the conformance suite reads
//! (RES27, RES20, O.5): the machine's id, its `Scope` (table, group or seat), the schema version
//! of its saved state, the closed list of its state names, the machines it may hand off to, the
//! ports it needs, the actions it sends to the rules owner before `step`, and what it does with
//! in-flight requests on suspend (H.9). A spec is built only valid: names are checked, the state
//! list is non-empty and free of duplicates, and the schema version is never zero. It never
//! checks a spec against the other machines (the registry does) and never runs a machine.

use std::fmt;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::element::{NameError, name_type};
use crate::identity::MachineId;
use crate::listed::listed_enum;
use crate::ports::{ActionId, PortName};

name_type!(
    /// The name of one state of a machine's closed state enum, such as `gathering`.
    StateName
);

listed_enum! {
    /// How far a machine's authority reaches (RES20, RES28).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Scope {
        /// The whole table before any scene exists (the lobby).
        Table,
        /// The one active machine of a scene: explore, dialogue, combat.
        Group,
        /// One seat's lane beside the group machine: creation, a personal item inspector.
        Seat,
    }
}

impl Scope {
    pub fn key(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Group => "group",
            Self::Seat => "seat",
        }
    }

    /// Whether a machine of this scope may hand off to a machine of `target` scope. A table
    /// machine starts a scene's group machine; a scene may return the table to its table machine;
    /// a seat lane never changes the group machine (RES20).
    pub fn may_hand_off_to(self, target: Self) -> bool {
        match self {
            Self::Table => matches!(target, Self::Group),
            Self::Group => matches!(target, Self::Group | Self::Table),
            Self::Seat => matches!(target, Self::Seat),
        }
    }
}

listed_enum! {
    /// What a machine does with a request still in flight when the table suspends (H.9).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum SuspendPolicy {
        /// The request finishes and its result is held until the table resumes.
        FinishAndHold,
        /// The request is cancelled with `request.cancelled{cause=suspend}`.
        CancelInFlight,
    }
}

impl SuspendPolicy {
    pub fn key(self) -> &'static str {
        match self {
            Self::FinishAndHold => "finish_and_hold",
            Self::CancelInFlight => "cancel_in_flight",
        }
    }
}

listed_enum! {
    /// An engine input that is not addressed to a seat or a pending, which a machine may claim.
    /// Claiming one is deliberate: the conformance suite sends every unclaimed kind to the
    /// machine and expects it to be declined.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum InputClaim {
        Timer,
        EngineProposal,
    }
}

impl InputClaim {
    pub fn key(self) -> &'static str {
        match self {
            Self::Timer => "timer",
            Self::EngineProposal => "engine_proposal",
        }
    }
}

/// Why a spec could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecError {
    InvalidName(NameError),
    NoStates,
    DuplicateState(StateName),
    ZeroSchemaVersion,
    DuplicateHandOffTarget(MachineId),
    DuplicateRulesAction(ActionId),
    DuplicateUnseatedAction(ActionId),
}

impl fmt::Display for SpecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(error) => write!(formatter, "a spec name is invalid: {error}"),
            Self::NoStates => formatter.write_str("a machine has at least one state"),
            Self::DuplicateState(name) => write!(formatter, "the state {name} is listed twice"),
            Self::ZeroSchemaVersion => formatter.write_str("a schema version is at least 1"),
            Self::DuplicateHandOffTarget(target) => {
                write!(formatter, "the hand-off target {target} is listed twice")
            }
            Self::DuplicateUnseatedAction(action) => write!(
                formatter,
                "the unseated action {} is listed twice",
                action.as_str()
            ),
            Self::DuplicateRulesAction(action) => {
                write!(
                    formatter,
                    "the rules action {} is listed twice",
                    action.as_str()
                )
            }
        }
    }
}

impl std::error::Error for SpecError {}

/// The declaration a machine registers with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineSpec {
    id: MachineId,
    scope: Scope,
    schema_version: NonZeroU32,
    states: Vec<StateName>,
    hand_off_targets: Vec<MachineId>,
    required_ports: Vec<PortName>,
    rules_actions: Vec<ActionId>,
    unseated_actions: Vec<ActionId>,
    claimed_inputs: Vec<InputClaim>,
    reads_entry: bool,
    on_suspend: SuspendPolicy,
}

impl MachineSpec {
    /// A spec with no hand-off targets, no required ports and no rules actions; the `handing_off_to`,
    /// `requiring_port` and `with_rules_action` methods add them.
    pub fn new(
        id: &str,
        scope: Scope,
        schema_version: u32,
        state_names: &[&str],
        on_suspend: SuspendPolicy,
    ) -> Result<Self, SpecError> {
        let id = MachineId::new(id).map_err(SpecError::InvalidName)?;
        let schema_version = NonZeroU32::new(schema_version).ok_or(SpecError::ZeroSchemaVersion)?;
        let mut states = Vec::with_capacity(state_names.len());
        for text in state_names {
            let name = StateName::new(text).map_err(SpecError::InvalidName)?;
            if states.contains(&name) {
                return Err(SpecError::DuplicateState(name));
            }
            states.push(name);
        }
        if states.is_empty() {
            return Err(SpecError::NoStates);
        }
        Ok(Self {
            id,
            scope,
            schema_version,
            states,
            hand_off_targets: Vec::new(),
            required_ports: Vec::new(),
            rules_actions: Vec::new(),
            unseated_actions: Vec::new(),
            claimed_inputs: Vec::new(),
            reads_entry: false,
            on_suspend,
        })
    }

    /// Declares a machine this one may hand off to.
    pub fn handing_off_to(mut self, target: &str) -> Result<Self, SpecError> {
        let target = MachineId::new(target).map_err(SpecError::InvalidName)?;
        if self.hand_off_targets.contains(&target) {
            return Err(SpecError::DuplicateHandOffTarget(target));
        }
        self.hand_off_targets.push(target);
        Ok(self)
    }

    /// Declares a port the machine's decisions need; declaring one twice is the same declaration.
    pub fn requiring_port(mut self, port: PortName) -> Self {
        if !self.required_ports.contains(&port) {
            self.required_ports.push(port);
        }
        self
    }

    /// Declares an action the pipeline sends to the rules owner before it calls `step`. This list
    /// stands in for the action registry of O.5 until it exists.
    pub fn with_rules_action(mut self, action: ActionId) -> Result<Self, SpecError> {
        if self.rules_actions.contains(&action) {
            return Err(SpecError::DuplicateRulesAction(action));
        }
        self.rules_actions.push(action);
        Ok(self)
    }

    /// Declares an action a seat that is not at the table may send (the lobby's `join`). Every
    /// other action of an unseated seat must be declined.
    pub fn with_unseated_action(mut self, action: ActionId) -> Result<Self, SpecError> {
        if self.unseated_actions.contains(&action) {
            return Err(SpecError::DuplicateUnseatedAction(action));
        }
        self.unseated_actions.push(action);
        Ok(self)
    }

    /// Declares that the machine handles this kind of engine input; claiming it twice is the
    /// same claim.
    pub fn claiming(mut self, claim: InputClaim) -> Self {
        if !self.claimed_inputs.contains(&claim) {
            self.claimed_inputs.push(claim);
        }
        self
    }

    /// Declares that `enter` reads the entry it is given, so the suite needs an entry the table
    /// disagrees with to prove it refuses one.
    pub fn reading_entry(mut self) -> Self {
        self.reads_entry = true;
        self
    }

    pub fn id(&self) -> &MachineId {
        &self.id
    }

    pub fn scope(&self) -> Scope {
        self.scope
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version.get()
    }

    pub fn states(&self) -> &[StateName] {
        &self.states
    }

    pub fn hand_off_targets(&self) -> &[MachineId] {
        &self.hand_off_targets
    }

    pub fn required_ports(&self) -> &[PortName] {
        &self.required_ports
    }

    pub fn rules_actions(&self) -> &[ActionId] {
        &self.rules_actions
    }

    pub fn unseated_actions(&self) -> &[ActionId] {
        &self.unseated_actions
    }

    pub fn claimed_inputs(&self) -> &[InputClaim] {
        &self.claimed_inputs
    }

    pub fn reads_entry(&self) -> bool {
        self.reads_entry
    }

    pub fn on_suspend(&self) -> SuspendPolicy {
        self.on_suspend
    }
}
