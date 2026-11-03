//! The shared language of change: `TableEffect` and the records and refusals around it.
//!
//! It owns `TableEffect`, a versioned closed enum that machines and the engine use to change facts
//! the whole table shares (RES18, RES32, O.5), `TableEffectKind` (its payload-free list, which
//! carries the version each effect was introduced in), `EffectRecord` (an effect as saved, refused
//! when it is newer than this engine, H.12), and the `EffectRefusal` and `EffectError` that say
//! why a commit was refused. The applier and the state it changes are `table_state` (C6):
//! `TableState::apply` is all or nothing, one `EffectApplied` event per effect, and idempotent
//! per operation id (coding-style section 8, pattern 5). The names the machines import
//! (`TableState`, `TableStateError`, `ApplyOutcome`, `HandOffRecord`) are re-exported here.
//!
//! Adding an effect is a new variant of `TableEffect` and `TableEffectKind` and an arm in `kind`,
//! `introduced_in`, `key` and the applier's `apply_one`; each is an exhaustive `match`, so a
//! half-added effect does not compile. Effects are added and never changed (Q44). The effects of
//! the C6 table state (presence, moves, time, status, cast, the hand-off stack) joined at schema
//! version 1 because no record has been saved yet (D7 allows deleting and recreating until the
//! stabilisation gate). It never decides whether a machine should propose an effect, and it holds
//! no machine state: a machine's own state lives in its slot.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::element::{ElementId, name_type};
use crate::event::TableEvent;
use crate::identity::{Correlation, MachineId, SceneId, SeatId, TableId, TableScope};
use crate::listed::listed_enum;
use crate::machine_slot::{LoadFault, MachineSlot, RecordKind, SlotError};
use crate::table_state::{CastChange, CastEntry, MachineHome, Presence};

pub use crate::table_state::{ApplyOutcome, HandOffRecord, TableState, TableStateError};

name_type!(
    /// The id of an authored beat of a scene, such as `locked_gate`.
    BeatName
);

listed_enum! {
    /// How a conversation ended, as the rules owner's typed verdict said: the speaker gave way
    /// or did not. A result with no verdict is no ending, so there is no third value.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ConversationOutcome {
        Moved,
        Unmoved,
    }
}

impl ConversationOutcome {
    pub fn key(self) -> &'static str {
        match self {
            Self::Moved => "moved",
            Self::Unmoved => "unmoved",
        }
    }
}

/// The newest effect schema this engine writes and reads. Raised when an effect is added.
pub const TABLE_EFFECT_SCHEMA_VERSION: u32 = 1;

listed_enum! {
    /// Every effect without its payload: the list tests walk and events name.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum TableEffectKind {
        SeatJoined,
        SeatLeft,
        SceneEntered,
        BeatResolved,
        ConversationEnded,
        HandOffRecorded,
        SeatPresenceChanged,
        SeatMoved,
        TimeAdvanced,
        TablePaused,
        TableResumed,
        TableIdled,
        CastEntered,
        CastLeft,
        CastChanged,
        ActiveMachineSet,
        ActiveSlotUpdated,
        HandOffPushed,
        HandOffPopped,
    }
}

impl TableEffectKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::SeatJoined => "seat_joined",
            Self::SeatLeft => "seat_left",
            Self::SceneEntered => "scene_entered",
            Self::BeatResolved => "beat_resolved",
            Self::ConversationEnded => "conversation_ended",
            Self::HandOffRecorded => "hand_off_recorded",
            Self::SeatPresenceChanged => "seat_presence_changed",
            Self::SeatMoved => "seat_moved",
            Self::TimeAdvanced => "time_advanced",
            Self::TablePaused => "table_paused",
            Self::TableResumed => "table_resumed",
            Self::TableIdled => "table_idled",
            Self::CastEntered => "cast_entered",
            Self::CastLeft => "cast_left",
            Self::CastChanged => "cast_changed",
            Self::ActiveMachineSet => "active_machine_set",
            Self::ActiveSlotUpdated => "active_slot_updated",
            Self::HandOffPushed => "hand_off_pushed",
            Self::HandOffPopped => "hand_off_popped",
        }
    }

    /// The schema version that first had this effect. An effect never changes after that.
    pub fn introduced_in(self) -> u32 {
        match self {
            Self::SeatJoined
            | Self::SeatLeft
            | Self::SceneEntered
            | Self::BeatResolved
            | Self::ConversationEnded
            | Self::HandOffRecorded
            | Self::SeatPresenceChanged
            | Self::SeatMoved
            | Self::TimeAdvanced
            | Self::TablePaused
            | Self::TableResumed
            | Self::TableIdled
            | Self::CastEntered
            | Self::CastLeft
            | Self::CastChanged
            | Self::ActiveMachineSet
            | Self::ActiveSlotUpdated
            | Self::HandOffPushed
            | Self::HandOffPopped => 1,
        }
    }
}

