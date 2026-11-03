//! The dialogue machine: one authored conversation with one urging (RES28, RES31, F.2, O.16 H3).
//!
//! The group machine of a talk, in its smallest honest form. It shows the opening line of one
//! authored speaker and offers one action, the urging (a persuasion, a bluff, a threat), which is
//! a rules action: the pipeline sends it to the rules owner (`influence::resolve` behind the host's
//! adapter, RES47), draws the dice at the one draw point and calls `step` with the decision. So
//! this machine proposes the urging and the rules owner settles it. The machine never rolls, never
//! reads the words of a line (the interpreter chose the action before the pipeline got here) and
//! never decides whether the speaker was moved: it reads the typed facts of the rules owner's
//! result (`succeeded`, or `complied` for a creature that needed no roll) and nothing else. A
//! result with neither fact (a scripted rules owner that reports none) ends as `heard`, which says
//! only that the speaker was told.
//!
//! When the roll is a player's own (`Decision::RollPending`), the machine reports `Waiting` and
//! holds only that pending id, as the explore machine does (RES26); the answer comes back as a
//! `Dice` input for the same pending, and a declined roll leaves the pending open for the pending
//! handler (C4) to cancel or ask again. A decline from the rules owner changes nothing. Once the
//! conversation has an end it is finished: it asks to hand the table to `lobby`, for as long as
//! the table has not moved on. The seats at the scene are read from the table on every call. It
//! owns three states (`listening`, `awaiting_roll`, `concluded`), reads the entry it is given (a
//! scene that is active, and seats at the table) and never imports another machine, draws a die,
//! reads a clock or calls a port.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::effects::TableState;
use crate::element::NameError;
use crate::identity::{SceneId, SeatId};
use crate::machine::{
    CarryItem, Decision, Entry, EntryRefusal, HandOff, InspectReport, Machine, MachineError,
    MachineId, MachineInput, MachineReadout, MachineSpec, Scope, SpecError, StateName, StepInput,
    StepOutcome, StepResult, SuspendPolicy,
};
use crate::machine_slot::MachineSlot;
use crate::outcome::{Decline, DeclineCode};
use crate::ports::{ActionId, Audience, PortName, Resolution, Resolved};

const DIALOGUE_SCHEMA_VERSION: u32 = 1;
const PHASE_TAG: &str = "phase";
const LISTENING: &str = "listening";
const AWAITING_ROLL: &str = "awaiting_roll";
const CONCLUDED: &str = "concluded";
const NEXT_MACHINE: &str = "lobby";
/// The typed facts that say the speaker gave way: a roll that met its difficulty class, and a
/// creature that was willing and needed no roll.
const SUCCEEDED_FACT: &str = "succeeded";
const COMPLIED_FACT: &str = "complied";

/// Why an authored conversation could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationError {
    /// A line the conversation shows is blank; the field says which.
    Blank(&'static str),
    InvalidAction(NameError),
}

impl fmt::Display for ConversationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blank(field) => write!(formatter, "the conversation's {field} is blank"),
            Self::InvalidAction(error) => write!(formatter, "the urging is not a name: {error}"),
        }
    }
}

impl std::error::Error for ConversationError {}

/// The authored content of the one conversation: who speaks, what they say first, the urging the
/// table may try, and what they say when it moves them and when it does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    speaker: String,
    opening: String,
    urge: ActionId,
    moved: String,
    unmoved: String,
}

impl Conversation {
    pub fn new(
        speaker: &str,
        opening: &str,
        urge: &str,
        moved: &str,
        unmoved: &str,
    ) -> Result<Self, ConversationError> {
        let line = |field: &'static str, text: &str| {
            if text.trim().is_empty() {
                Err(ConversationError::Blank(field))
            } else {
                Ok(text.trim().to_owned())
            }
        };
        Ok(Self {
            speaker: line("speaker", speaker)?,
            opening: line("opening line", opening)?,
            urge: ActionId::new(urge).map_err(ConversationError::InvalidAction)?,
            moved: line("line when moved", moved)?,
            unmoved: line("line when unmoved", unmoved)?,
        })
    }

    /// The urging the table may try.
    pub fn urge(&self) -> &ActionId {
        &self.urge
    }

    fn line_for(&self, outcome: Conclusion) -> String {
        let said = match outcome {
            Conclusion::Moved => &self.moved,
            Conclusion::Unmoved => &self.unmoved,
            Conclusion::Heard => return format!("{} has heard you out.", self.speaker),
        };
        format!("{}: \"{said}\"", self.speaker)
    }
}

/// How the conversation ended, as the typed facts of the rules owner's result said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Conclusion {
    Moved,
    Unmoved,
    Heard,
}

fn conclusion_of(resolved: &Resolved) -> Conclusion {
    let fact = |label: &str| {
        resolved
            .facts
            .iter()
            .find(|fact| fact.label.as_str() == label)
            .map(|fact| fact.amount)
    };
    match (fact(SUCCEEDED_FACT), fact(COMPLIED_FACT)) {
        (Some(0), _) | (None, Some(0)) => Conclusion::Unmoved,
        (Some(_), _) | (None, Some(_)) => Conclusion::Moved,
        (None, None) => Conclusion::Heard,
    }
}

/// The dialogue machine's closed states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum DialogueState {
    Listening { scene: SceneId },
    AwaitingRoll { scene: SceneId, pending: u64 },
    Concluded { scene: SceneId, how: Conclusion },
}

