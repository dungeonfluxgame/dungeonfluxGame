//! The minimal explore machine: one authored beat and one check (RES28, RES31, question 90).
//!
//! The group machine of a scene, in its smallest honest form. It shows one authored beat and
//! offers one check on it. The check is a rules action: the pipeline sends it to the rules owner
//! and calls `step` with the decision (RES27), so this machine proposes the check and the rules
//! owner settles it. When the decision is a roll the player must make, the machine reports
//! `Waiting{pending_id}` and holds only that id (RES26); the answer comes back as a `Dice` input
//! for the same pending. Resolving the beat leaves as a `BeatResolved` effect, and the machine is
//! then finished: it asks to hand the table back to `lobby`, for as long as the table has not
//! moved on. A decline from the rules owner changes nothing, so a declined roll answer leaves the
//! pending open for the pending handler (C4) to cancel or re-ask. The seats at the scene are
//! read from the table on every call, so a seat that left cannot act. It owns three states
//! (`presenting`, `awaiting_roll`, `resolved`).
//!
//! `Resolution` says only whether the rules applied the action, so this machine treats an applied
//! check as made; it never compares a total with a difficulty, because the rules owner decides
//! mechanics. It declines any other input `not_viable`; spells, hazards, light and searching are
//! the rules adapter's `not_supported_yet` (RES31). Going back to the lobby is the test pair's
//! flow, not a plan row (see the plan's note on explore). It never imports another machine,
//! draws a die, reads a clock or calls a port.

use serde::{Deserialize, Serialize};

use crate::effects::{BeatName, TableEffect, TableState};
use crate::element::NameError;
use crate::identity::{SceneId, SeatId};
use crate::machine::slot::MachineSlot;
use crate::machine::{
    CarryItem, Decision, Entry, EntryError, HandOff, InspectReport, Machine, MachineError,
    MachineId, MachineInput, MachineReadout, MachineSpec, Scope, SpecError, StandingOffers,
    StateName, StepInput, StepOutcome, StepResult, SuspendPolicy, rules_action_offers,
};
use crate::outcome::{Decline, DeclineCode};
use crate::ports::{ActionId, Audience, PortName, Resolution};

const EXPLORE_SCHEMA_VERSION: u32 = 1;
const PHASE_TAG: &str = "phase";
const PRESENTING: &str = "presenting";
const AWAITING_ROLL: &str = "awaiting_roll";
const RESOLVED: &str = "resolved";
/// The explore machine's id, the name its spec registers and hand-offs name.
pub const MACHINE_ID: &str = "explore";
/// The machine explore hands back to by default, named by id only: machines never import one
/// another (the isolation rule `machine_scan_tests` enforces).
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

/// The explore machine's closed states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum ExploreState {
    Presenting { scene: SceneId },
    AwaitingRoll { scene: SceneId, pending: u64 },
    Resolved { scene: SceneId, beat: BeatName },
}