/// A change to the facts the table shares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case", deny_unknown_fields)]
pub enum TableEffect {
    SeatJoined {
        seat: SeatId,
    },
    SeatLeft {
        seat: SeatId,
    },
    /// The whole party entered a scene, which becomes the table's focus scene with a new
    /// generation and an empty beat; every other scene is closed and its creatures that are not
    /// heroes leave with it.
    SceneEntered {
        scene: SceneId,
    },
    /// An open scene's authored beat was played through.
    BeatResolved {
        scene: SceneId,
        beat: BeatName,
    },
    /// The conversation of an open scene ended, and how: the one record of whether the speaker
    /// was moved, readable by a later scene, the narrator and replay.
    ConversationEnded {
        scene: SceneId,
        outcome: ConversationOutcome,
    },
    /// The engine moved the table from one machine to another, carrying `carried` items.
    HandOffRecorded {
        from: MachineId,
        to: MachineId,
        carried: usize,
    },
    /// A seat's player connected, fell asleep, dropped or stepped away (D.4).
    SeatPresenceChanged {
        seat: SeatId,
        presence: Presence,
    },
    /// One seat, with its heroes, went to a scene alone: a split, which the table declines while
    /// it keeps the party together (RES25).
    SeatMoved {
        seat: SeatId,
        scene: SceneId,
    },
    /// The world clock moved on. It never moves while the table is paused (D.3).
    TimeAdvanced {
        advance_s: u64,
    },
    TablePaused,
    /// A paused or idle table plays again.
    TableResumed,
    /// An active table had no input for `idle_minutes`.
    TableIdled,
    CastEntered {
        entry: CastEntry,
    },
    CastLeft {
        cast: ElementId,
    },
    CastChanged {
        cast: ElementId,
        change: CastChange,
    },
    /// `machine` with its saved state `slot` became the active machine of `home` (the table or one
    /// scene), replacing the one that ran; the hand-off stack is untouched. The one home of the
    /// active slot is the table state (RES27, O.3).
    ActiveMachineSet {
        home: MachineHome,
        machine: MachineId,
        slot: MachineSlot,
    },
    /// A step changed the saved state of the scene's active machine.
    ActiveSlotUpdated {
        home: MachineHome,
        slot: MachineSlot,
    },
    /// The active machine was suspended onto the scene's hand-off stack and `machine`, with its
    /// state `slot`, became active, in one step (RES27).
    HandOffPushed {
        scene: SceneId,
        machine: MachineId,
        slot: MachineSlot,
    },
    HandOffPopped {
        scene: SceneId,
    },
}

impl TableEffect {
    pub fn kind(&self) -> TableEffectKind {
        match self {
            Self::SeatJoined { .. } => TableEffectKind::SeatJoined,
            Self::SeatLeft { .. } => TableEffectKind::SeatLeft,
            Self::SceneEntered { .. } => TableEffectKind::SceneEntered,
            Self::BeatResolved { .. } => TableEffectKind::BeatResolved,
            Self::ConversationEnded { .. } => TableEffectKind::ConversationEnded,
            Self::HandOffRecorded { .. } => TableEffectKind::HandOffRecorded,
            Self::SeatPresenceChanged { .. } => TableEffectKind::SeatPresenceChanged,
            Self::SeatMoved { .. } => TableEffectKind::SeatMoved,
            Self::TimeAdvanced { .. } => TableEffectKind::TimeAdvanced,
            Self::TablePaused => TableEffectKind::TablePaused,
            Self::TableResumed => TableEffectKind::TableResumed,
            Self::TableIdled => TableEffectKind::TableIdled,
            Self::CastEntered { .. } => TableEffectKind::CastEntered,
            Self::CastLeft { .. } => TableEffectKind::CastLeft,
            Self::CastChanged { .. } => TableEffectKind::CastChanged,
            Self::ActiveMachineSet { .. } => TableEffectKind::ActiveMachineSet,
            Self::ActiveSlotUpdated { .. } => TableEffectKind::ActiveSlotUpdated,
            Self::HandOffPushed { .. } => TableEffectKind::HandOffPushed,
            Self::HandOffPopped { .. } => TableEffectKind::HandOffPopped,
        }
    }
}

/// An effect as saved: stamped with the schema version it was written under (H.12). The effect
/// stays raw data until the version has been checked, so a record from a newer engine, with an
/// effect this one does not know, is refused as a version problem and not as a parse failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectRecord {
    schema_version: u32,
    effect: Value,
}

/// A saved effect this engine cannot use, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordRefused {
    pub found: u32,
    pub expected: u32,
    pub fault: LoadFault,
}

impl RecordRefused {
    /// The `load.refused` event of refusing this record when the host loads `table` (H.12).
    pub fn event(&self, table: TableId) -> TableEvent {
        TableEvent::LoadRefused {
            table,
            record: RecordKind::EffectIntent,
            machine: None,
            version: u64::from(self.found),
            expected: u64::from(self.expected),
            cause: self.fault,
        }
    }
}

impl fmt::Display for RecordRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "an effect record at schema version {} was refused ({}); this engine reads up to {}",
            self.found,
            self.fault.key(),
            self.expected
        )
    }
}

impl std::error::Error for RecordRefused {}

