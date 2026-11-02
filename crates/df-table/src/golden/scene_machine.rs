//! A group machine for golden sessions and pipeline tests: it takes any action it was given.
//!
//! It owns `SceneMachine`, a scene that accepts the actions it was built with, counts what
//! happened to it (turns taken, ticks, resource results, proposals) and so changes its saved state
//! on every input, which makes the state digest of a golden session move. An action is either
//! committing (an applied step leaves as a `BeatResolved` effect named after the action, standing
//! in for the real change such as a hit point movement) or quiet (applied, nothing committed).
//! It reports `Waiting{pending_id}` when the pipeline opens a pending for an action and goes back
//! to open when the dice of that pending arrive. It never decides a rule: the rules owner does.
//! It fails with the same typed errors as every machine and never panics.

use serde::{Deserialize, Serialize};

use crate::effects::{BeatName, TableEffect, TableState};
use crate::element::NameError;
use crate::identity::SceneId;
use crate::machine::{
    Decision, Entry, HandOff, InspectReport, Machine, MachineError, MachineId, MachineInput,
    MachineReadout, MachineSpec, RequestKey, ResourceRequest, Scope, SpecError, StateName,
    StepInput, StepOutcome, StepResult, SuspendPolicy,
};
use crate::machine_slot::MachineSlot;
use crate::machine_spec::InputClaim;
use crate::outcome::{Decline, DeclineCode, Outcome, PendingKind};
use crate::ports::{ActionId, Audience, PortName, PurposeName};
use crate::table_state::MachineHome;

const SCENE_SCHEMA_VERSION: u32 = 1;
const PHASE_TAG: &str = "phase";
const OPEN: &str = "open";
const WAITING: &str = "waiting";
const NEXT_MACHINE: &str = "lobby";