impl ExploreState {
    fn scene(&self) -> SceneId {
        match self {
            Self::Presenting { scene }
            | Self::AwaitingRoll { scene, .. }
            | Self::Resolved { scene, .. } => *scene,
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
        Self::followed_by(beat, NEXT_MACHINE)
    }

    /// The machine with the machine it hands the table to when the beat is done: `new` hands it
    /// back to the lobby, and a story that goes on (the first playable path hands the table to its
    /// conversation) names the next scene machine here. The registry checks the target.
    pub fn followed_by(beat: AuthoredBeat, next: &str) -> Result<Self, SpecError> {
        let spec = MachineSpec::new(
            MACHINE_ID,
            Scope::Group,
            EXPLORE_SCHEMA_VERSION,
            &[PRESENTING, AWAITING_ROLL, RESOLVED],
            SuspendPolicy::FinishAndHold,
        )?
        .handing_off_to(next)?
        .requiring_port(PortName::RulesOwner)
        .with_rules_action(beat.check.clone())?
        .reading_entry();
        let next_machine = MachineId::new(next).map_err(SpecError::InvalidName)?;
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

    /// Whether the input is one the machine handles in this state, for the table as it is now.
    fn handles(&self, table: &TableState, state: &ExploreState, input: &MachineInput) -> bool {
        match (state, input) {
            (ExploreState::Presenting { .. }, MachineInput::Intent { seat, action }) => {
                *action == self.beat.check && table.seats().contains(seat)
            }
            (
                ExploreState::AwaitingRoll { pending, .. },
                MachineInput::Dice {
                    pending: answered, ..
                },
            ) => answered.get() == *pending,
            (
                ExploreState::AwaitingRoll { pending, .. },
                MachineInput::PendingCancelled { pending: cancelled },
            ) => cancelled.get() == *pending,
            _ => false,
        }
    }

    fn settle(&self, state: &ExploreState) -> Result<StepResult, MachineError> {
        let scene = state.scene();
        let next = self.encode(&ExploreState::Resolved {
            scene,
            beat: self.beat.id.clone(),
        })?;
        let effects = vec![TableEffect::BeatResolved {
            scene,
            beat: self.beat.id.clone(),
        }];
        Ok(StepResult::applied(next, effects))
    }
}

impl Machine for ExploreMachine {
    fn spec(&self) -> &MachineSpec {
        &self.spec
    }

    fn enter(&self, table: &TableState, entry: &Entry) -> Result<MachineSlot, MachineError> {
        let scene = entry.scene().ok_or(EntryError::MissingScene)?;
        if table.active_scene() != Some(scene) {
            return Err(EntryError::SceneNotActive { scene }.into());
        }
        let mut seats = entry.seats().peekable();
        if seats.peek().is_none() {
            return Err(EntryError::NoSeats.into());
        }
        if let Some(seat) = seats.find(|seat| !table.seats().contains(seat)) {
            return Err(EntryError::SeatNotAtTable { seat }.into());
        }
        self.encode(&ExploreState::Presenting { scene })
    }

    fn accepts(&self, table: &TableState, input: &MachineInput, state: &MachineSlot) -> bool {
        self.state_of(state)
            .is_ok_and(|state| self.handles(table, &state, input))
    }

    fn step(&self, slot: &MachineSlot, input: &StepInput<'_>) -> Result<StepResult, MachineError> {
        let state = self.state_of(slot)?;
        let not_viable = || {
            Ok(StepResult::declined(Decline::with_default_reason(
                DeclineCode::NotViable,
            )))
        };
        if !self.handles(input.table, &state, &input.input) {
            return not_viable();
        }
        if let (ExploreState::AwaitingRoll { scene, .. }, MachineInput::PendingCancelled { .. }) =
            (&state, &input.input)
        {
            let back = self.encode(&ExploreState::Presenting { scene: *scene })?;
            return Ok(StepResult::applied(back, Vec::new()));
        }
        match (&state, &input.decision) {
            (ExploreState::Presenting { scene }, Some(Decision::RollPending(pending))) => {
                let next = self.encode(&ExploreState::AwaitingRoll {
                    scene: *scene,
                    pending: pending.get(),
                })?;
                let mut result = StepResult::applied(next, Vec::new());
                result.outcome = StepOutcome::Waiting(*pending);
                Ok(result)
            }
            (_, Some(Decision::Resolved(Resolution::Applied(_)))) => self.settle(&state),
            (_, Some(Decision::Resolved(Resolution::Declined(decline)))) => {
                Ok(StepResult::declined(decline.clone()))
            }
            _ => not_viable(),
        }
    }

    fn standing_offers(
        &self,
        table: &TableState,
        slot: &MachineSlot,
        seats: &[SeatId],
    ) -> Result<StandingOffers, MachineError> {
        let state = self.state_of(slot)?;
        Ok(rules_action_offers(&self.spec, seats, |input| {
            self.handles(table, &state, input)
        }))
    }

    fn wants_hand_off(&self, slot: &MachineSlot) -> Result<bool, MachineError> {
        Ok(self.state_name(slot)?.as_str() == RESOLVED)
    }

    fn exit(&self, table: &TableState, slot: &MachineSlot) -> Result<HandOff, MachineError> {
        let scene = self.state_of(slot)?.scene();
        let carry = std::iter::once(CarryItem::Scene(scene))
            .chain(table.seats().iter().copied().map(CarryItem::Seat))
            .collect();
        Ok(HandOff::new(self.next_machine.clone(), carry)?)
    }

    fn state_name(&self, slot: &MachineSlot) -> Result<StateName, MachineError> {
        Ok(slot.named_state(&self.spec, PHASE_TAG)?)
    }

    fn validate(&self, slot: &MachineSlot) -> Result<(), MachineError> {
        self.state_of(slot).map(|_| ())
    }

    fn project(
        &self,
        table: &TableState,
        slot: &MachineSlot,
        audience: Audience,
    ) -> Result<MachineReadout, MachineError> {
        let beat = self.beat.id.as_str();
        let text = match (self.state_of(slot)?, audience) {
            (ExploreState::Presenting { .. }, Audience::Seat(seat))
                if table.seats().contains(&seat) =>
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