impl EffectRecord {
    /// The effect stamped with the current schema version.
    pub fn stamp(effect: &TableEffect) -> Result<Self, SlotError> {
        let effect = serde_json::to_value(effect).map_err(|error| SlotError::NotEncodable {
            class: error.classify().into(),
        })?;
        Ok(Self {
            schema_version: TABLE_EFFECT_SCHEMA_VERSION,
            effect,
        })
    }

    /// A record from its parts, as storage hands it back.
    pub fn from_parts(schema_version: u32, effect: Value) -> Self {
        Self {
            schema_version,
            effect,
        }
    }

    /// The effect, unless the record is newer than this engine, has no valid version, claims a
    /// version older than the one that introduced its effect, or does not hold an effect.
    pub fn open(self) -> Result<TableEffect, RecordRefused> {
        let refused = |fault| RecordRefused {
            found: self.schema_version,
            expected: TABLE_EFFECT_SCHEMA_VERSION,
            fault,
        };
        if self.schema_version == 0 || self.schema_version > TABLE_EFFECT_SCHEMA_VERSION {
            return Err(refused(LoadFault::UnsupportedVersion));
        }
        let effect =
            TableEffect::deserialize(&self.effect).map_err(|_| refused(LoadFault::Malformed))?;
        if self.schema_version < effect.kind().introduced_in() {
            return Err(refused(LoadFault::UnsupportedVersion));
        }
        Ok(effect)
    }
}

listed_enum! {
    /// Why an effect was not applied.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum EffectRefusal {
        SeatAlreadySeated,
        SeatNotSeated,
        TableFull,
        /// The effect names a scene that is not an open scene of the table.
        SceneNotActive,
        /// A seat went to another scene than the rest of the party while the table keeps the
        /// party together (RES25).
        SplitPartyDeclined,
        /// The world clock was asked to move while the table is paused.
        TablePaused,
        AlreadyPaused,
        /// Only an active table goes idle.
        TableNotActive,
        /// Only a paused or idle table is resumed.
        TableNotPausedOrIdle,
        WorldTimeOverflow,
        GenerationOverflow,
        CastEntryExists,
        CastEntryUnknown,
        TooManyCastEntries,
        TooManyConditions,
        /// A cast entry is controlled by a seat that is not at the table.
        ControllerNotSeated,
        /// A hero is controlled by a seat, never by the engine.
        PartyNeedsSeatController,
        /// A creature a seat controls was put in a scene other than the one its seat is in.
        ControlledOutsideSeatScene,
        /// A seat already plays a hero; the old one leaves before a replacement enters.
        SeatHasHero,
        /// A hand-off or a state change needs a machine to be active in the scene.
        NoActiveMachine,
        /// A machine was handed to while it is already active or waiting on the stack.
        MachineAlreadyOnStack,
        HitPointsAboveMaximum,
        HandOffStackFull,
        HandOffStackEmpty,
        /// A hand-off frame names a machine other than the one its slot belongs to.
        FrameSlotMismatch,
    }
}

impl EffectRefusal {
    pub fn key(self) -> &'static str {
        match self {
            Self::SeatAlreadySeated => "seat_already_seated",
            Self::SeatNotSeated => "seat_not_seated",
            Self::TableFull => "table_full",
            Self::SceneNotActive => "scene_not_active",
            Self::SplitPartyDeclined => "split_party_declined",
            Self::TablePaused => "table_paused",
            Self::AlreadyPaused => "already_paused",
            Self::TableNotActive => "table_not_active",
            Self::TableNotPausedOrIdle => "table_not_paused_or_idle",
            Self::WorldTimeOverflow => "world_time_overflow",
            Self::GenerationOverflow => "generation_overflow",
            Self::CastEntryExists => "cast_entry_exists",
            Self::CastEntryUnknown => "cast_entry_unknown",
            Self::TooManyCastEntries => "too_many_cast_entries",
            Self::TooManyConditions => "too_many_conditions",
            Self::ControllerNotSeated => "controller_not_seated",
            Self::PartyNeedsSeatController => "party_needs_seat_controller",
            Self::ControlledOutsideSeatScene => "controlled_outside_seat_scene",
            Self::SeatHasHero => "seat_has_hero",
            Self::NoActiveMachine => "no_active_machine",
            Self::MachineAlreadyOnStack => "machine_already_on_stack",
            Self::HitPointsAboveMaximum => "hit_points_above_maximum",
            Self::HandOffStackFull => "hand_off_stack_full",
            Self::HandOffStackEmpty => "hand_off_stack_empty",
            Self::FrameSlotMismatch => "frame_slot_mismatch",
        }
    }
}

/// An effect of a commit was refused, so none of the commit was applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectError {
    pub effect: TableEffectKind,
    pub cause: EffectRefusal,
}

impl EffectError {
    /// The class A event that records the refusal, once, where the commit handles it.
    pub fn event(&self, scope: &TableScope, correlation: Correlation) -> TableEvent {
        TableEvent::EffectRefused {
            scope: scope.clone(),
            correlation,
            effect: self.effect,
            cause: self.cause,
        }
    }
}

impl fmt::Display for EffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the effect {} was refused: {}",
            self.effect.key(),
            self.cause.key()
        )
    }
}

impl std::error::Error for EffectError {}
