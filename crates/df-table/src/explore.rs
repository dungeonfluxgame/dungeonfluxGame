//! The minimal explore machine: one authored beat and one check (RES28, RES31, question 90).
//!
//! The group machine of a scene, in its smallest honest form. It shows one authored beat and
//! offers one check on it. The check is a rules action: the pipeline sends it to the rules owner
//! and calls `step` with the decision (RES27), so this machine proposes the check and the rules
//! owner settles it. When the decision is a roll the player must make, the machine reports
//! `Waiting{pending_id}` and holds only that id (RES26); the answer comes back as a `Dice` input
//! for the same pending. Resolving the beat leaves as a `BeatResolved` effect and a request to
//! hand the table back to `lobby`. It owns three states (`presenting`, `awaiting_roll`,
//! `resolved`).
//!
//! `Resolution` says only whether the rules applied the action, so this machine treats an applied
//! check as made; it never compares a total with a difficulty, because the rules owner decides
//! mechanics. It declines any other input `not_viable`; spells, hazards, light and searching are
//! the rules adapter's `not_supported_yet` (RES31). It never imports another machine, draws a
//! die, reads a clock or calls a port.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::effects::{BeatName, TableEffect, TableState};
use crate::element::NameError;
use crate::identity::{SceneId, SeatId};
use crate::machine::{
    CarryItem, Decision, Entry, EntryRefusal, HandOff, InspectReport, Machine, MachineError,
    MachineId, MachineInput, MachineReadout, MachineSpec, Scope, SpecError, StateName, StepInput,
    StepOutcome, StepResult, SuspendPolicy,
};
use crate::machine_slot::MachineSlot;
use crate::outcome::{Decline, DeclineCode};
use crate::ports::{ActionId, Audience, PortName, Resolution};

const EXPLORE_SCHEMA_VERSION: u32 = 1;
const PRESENTING: &str = "presenting";
const AWAITING_ROLL: &str = "awaiting_roll";
const RESOLVED: &str = "resolved";
const NEXT_MACHINE: &str = "lobby";

/// The authored content of the one beat: its id and the check it offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredBeat {
    id: BeatName,
    check: ActionId,
}

impl AuthoredBeat {
    pub fn new(id: &str, check: &str) -> Result<Self, NameError> {
        Ok(Self {
            id: BeatName::new(id)?,
            check: ActionId::new(check)?,
        })
    }
}

/// The explore machine's closed states. The seats are those carried in, sorted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum ExploreState {
    Presenting {
        scene: SceneId,
        seats: Vec<SeatId>,
    },
    AwaitingRoll {
        scene: SceneId,
        seats: Vec<SeatId>,
        roller: SeatId,
        pending: u64,
    },
    Resolved {
        scene: SceneId,
        seats: Vec<SeatId>,
        beat: BeatName,
    },
}

impl ExploreState {
    fn name(&self) -> &'static str {
        match self {
            Self::Presenting { .. } => PRESENTING,
            Self::AwaitingRoll { .. } => AWAITING_ROLL,
            Self::Resolved { .. } => RESOLVED,
        }
    }

    fn scene(&self) -> SceneId {
        match self {
            Self::Presenting { scene, .. }
            | Self::AwaitingRoll { scene, .. }
            | Self::Resolved { scene, .. } => *scene,
        }
    }

    fn seats(&self) -> &[SeatId] {
        match self {
            Self::Presenting { seats, .. }
            | Self::AwaitingRoll { seats, .. }
            | Self::Resolved { seats, .. } => seats,
        }
    }
}

/// The explore machine over one authored beat.
#[derive(Debug, Clone)]
pub struct ExploreMachine {
    spec: MachineSpec,
    next_machine: MachineId,
    beat: AuthoredBeat,
}

impl ExploreMachine {
    pub fn new(beat: AuthoredBeat) -> Result<Self, SpecError> {
        let spec = MachineSpec::new(
            "explore",
            Scope::Group,
            EXPLORE_SCHEMA_VERSION,
            &[PRESENTING, AWAITING_ROLL, RESOLVED],
            SuspendPolicy::FinishAndHold,
        )?
        .handing_off_to(NEXT_MACHINE)?
        .requiring_port(PortName::RulesOwner)
        .with_rules_action(beat.check.clone())?;
        let next_machine = MachineId::new(NEXT_MACHINE).map_err(SpecError::InvalidName)?;
        Ok(Self {
            spec,
            next_machine,
            beat,
        })
    }

    fn state_of(&self, slot: &MachineSlot) -> Result<ExploreState, MachineError> {
        Ok(slot.decode(&self.spec)?)
    }

    fn encode(&self, state: &ExploreState) -> Result<MachineSlot, MachineError> {
        Ok(MachineSlot::encode(&self.spec, state)?)
    }

    /// Whether the input is one the machine handles in this state.
    fn handles(&self, state: &ExploreState, input: &MachineInput) -> bool {
        match (state, input) {
            (ExploreState::Presenting { seats, .. }, MachineInput::Intent { seat, action }) => {
                *action == self.beat.check && seats.contains(seat)
            }
            (
                ExploreState::AwaitingRoll { pending, .. },
                MachineInput::Dice {
                    pending: answered, ..
                },
            ) => answered.get() == *pending,
            _ => false,
        }
    }