/// What an applied action leaves behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leaves {
    /// A `BeatResolved` effect named after the action.
    Effect,
    /// Nothing but the turn count.
    Nothing,
    /// The party enters this scene, and the scene machine runs there: `SceneEntered` and
    /// `ActiveMachineSet` with a fresh state. The scene machine's own state does not change.
    Enters(u64),
    /// A machine that breaks its contract: the step says it settled as `Pending`, which only the
    /// pipeline may open (O.7).
    SettlesPending,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Counters {
    turns: u64,
    ticks: u64,
    resources: u64,
    proposals: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum SceneState {
    Open {
        counters: Counters,
    },
    Waiting {
        pending: u64,
        action: String,
        counters: Counters,
    },
}

/// The scene machine of golden sessions.
#[derive(Debug, Clone)]
pub struct SceneMachine {
    spec: MachineSpec,
    actions: Vec<(ActionId, Leaves)>,
    finishing_after: Option<u64>,
    request: Option<(ActionId, PurposeName, RequestKey)>,
    next_machine: MachineId,
}

impl SceneMachine {
    /// A scene that takes these actions. It claims timers and engine proposals.
    pub fn new(actions: &[(&str, Leaves)]) -> Result<Self, SpecError> {
        let mut spec = MachineSpec::new(
            "scene",
            Scope::Group,
            SCENE_SCHEMA_VERSION,
            &[OPEN, WAITING],
            SuspendPolicy::FinishAndHold,
        )?
        .handing_off_to(NEXT_MACHINE)?
        .requiring_port(PortName::RulesOwner)
        .claiming(InputClaim::Timer)
        .claiming(InputClaim::EngineProposal);
        let mut held = Vec::with_capacity(actions.len());
        for (name, leaves) in actions {
            let action = ActionId::new(name).map_err(SpecError::InvalidName)?;
            spec = spec.with_rules_action(action.clone())?;
            held.push((action, *leaves));
        }
        let next_machine = MachineId::new(NEXT_MACHINE).map_err(SpecError::InvalidName)?;
        Ok(Self {
            spec,
            actions: held,
            finishing_after: None,
            request: None,
            next_machine,
        })
    }

    /// Applying `action` also asks the broker for `purpose`, under `key`.
    pub fn requesting_on(
        mut self,
        action: &str,
        purpose: &str,
        key: u64,
    ) -> Result<Self, NameError> {
        self.request = Some((
            ActionId::new(action)?,
            PurposeName::new(purpose)?,
            RequestKey::new(key),
        ));
        Ok(self)
    }

    /// The scene asks for a hand-off to the lobby once it has taken this many turns.
    pub fn finishing_after(mut self, turns: u64) -> Self {
        self.finishing_after = Some(turns);
        self
    }

    fn state_of(&self, slot: &MachineSlot) -> Result<SceneState, MachineError> {
        Ok(slot.decode(&self.spec)?)
    }

    fn encode(&self, state: &SceneState) -> Result<MachineSlot, MachineError> {
        Ok(MachineSlot::encode(&self.spec, state)?)
    }

    fn leaves(&self, action: &ActionId) -> Option<Leaves> {
        self.actions
            .iter()
            .find(|(known, _)| known == action)
            .map(|(_, leaves)| *leaves)
    }

    fn effect_for(
        &self,
        table: &TableState,
        action: &str,
        leaves: Leaves,
    ) -> Result<Vec<TableEffect>, NameError> {
        let scene = match (leaves, table.active_scene()) {
            (Leaves::Effect, Some(scene)) => scene,
            (Leaves::Effect, None)
            | (Leaves::Nothing | Leaves::Enters(_) | Leaves::SettlesPending, _) => {
                return Ok(Vec::new());
            }
        };
        Ok(vec![TableEffect::BeatResolved {
            scene,
            beat: BeatName::new(action)?,
        }])
    }
}

fn counters_of(state: &SceneState) -> Counters {
    match state {
        SceneState::Open { counters } | SceneState::Waiting { counters, .. } => counters.clone(),
    }
}

fn not_viable() -> StepResult {
    StepResult::declined(Decline::with_default_reason(DeclineCode::NotViable))
}

impl Machine for SceneMachine {
    fn spec(&self) -> &MachineSpec {
        &self.spec
    }

    fn enter(&self, _table: &TableState, _entry: &Entry) -> Result<MachineSlot, MachineError> {
        self.encode(&SceneState::Open {
            counters: Counters::default(),
        })
    }

    fn accepts(&self, table: &TableState, input: &MachineInput, state: &MachineSlot) -> bool {
        let Ok(state) = self.state_of(state) else {
            return false;
        };
        match (&state, input) {
            (SceneState::Open { .. }, MachineInput::Intent { seat, action }) => {
                self.leaves(action).is_some() && table.seats().contains(seat)
            }
            (
                SceneState::Waiting { pending, .. },
                MachineInput::PendingCancelled { pending: cancelled },
            ) => cancelled.get() == *pending,
            (
                SceneState::Waiting { pending, .. },
                MachineInput::Dice {
                    pending: answered, ..
                },
            ) => answered.get() == *pending,
            (_, MachineInput::Timer { .. } | MachineInput::Proposal { .. }) => true,
            (_, MachineInput::ResourceResult { .. }) => true,
            _ => false,
        }
    }

    fn step(&self, slot: &MachineSlot, input: &StepInput<'_>) -> Result<StepResult, MachineError> {
        let state = self.state_of(slot)?;
        let mut counters = counters_of(&state);
        match (&state, &input.input) {
            (SceneState::Open { .. }, MachineInput::Intent { action, .. }) => {
                let Some(leaves) = self.leaves(action) else {
                    return Ok(not_viable());
                };
                match &input.decision {
                    Some(Decision::RollPending(pending) | Decision::Pending(pending)) => {
                        let waiting = SceneState::Waiting {
                            pending: pending.get(),
                            action: action.as_str().to_owned(),
                            counters,
                        };
                        let mut result = StepResult::applied(self.encode(&waiting)?, Vec::new());
                        result.outcome = StepOutcome::Waiting(*pending);
                        Ok(result)
                    }
                    Some(Decision::Resolved(crate::ports::Resolution::Declined(decline))) => {
                        Ok(StepResult::declined(decline.clone()))
                    }
                    Some(Decision::Resolved(_)) | None => {
                        if let Leaves::Enters(scene) = leaves {
                            return self.enter_scene(SceneId::new(scene));
                        }
                        if leaves == Leaves::SettlesPending {
                            let next = self.encode(&SceneState::Open { counters })?;
                            let mut result = StepResult::applied(next, Vec::new());
                            result.outcome =
                                StepOutcome::Settled(Outcome::Pending(PendingKind::Choice));
                            return Ok(result);
                        }
                        counters.turns += 1;
                        let effects = self.effect_for(input.table, action.as_str(), leaves)?;
                        let next = self.encode(&SceneState::Open { counters })?;
                        let mut result = StepResult::applied(next, effects);
                        if let Some((asked, purpose, key)) = &self.request
                            && asked == action
                        {
                            result.resource_requests.push(ResourceRequest {
                                key: *key,
                                purpose: purpose.clone(),
                            });
                        }
                        Ok(result)
                    }
                }
            }
            (
                SceneState::Waiting {
                    pending, action, ..
                },
                MachineInput::Dice {
                    pending: answered, ..
                },
            ) if answered.get() == *pending => {
                counters.turns += 1;
                let leaves = ActionId::new(action)
                    .ok()
                    .and_then(|known| self.leaves(&known))
                    .unwrap_or(Leaves::Nothing);
                let effects = self.effect_for(input.table, action, leaves)?;
                let next = self.encode(&SceneState::Open { counters })?;
                Ok(StepResult::applied(next, effects))
            }
            (SceneState::Waiting { .. }, MachineInput::PendingCancelled { .. }) => {
                let next = self.encode(&SceneState::Open { counters })?;
                Ok(StepResult::applied(next, Vec::new()))
            }
            (_, MachineInput::Timer { .. }) => {
                counters.ticks += 1;
                self.counted(&state, counters)
            }
            (_, MachineInput::ResourceResult { .. }) => {
                counters.resources += 1;
                self.counted(&state, counters)
            }
            (_, MachineInput::Proposal { .. }) => {
                counters.proposals += 1;
                self.counted(&state, counters)
            }
            _ => Ok(not_viable()),
        }
    }

    fn wants_hand_off(&self, slot: &MachineSlot) -> Result<bool, MachineError> {
        let turns = counters_of(&self.state_of(slot)?).turns;
        Ok(self.finishing_after.is_some_and(|after| turns >= after))
    }

    fn exit(&self, _table: &TableState, _slot: &MachineSlot) -> Result<HandOff, MachineError> {
        Ok(HandOff::new(self.next_machine.clone(), Vec::new())?)
    }

    fn state_name(&self, slot: &MachineSlot) -> Result<StateName, MachineError> {
        Ok(slot.named_state(&self.spec, PHASE_TAG)?)
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
        let text = match self.state_of(slot)? {
            SceneState::Open { .. } => "The scene is open.",
            SceneState::Waiting { .. } => "The scene is waiting for an answer.",
        };
        Ok(MachineReadout::new(text)?)
    }

    fn inspect(&self, slot: &MachineSlot) -> Result<InspectReport, MachineError> {
        Ok(InspectReport {
            machine: self.spec.id().clone(),
            state: self.state_name(slot)?,
            schema_version: slot.schema_version(),
        })
    }
}

impl SceneMachine {
    /// The party enters a scene and this machine runs there with a fresh state. The step changes
    /// no state of its own, so the commit adds no slot update for the scene that closed.
    fn enter_scene(&self, scene: SceneId) -> Result<StepResult, MachineError> {
        let fresh = self.encode(&SceneState::Open {
            counters: Counters::default(),
        })?;
        let effects = vec![
            TableEffect::SceneEntered { scene },
            TableEffect::ActiveMachineSet {
                home: MachineHome::Scene { scene },
                machine: self.spec.id().clone(),
                slot: fresh,
            },
        ];
        Ok(StepResult::applied_in_place(effects))
    }

    /// An applied input that only changed the counters, in the phase the scene was in.
    fn counted(&self, state: &SceneState, counters: Counters) -> Result<StepResult, MachineError> {
        let next = match state {
            SceneState::Open { .. } => SceneState::Open { counters },
            SceneState::Waiting {
                pending, action, ..
            } => SceneState::Waiting {
                pending: *pending,
                action: action.clone(),
                counters,
            },
        };
        let mut result = StepResult::applied(self.encode(&next)?, Vec::new());
        result.outcome = StepOutcome::Settled(Outcome::Applied);
        Ok(result)
    }
}
