//! The lobby: the table machine before the first scene (RES28, D.4, question 90).
//!
//! Seats join and leave, say they are ready, and one of them starts the game. It owns two states
//! (`gathering` and `started`), the five lobby actions, and the lobby's own fact: which seats said
//! they are ready. Which seats sit at the table, and how many fit, are shared facts that the table
//! owns, so the lobby reads them from the table view on every call and never keeps a copy that can
//! go stale: a seat the host removed cannot hold up a start, and a join is judged against the
//! seats now. A join and a leave leave as `SeatJoined` and `SeatLeft` effects, and starting
//! leaves as `SceneEntered` for the first scene; the lobby never reads or writes another
//! machine's state.
//!
//! It models `df_session::lobby::decide_start` by reading it, without depending on `df-session`:
//! a start needs at least one seat and every seat ready. `decide_start` returns every blocker at
//! once; the machine declines with the first it finds. The campaign, character, display, funding
//! and authorization blockers of that function belong to the creation machine and the host (a
//! seat that is not authorized never reaches the table), not to this machine. The machine is
//! finished once it has started, and it then asks for a hand-off to `explore` carrying the seats
//! and the first scene, for as long as the table has not moved on.
//!
//! It never draws a die, reads a clock or calls a port, and it declines (never panics on) an
//! input it does not handle.

use serde::{Deserialize, Serialize};

use crate::effects::{TableEffect, TableState};
use crate::identity::{SceneId, SeatId};
use crate::listed::listed_enum;
use crate::machine::{
    CarryItem, Entry, HandOff, InspectReport, Machine, MachineError, MachineId, MachineInput,
    MachineReadout, MachineSpec, Scope, SpecError, StateName, StepInput, StepResult, SuspendPolicy,
};
use crate::machine_slot::MachineSlot;
use crate::outcome::{Decline, DeclineCode};
use crate::ports::{ActionId, Audience};

const LOBBY_SCHEMA_VERSION: u32 = 1;
const PHASE_TAG: &str = "phase";
const GATHERING: &str = "gathering";
const STARTED: &str = "started";
const NEXT_MACHINE: &str = "explore";

const ALREADY_SEATED: &str = "You already have a seat.";
const TABLE_FULL: &str = "The table is full.";
const NOT_SEATED: &str = "Take a seat first.";
const NOBODY_READY: &str = "Everyone at the table needs to be ready first.";
const ALL_READY: &str = "every seat ready";

listed_enum! {
    /// The actions the lobby handles.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum LobbyAction {
        Join,
        Leave,
        Ready,
        Unready,
        Start,
    }
}

impl LobbyAction {
    pub fn key(self) -> &'static str {
        match self {
            Self::Join => "join",
            Self::Leave => "leave",
            Self::Ready => "ready",
            Self::Unready => "unready",
            Self::Start => "start",
        }
    }

    /// The lobby action a registry name stands for, if it is one.
    pub fn of(action: &ActionId) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.key() == action.as_str())
    }
}

/// The lobby's closed states. The ready seats stay sorted so equal tables give equal records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum LobbyState {
    Gathering { ready: Vec<SeatId> },
    Started { scene: SceneId },
}

/// The lobby machine. `first_scene` is the scene the game starts in: authored content, handed in
/// when the machine is registered.
#[derive(Debug, Clone)]
pub struct LobbyMachine {
    spec: MachineSpec,
    next_machine: MachineId,
    first_scene: SceneId,
}

impl LobbyMachine {
    pub fn new(first_scene: SceneId) -> Result<Self, SpecError> {
        let join = ActionId::new(LobbyAction::Join.key()).map_err(SpecError::InvalidName)?;
        let spec = MachineSpec::new(
            "lobby",
            Scope::Table,
            LOBBY_SCHEMA_VERSION,
            &[GATHERING, STARTED],
            SuspendPolicy::FinishAndHold,
        )?
        .handing_off_to(NEXT_MACHINE)?
        .with_unseated_action(join)?;
        let next_machine = MachineId::new(NEXT_MACHINE).map_err(SpecError::InvalidName)?;
        Ok(Self {
            spec,
            next_machine,
            first_scene,
        })
    }

    fn state_of(&self, slot: &MachineSlot) -> Result<LobbyState, MachineError> {
        Ok(slot.decode(&self.spec)?)
    }

    fn encode(&self, state: &LobbyState) -> Result<MachineSlot, MachineError> {
        Ok(MachineSlot::encode(&self.spec, state)?)
    }

    fn gathering(&self, mut ready: Vec<SeatId>) -> Result<MachineSlot, MachineError> {
        ready.sort();
        ready.dedup();
        self.encode(&LobbyState::Gathering { ready })
    }
}

/// The seat and action of an input the lobby handles, if it is one.
fn recognised(input: &MachineInput) -> Option<(SeatId, LobbyAction)> {
    match input {
        MachineInput::Intent { seat, action } => {
            LobbyAction::of(action).map(|action| (*seat, action))
        }
        _ => None,
    }
}