    fn resolve(&self, state: &ExploreState) -> Result<StepResult, MachineError> {
        let scene = state.scene();
        let next = self.encode(&ExploreState::Resolved {
            scene,
            seats: state.seats().to_vec(),
            beat: self.beat.id.clone(),
        })?;
        let effects = vec![TableEffect::BeatResolved {
            scene,
            beat: self.beat.id.clone(),
        }];
        Ok(StepResult::applied(next, effects).asking_for_hand_off())
    }
}

impl Machine for ExploreMachine {
    fn spec(&self) -> &MachineSpec {
        &self.spec
    }

    fn enter(&self, table: &TableState, entry: &Entry) -> Result<MachineSlot, MachineError> {
        let scene = entry.scene().ok_or(EntryRefusal::MissingScene)?;
        if table.active_scene() != Some(scene) {
            return Err(EntryRefusal::SceneNotActive { scene }.into());
        }
        let seats: BTreeSet<SeatId> = entry.seats().collect();
        if seats.is_empty() {
            return Err(EntryRefusal::NoSeats.into());
        }
        if let Some(seat) = seats.iter().find(|seat| !table.seats().contains(seat)) {
            return Err(EntryRefusal::SeatNotAtTable { seat: *seat }.into());
        }
        self.encode(&ExploreState::Presenting {
            scene,
            seats: seats.into_iter().collect(),
        })
    }

    fn accepts(&self, input: &MachineInput, state: &MachineSlot) -> bool {
        self.state_of(state)
            .is_ok_and(|state| self.handles(&state, input))
    }

    fn step(&self, slot: &MachineSlot, input: &StepInput) -> Result<StepResult, MachineError> {
        let state = self.state_of(slot)?;
        let not_viable = || {
            Ok(StepResult::declined(
                slot,
                Decline::with_default_reason(DeclineCode::NotViable),
            ))
        };
        if !self.handles(&state, &input.input) {
            return not_viable();
        }
        match (&state, &input.input, &input.decision) {
            (
                ExploreState::Presenting { scene, seats },
                MachineInput::Intent { seat, .. },
                Some(Decision::RollPending(pending)),
            ) => {
                let next = self.encode(&ExploreState::AwaitingRoll {
                    scene: *scene,
                    seats: seats.clone(),
                    roller: *seat,
                    pending: pending.get(),
                })?;
                let mut result = StepResult::applied(next, Vec::new());
                result.outcome = StepOutcome::Waiting(*pending);
                Ok(result)
            }
            (_, _, Some(Decision::Resolved(Resolution::Applied))) => self.resolve(&state),
            (_, _, Some(Decision::Resolved(Resolution::Declined(decline)))) => {
                let unchanged = match &state {
                    ExploreState::AwaitingRoll { scene, seats, .. } => {
                        self.encode(&ExploreState::Presenting {
                            scene: *scene,
                            seats: seats.clone(),
                        })?
                    }
                    ExploreState::Presenting { .. } | ExploreState::Resolved { .. } => slot.clone(),
                };
                Ok(StepResult::declined(&unchanged, decline.clone()))
            }
            _ => not_viable(),
        }
    }

    fn exit(&self, slot: &MachineSlot) -> Result<HandOff, MachineError> {
        let state = self.state_of(slot)?;
        let mut carry = vec![CarryItem::Scene(state.scene())];
        carry.extend(state.seats().iter().copied().map(CarryItem::Seat));
        Ok(HandOff::new(self.next_machine.clone(), carry)?)
    }

    fn state_name(&self, slot: &MachineSlot) -> Result<StateName, MachineError> {
        Ok(StateName::new(self.state_of(slot)?.name())?)
    }

    fn project(
        &self,
        slot: &MachineSlot,
        audience: Audience,
    ) -> Result<MachineReadout, MachineError> {
        let beat = self.beat.id.as_str();
        let text = match (self.state_of(slot)?, audience) {
            (ExploreState::Presenting { seats, .. }, Audience::Seat(seat))
                if seats.contains(&seat) =>
            {
                format!(
                    "You can make the {} check at {beat}.",
                    self.beat.check.as_str()
                )
            }
            (ExploreState::Presenting { .. }, _) => {
                format!("The party is at {beat}. One check is offered.")
            }
            (ExploreState::AwaitingRoll { .. }, _) => {
                format!("The party is at {beat}. A roll is waiting.")
            }
            (ExploreState::Resolved { .. }, _) => format!("{beat} is done."),
        };
        Ok(MachineReadout::new(&text)?)
    }

    fn inspect(&self, slot: &MachineSlot) -> Result<InspectReport, MachineError> {
        Ok(InspectReport {
            machine: self.spec.id().clone(),
            state: self.state_name(slot)?,
            schema_version: slot.schema_version(),
        })
    }
}