impl DialogueState {
    fn scene(&self) -> SceneId {
        match self {
            Self::Listening { scene }
            | Self::AwaitingRoll { scene, .. }
            | Self::Concluded { scene, .. } => *scene,
        }
    }
}

/// The dialogue machine over one authored conversation.
#[derive(Debug, Clone)]
pub struct DialogueMachine {
    spec: MachineSpec,
    next_machine: MachineId,
    conversation: Conversation,
}

impl DialogueMachine {
    pub fn new(conversation: Conversation) -> Result<Self, SpecError> {
        let spec = MachineSpec::new(
            "dialogue",
            Scope::Group,
            DIALOGUE_SCHEMA_VERSION,
            &[LISTENING, AWAITING_ROLL, CONCLUDED],
            SuspendPolicy::FinishAndHold,
        )?
        .handing_off_to(NEXT_MACHINE)?
        .requiring_port(PortName::RulesOwner)
        .with_rules_action(conversation.urge.clone())?
        .reading_entry();
        let next_machine = MachineId::new(NEXT_MACHINE).map_err(SpecError::InvalidName)?;
        Ok(Self {
            spec,
            next_machine,
            conversation,
        })
    }

    fn state_of(&self, slot: &MachineSlot) -> Result<DialogueState, MachineError> {
        Ok(slot.decode(&self.spec)?)
    }

    fn encode(&self, state: &DialogueState) -> Result<MachineSlot, MachineError> {
        Ok(MachineSlot::encode(&self.spec, state)?)
    }

    /// Whether the input is one the machine handles in this state, for the table as it is now.
    fn handles(&self, table: &TableState, state: &DialogueState, input: &MachineInput) -> bool {
        match (state, input) {
            (DialogueState::Listening { .. }, MachineInput::Intent { seat, action }) => {
                *action == self.conversation.urge && table.seats().contains(seat)
            }
            (
                DialogueState::AwaitingRoll { pending, .. },
                MachineInput::Dice {
                    pending: answered, ..
                },
            ) => answered.get() == *pending,
            (
                DialogueState::AwaitingRoll { pending, .. },
                MachineInput::PendingCancelled { pending: cancelled },
            ) => cancelled.get() == *pending,
            _ => false,
        }
    }

    fn conclude(&self, scene: SceneId, resolved: &Resolved) -> Result<StepResult, MachineError> {
        let how = conclusion_of(resolved);
        let next = self.encode(&DialogueState::Concluded { scene, how })?;
        Ok(StepResult::applied(next, Vec::new()))
    }
}

impl Machine for DialogueMachine {
    fn spec(&self) -> &MachineSpec {
        &self.spec
    }

    fn enter(&self, table: &TableState, entry: &Entry) -> Result<MachineSlot, MachineError> {
        let scene = entry.scene().ok_or(EntryRefusal::MissingScene)?;
        if table.active_scene() != Some(scene) {
            return Err(EntryRefusal::SceneNotActive { scene }.into());
        }
        let mut seats = entry.seats().peekable();
        if seats.peek().is_none() {
            return Err(EntryRefusal::NoSeats.into());
        }
        if let Some(seat) = seats.find(|seat| !table.seats().contains(seat)) {
            return Err(EntryRefusal::SeatNotAtTable { seat }.into());
        }
        self.encode(&DialogueState::Listening { scene })
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
        if let (DialogueState::AwaitingRoll { scene, .. }, MachineInput::PendingCancelled { .. }) =
            (&state, &input.input)
        {
            let back = self.encode(&DialogueState::Listening { scene: *scene })?;
            return Ok(StepResult::applied(back, Vec::new()));
        }
        match (&state, &input.decision) {
            (DialogueState::Listening { scene }, Some(Decision::RollPending(pending))) => {
                let next = self.encode(&DialogueState::AwaitingRoll {
                    scene: *scene,
                    pending: pending.get(),
                })?;
                let mut result = StepResult::applied(next, Vec::new());
                result.outcome = StepOutcome::Waiting(*pending);
                Ok(result)
            }
            (_, Some(Decision::Resolved(Resolution::Applied(resolved)))) => {
                self.conclude(state.scene(), resolved)
            }
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
        seat: SeatId,
    ) -> Vec<ActionId> {
        let input = MachineInput::Intent {
            seat,
            action: self.conversation.urge.clone(),
        };
        if self.accepts(table, &input, slot) {
            vec![self.conversation.urge.clone()]
        } else {
            Vec::new()
        }
    }

    fn wants_hand_off(&self, slot: &MachineSlot) -> Result<bool, MachineError> {
        Ok(self.state_name(slot)?.as_str() == CONCLUDED)
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
        let speaker = &self.conversation.speaker;
        let text = match (self.state_of(slot)?, audience) {
            (DialogueState::Listening { .. }, Audience::Seat(seat))
                if table.seats().contains(&seat) =>
            {
                format!(
                    "{speaker}: \"{}\" You can try to {}.",
                    self.conversation.opening,
                    self.conversation.urge.as_str().replace(['_', '-'], " ")
                )
            }
            (DialogueState::Listening { .. }, _) => {
                format!("{speaker}: \"{}\"", self.conversation.opening)
            }
            (DialogueState::AwaitingRoll { .. }, _) => {
                format!("{speaker} waits while a roll is made.")
            }
            (DialogueState::Concluded { how, .. }, _) => self.conversation.line_for(how),
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