fn decline(reason: &str) -> Result<StepResult, MachineError> {
    Ok(StepResult::declined(Decline::new(
        DeclineCode::NotViable,
        reason,
    )?))
}

fn carry_of(scene: Option<SceneId>, table: &TableState) -> Vec<CarryItem> {
    let seats = table.seats().iter().copied().map(CarryItem::Seat);
    scene
        .map(CarryItem::Scene)
        .into_iter()
        .chain(seats)
        .collect()
}

impl Machine for LobbyMachine {
    fn spec(&self) -> &MachineSpec {
        &self.spec
    }

    fn enter(&self, _table: &TableState, _entry: &Entry) -> Result<MachineSlot, MachineError> {
        self.gathering(Vec::new())
    }

    fn accepts(&self, table: &TableState, input: &MachineInput, state: &MachineSlot) -> bool {
        let gathering = matches!(self.state_of(state), Ok(LobbyState::Gathering { .. }));
        let addressed = recognised(input).is_some_and(|(seat, action)| {
            action == LobbyAction::Join || table.seats().contains(&seat)
        });
        gathering && addressed
    }

    fn step(&self, slot: &MachineSlot, input: &StepInput<'_>) -> Result<StepResult, MachineError> {
        let LobbyState::Gathering { mut ready } = self.state_of(slot)? else {
            return Ok(StepResult::declined(Decline::with_default_reason(
                DeclineCode::NotViable,
            )));
        };
        let Some((seat, action)) = recognised(&input.input) else {
            return Ok(StepResult::declined(Decline::with_default_reason(
                DeclineCode::NotViable,
            )));
        };
        let table = input.table;
        let is_seated = table.seats().contains(&seat);
        match (action, is_seated) {
            (LobbyAction::Join, true) => decline(ALREADY_SEATED),
            (LobbyAction::Join, false) if table.seats().len() >= table.seat_capacity() => {
                decline(TABLE_FULL)
            }
            (LobbyAction::Join, false) => Ok(StepResult::applied_in_place(vec![
                TableEffect::SeatJoined { seat },
            ])),
            (_, false) => decline(NOT_SEATED),
            (LobbyAction::Leave, true) => {
                let effects = vec![TableEffect::SeatLeft { seat }];
                if !ready.contains(&seat) {
                    return Ok(StepResult::applied_in_place(effects));
                }
                ready.retain(|other| *other != seat);
                Ok(StepResult::applied(self.gathering(ready)?, effects))
            }
            (LobbyAction::Ready, true) => {
                if ready.contains(&seat) {
                    return Ok(StepResult::applied_in_place(Vec::new()));
                }
                ready.push(seat);
                Ok(StepResult::applied(self.gathering(ready)?, Vec::new()))
            }
            (LobbyAction::Unready, true) => {
                if !ready.contains(&seat) {
                    return Ok(StepResult::applied_in_place(Vec::new()));
                }
                ready.retain(|other| *other != seat);
                Ok(StepResult::applied(self.gathering(ready)?, Vec::new()))
            }
            (LobbyAction::Start, true) => {
                if table.seats().iter().any(|seated| !ready.contains(seated)) {
                    let decline = Decline::new(DeclineCode::NotViable, NOBODY_READY)?
                        .with_missing_piece(ALL_READY)?;
                    return Ok(StepResult::declined(decline));
                }
                let next = self.encode(&LobbyState::Started {
                    scene: self.first_scene,
                })?;
                let effects = vec![TableEffect::SceneEntered {
                    scene: self.first_scene,
                }];
                Ok(StepResult::applied(next, effects))
            }
        }
    }

    fn wants_hand_off(&self, slot: &MachineSlot) -> Result<bool, MachineError> {
        Ok(self.state_name(slot)?.as_str() == STARTED)
    }

    fn exit(&self, table: &TableState, slot: &MachineSlot) -> Result<HandOff, MachineError> {
        let scene = match self.state_of(slot)? {
            LobbyState::Gathering { .. } => None,
            LobbyState::Started { scene } => Some(scene),
        };
        Ok(HandOff::new(
            self.next_machine.clone(),
            carry_of(scene, table),
        )?)
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
        let text = match self.state_of(slot)? {
            LobbyState::Started { .. } => {
                format!("The game has started with {} seats.", table.seats().len())
            }
            LobbyState::Gathering { ready } => {
                let ready_seated = table.seats().iter().filter(|seat| ready.contains(seat));
                let summary = format!(
                    "{} of {} seats are ready.",
                    ready_seated.count(),
                    table.seats().len()
                );
                match audience {
                    Audience::Seat(seat)
                        if ready.contains(&seat) && table.seats().contains(&seat) =>
                    {
                        format!("You are ready. {summary}")
                    }
                    Audience::Seat(seat) if table.seats().contains(&seat) => {
                        format!("You have a seat and are not ready. {summary}")
                    }
                    Audience::Seat(_) => format!("Join the table to take a seat. {summary}"),
                    Audience::Public => summary,
                }
            }
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
