//! The lobby: the table machine before the first scene (RES28, D.4, question 90).
//!
//! Seats join and leave, say they are ready, and one of them starts the game. It owns two states
//! (`gathering` and `started`), the five lobby actions, and the lobby's own record of who is ready.
//! Which seats sit at the table is a shared fact, so a join and a leave leave as `SeatJoined` and
//! `SeatLeft` effects, and starting leaves as `SceneEntered` for the first scene; the lobby never
//! reads or writes another machine's state. It models `df_session::lobby::decide_start` by
//! reading it, without depending on `df-session`: a start needs at least one seat and every seat
//! ready. The character, display and funding blockers of that function belong to the creation
//! machine and the host, not to this machine. It asks for a hand-off to `explore` when the game
//! starts and carries the roster and the first scene.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SeatEntry {
    seat: SeatId,
    ready: bool,
}

/// The lobby's closed states. Seats stay sorted by id so equal tables give equal records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum LobbyState {
    Gathering {
        capacity: usize,
        seats: Vec<SeatEntry>,
    },
    Started {
        scene: SceneId,
        roster: Vec<SeatId>,
    },
}

impl LobbyState {
    fn name(&self) -> &'static str {
        match self {
            Self::Gathering { .. } => GATHERING,
            Self::Started { .. } => STARTED,
        }
    }
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
        let spec = MachineSpec::new(
            "lobby",
            Scope::Table,
            LOBBY_SCHEMA_VERSION,
            &[GATHERING, STARTED],
            SuspendPolicy::FinishAndHold,
        )?
        .handing_off_to(NEXT_MACHINE)?;
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

fn refusal(code: DeclineCode, reason: &str) -> Result<Decline, MachineError> {
    Ok(Decline::new(code, reason)?)
}

impl Machine for LobbyMachine {
    fn spec(&self) -> &MachineSpec {
        &self.spec
    }

    fn enter(&self, table: &TableState, _entry: &Entry) -> Result<MachineSlot, MachineError> {
        let seats = table
            .seats()
            .iter()
            .map(|seat| SeatEntry {
                seat: *seat,
                ready: false,
            })
            .collect();
        self.encode(&LobbyState::Gathering {
            capacity: table.seat_capacity(),
            seats,
        })
    }

    fn accepts(&self, input: &MachineInput, state: &MachineSlot) -> bool {
        let gathering = matches!(self.state_of(state), Ok(LobbyState::Gathering { .. }));
        gathering && recognised(input).is_some()
    }

    fn step(&self, slot: &MachineSlot, input: &StepInput) -> Result<StepResult, MachineError> {
        let not_viable = || {
            Ok(StepResult::declined(
                slot,
                Decline::with_default_reason(DeclineCode::NotViable),
            ))
        };
        let LobbyState::Gathering {
            capacity,
            mut seats,
        } = self.state_of(slot)?
        else {
            return not_viable();
        };
        let Some((seat, action)) = recognised(&input.input) else {
            return not_viable();
        };
        let is_seated = seats.iter().any(|entry| entry.seat == seat);
        match (action, is_seated) {
            (LobbyAction::Join, true) => decline(slot, ALREADY_SEATED),
            (LobbyAction::Join, false) if seats.len() >= capacity => decline(slot, TABLE_FULL),
            (LobbyAction::Join, false) => {
                seats.push(SeatEntry { seat, ready: false });
                seats.sort_by_key(|entry| entry.seat);
                let next = self.encode(&LobbyState::Gathering { capacity, seats })?;
                Ok(StepResult::applied(
                    next,
                    vec![TableEffect::SeatJoined { seat }],
                ))
            }
            (_, false) => decline(slot, NOT_SEATED),
            (LobbyAction::Leave, true) => {
                seats.retain(|entry| entry.seat != seat);
                let next = self.encode(&LobbyState::Gathering { capacity, seats })?;
                Ok(StepResult::applied(
                    next,
                    vec![TableEffect::SeatLeft { seat }],
                ))
            }
            (LobbyAction::Ready | LobbyAction::Unready, true) => {
                for entry in seats.iter_mut().filter(|entry| entry.seat == seat) {
                    entry.ready = action == LobbyAction::Ready;
                }
                let next = self.encode(&LobbyState::Gathering { capacity, seats })?;
                Ok(StepResult::applied(next, Vec::new()))
            }
            (LobbyAction::Start, true) => {
                if seats.iter().any(|entry| !entry.ready) {
                    let decline = refusal(DeclineCode::NotViable, NOBODY_READY)?
                        .with_missing_piece(ALL_READY)?;
                    return Ok(StepResult::declined(slot, decline));
                }
                let roster = seats.iter().map(|entry| entry.seat).collect();
                let next = self.encode(&LobbyState::Started {
                    scene: self.first_scene,
                    roster,
                })?;
                let effects = vec![TableEffect::SceneEntered {
                    scene: self.first_scene,
                }];
                Ok(StepResult::applied(next, effects).asking_for_hand_off())
            }
        }
    }

    fn exit(&self, slot: &MachineSlot) -> Result<HandOff, MachineError> {
        let (scene, roster): (SceneId, Vec<SeatId>) = match self.state_of(slot)? {
            LobbyState::Gathering { seats, .. } => (
                self.first_scene,
                seats.into_iter().map(|entry| entry.seat).collect(),
            ),
            LobbyState::Started { scene, roster } => (scene, roster),
        };
        let mut carry = vec![CarryItem::Scene(scene)];
        carry.extend(roster.into_iter().map(CarryItem::Seat));
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
        let text = match self.state_of(slot)? {
            LobbyState::Started { roster, .. } => {
                format!("The game has started with {} seats.", roster.len())
            }
            LobbyState::Gathering { seats, .. } => {
                let ready = seats.iter().filter(|entry| entry.ready).count();
                let summary = format!("{ready} of {} seats are ready.", seats.len());
                match audience {
                    Audience::Public => summary,
                    Audience::Seat(seat) => match seats.iter().find(|entry| entry.seat == seat) {
                        Some(entry) if entry.ready => format!("You are ready. {summary}"),
                        Some(_) => format!("You have a seat and are not ready. {summary}"),
                        None => format!("Join the table to take a seat. {summary}"),
                    },
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

fn decline(slot: &MachineSlot, reason: &str) -> Result<StepResult, MachineError> {
    Ok(StepResult::declined(
        slot,
        refusal(DeclineCode::NotViable, reason)?,
    ))
}
