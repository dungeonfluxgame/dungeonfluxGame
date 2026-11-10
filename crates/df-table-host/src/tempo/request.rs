//! What crosses `/dev/tempo/..`: the bodies, the replies and `TempoError`.
//!
//! Every route answers a `TempoReply<T>`: `{"outcome":"done","value":..}` or
//! `{"outcome":"rejected","error":{"error":"<key>",..},"message":".."}`. The load route reads the
//! table file itself (`table::MusicTable`) as its body.

use std::fmt;

use axum::http::StatusCode;
use df_tempo::presentation::{PresentationError, TempoPhase};
use serde::{Deserialize, Serialize};

use super::limits::{MAXIMUM_NOTE_CHARACTERS, MAXIMUM_PERMILLE};
use crate::dev_http::{BodyError, DevReply, RouteError};
use crate::music::{MusicError, MusicSource};

/// The answer of a tempo route: its value, or why it was refused.
pub type TempoReply<T> = DevReply<T, TempoError>;

/// `POST /dev/tempo/impulse`: one thing that happened in the journey, by the kind the table names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImpulseRequest {
    pub kind: String,
    /// How strongly the kind's impulse acts, in permille (0 to 1000); the kind's full strength
    /// when left out.
    #[serde(default)]
    pub intensity: Option<u16>,
    /// What happened, for a person reading `tempo state`; never logged.
    #[serde(default)]
    pub note: Option<String>,
}

impl ImpulseRequest {
    /// That the strength and the note are ones the route takes.
    pub fn check(&self) -> Result<(), TempoError> {
        if let Some(intensity) = self.intensity
            && intensity > MAXIMUM_PERMILLE
        {
            return Err(TempoError::InvalidIntensity { intensity });
        }
        if let Some(note) = &self.note {
            let note_characters = note.chars().count();
            if note_characters > MAXIMUM_NOTE_CHARACTERS || note.chars().any(char::is_control) {
                return Err(TempoError::InvalidNote { note_characters });
            }
        }
        Ok(())
    }
}

/// The phase of the journey's tempo, as the engine classifies it (`df_tempo::presentation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseKey {
    Calm,
    Build,
    Anticipation,
    Peak,
    Release,
}

impl PhaseKey {
    pub const ALL: [Self; 5] = [
        Self::Calm,
        Self::Build,
        Self::Anticipation,
        Self::Peak,
        Self::Release,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Calm => "calm",
            Self::Build => "build",
            Self::Anticipation => "anticipation",
            Self::Peak => "peak",
            Self::Release => "release",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|phase| phase.key() == key)
    }

    pub(super) fn of(phase: TempoPhase) -> Self {
        match phase {
            TempoPhase::Calm => Self::Calm,
            TempoPhase::Build => Self::Build,
            TempoPhase::Anticipation => Self::Anticipation,
            TempoPhase::Peak => Self::Peak,
            TempoPhase::Release => Self::Release,
        }
    }
}

/// Whether the journey's tempo is climbing, falling or level, from the engine's inertia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Momentum {
    Rising,
    Falling,
    Stable,
}

impl Momentum {
    pub const ALL: [Self; 3] = [Self::Rising, Self::Falling, Self::Stable];

    pub fn key(self) -> &'static str {
        match self {
            Self::Rising => "rising",
            Self::Falling => "falling",
            Self::Stable => "stable",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|momentum| momentum.key() == key)
    }
}

/// What the director did at a boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAction {
    /// The playing cue stays.
    Keep,
    /// A track change was ordered.
    Switch,
    /// A change is wanted and waits (the minimum dwell).
    Hold,
    /// A change is wanted and its track is not ready; the playing cue stays.
    Skip,
}

impl DecisionAction {
    pub const ALL: [Self; 4] = [Self::Keep, Self::Switch, Self::Hold, Self::Skip];

    pub fn key(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Switch => "switch",
            Self::Hold => "hold",
            Self::Skip => "skip",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.key() == key)
    }
}

/// Why the director decided as it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionReason {
    /// The tempo wants the cue that already plays.
    Wanted,
    /// The playing cue still holds within its margin and no cue before it in the table applies.
    Margin,
    /// Nothing plays yet.
    Start,
    /// The tempo wants another cue and it may change now.
    Changed,
    /// The playing cue has not played its minimum dwell.
    Dwell,
    /// The wanted cue's track is not in the library (`tempo prepare` makes it).
    NotPrepared,
}

impl DecisionReason {
    pub const ALL: [Self; 6] = [
        Self::Wanted,
        Self::Margin,
        Self::Start,
        Self::Changed,
        Self::Dwell,
        Self::NotPrepared,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Wanted => "wanted",
            Self::Margin => "margin",
            Self::Start => "start",
            Self::Changed => "changed",
            Self::Dwell => "dwell",
            Self::NotPrepared => "not_prepared",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.key() == key)
    }
}

/// An impulse the run remembers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecentView {
    pub kind: String,
    pub intensity: u16,
    pub at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The cue that plays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayingView {
    pub profile: String,
    pub track: String,
    /// Where the track came from when `tempo prepare` last made it in this run of the host:
    /// composed then, or already in the library.
    pub source: MusicSource,
    pub since_ms: u64,
    pub volume_percent: u8,
}

/// The newest decision of the director.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionView {
    pub action: DecisionAction,
    pub reason: DecisionReason,
    /// The cue the tempo wanted at that boundary.
    pub wanted: String,
    /// When a held change may happen, in the run's milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_ms: Option<u64>,
}

/// `GET /dev/tempo` and the answer of every route that moves the run: where the journey's tempo
/// stands and what the director plays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TempoView {
    /// The table's name for the policy: `tempo/<version>`.
    pub policy: String,
    pub policy_version: u32,
    /// The run's clock, in milliseconds since the table was loaded or the run was reset.
    pub now_ms: u64,
    pub phase: PhaseKey,
    pub intensity: u16,
    pub inertia: i16,
    pub momentum: Momentum,
    pub target: u16,
    pub fatigue: u16,
    pub recent: Vec<RecentView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playing: Option<PlayingView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<DecisionView>,
}

/// `POST /dev/tempo/table`: what the table holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableLoaded {
    pub version: u32,
    pub base: String,
    pub profiles: Vec<String>,
    pub impulses: Vec<String>,
}

/// How one profile's track stands after `tempo prepare`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PrepareStatus {
    /// The library already held the track; nothing was composed.
    Reused {
        bytes: u64,
    },
    Composed {
        bytes: u64,
        compose_ms: u64,
    },
    Failed {
        error: MusicError,
    },
}

/// One profile of the table in a prepare report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prepared {
    pub profile: String,
    pub track: String,
    pub length_seconds: u32,
    #[serde(flatten)]
    pub status: PrepareStatus,
}

/// `POST /dev/tempo/prepare`: every profile's track, reused or composed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareReport {
    pub profiles: Vec<Prepared>,
    pub reused: u32,
    pub composed: u32,
    pub failed: u32,
    /// The seconds of music the compositions of this call asked for.
    pub composed_seconds: u32,
}

/// Why a table was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum TableCause {
    NoProfiles,
    NoImpulses,
    TooMany,
    /// A policy the engine refuses (`df_tempo::presentation::PresentationPolicy::validate`).
    InvalidPolicy,
    InvalidDirector,
    InvalidId {
        id: String,
    },
    DuplicateProfile {
        id: String,
    },
    DuplicateImpulse {
        kind: String,
    },
    UnknownBase {
        id: String,
    },
    InvalidImpulse {
        kind: String,
    },
    InvalidPrompt {
        id: String,
        prompt_characters: usize,
    },
    InvalidLength {
        id: String,
        length_seconds: u32,
    },
    InvalidVolume {
        id: String,
        volume_percent: u8,
    },
    InvalidCrossfade {
        id: String,
        crossfade_ms: u32,
    },
    InvalidCondition {
        id: String,
        condition: ConditionCause,
    },
}

impl TableCause {
    pub fn key(&self) -> &'static str {
        match self {
            Self::NoProfiles => "no_profiles",
            Self::NoImpulses => "no_impulses",
            Self::TooMany => "too_many",
            Self::InvalidPolicy => "invalid_policy",
            Self::InvalidDirector => "invalid_director",
            Self::InvalidId { .. } => "invalid_id",
            Self::DuplicateProfile { .. } => "duplicate_profile",
            Self::DuplicateImpulse { .. } => "duplicate_impulse",
            Self::UnknownBase { .. } => "unknown_base",
            Self::InvalidImpulse { .. } => "invalid_impulse",
            Self::InvalidPrompt { .. } => "invalid_prompt",
            Self::InvalidLength { .. } => "invalid_length",
            Self::InvalidVolume { .. } => "invalid_volume",
            Self::InvalidCrossfade { .. } => "invalid_crossfade",
            Self::InvalidCondition { .. } => "invalid_condition",
        }
    }
}

/// What is wrong with a profile's `when`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionCause {
    /// `phases` is listed and empty.
    EmptyPhases,
    /// A bound over 1000, or a minimum above the maximum.
    InvalidIntensityRange,
    /// `recent` names no kind, or a window of no time or more than a day.
    InvalidRecent,
    /// `recent` names a kind the table has no impulse for.
    UnknownImpulseKind,
}

impl ConditionCause {
    pub const ALL: [Self; 4] = [
        Self::EmptyPhases,
        Self::InvalidIntensityRange,
        Self::InvalidRecent,
        Self::UnknownImpulseKind,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::EmptyPhases => "empty_phases",
            Self::InvalidIntensityRange => "invalid_intensity_range",
            Self::InvalidRecent => "invalid_recent",
            Self::UnknownImpulseKind => "unknown_impulse_kind",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

/// Why the engine refused a step. A run the host drives with its own clock reaches none of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineCause {
    InvalidPolicy,
    InvalidState,
    InvalidInput,
    BackwardsTime,
}

impl EngineCause {
    pub const ALL: [Self; 4] = [
        Self::InvalidPolicy,
        Self::InvalidState,
        Self::InvalidInput,
        Self::BackwardsTime,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::InvalidPolicy => "invalid_policy",
            Self::InvalidState => "invalid_state",
            Self::InvalidInput => "invalid_input",
            Self::BackwardsTime => "backwards_time",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

impl From<PresentationError> for EngineCause {
    fn from(error: PresentationError) -> Self {
        match error {
            PresentationError::InvalidPolicy => Self::InvalidPolicy,
            PresentationError::InvalidState => Self::InvalidState,
            PresentationError::InvalidInput => Self::InvalidInput,
            PresentationError::BackwardsTime => Self::BackwardsTime,
        }
    }
}

/// Why a tempo call changed nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum TempoError {
    InvalidBody(BodyError),
    /// No table is loaded: `tempo load <file>` first.
    NoTable,
    /// The table has no impulse of this kind.
    UnknownImpulse {
        kind: String,
    },
    /// A strength over 1000 permille.
    InvalidIntensity {
        intensity: u16,
    },
    /// A note too long or with a control character.
    InvalidNote {
        note_characters: usize,
    },
    InvalidTable {
        cause: TableCause,
    },
    Engine {
        cause: EngineCause,
    },
    /// The task that made the tracks stopped without an answer.
    Stopped,
}

impl TempoError {
    /// The error's closed word, as its serde tag writes it.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::NoTable => "no_table",
            Self::UnknownImpulse { .. } => "unknown_impulse",
            Self::InvalidIntensity { .. } => "invalid_intensity",
            Self::InvalidNote { .. } => "invalid_note",
            Self::InvalidTable { .. } => "invalid_table",
            Self::Engine { .. } => "engine",
            Self::Stopped => "stopped",
        }
    }
}

impl fmt::Display for TempoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::NoTable => formatter.write_str("no music table is loaded: run tempo load <file>"),
            Self::UnknownImpulse { kind } => {
                write!(formatter, "the table has no impulse of kind '{kind}'")
            }
            Self::InvalidIntensity { intensity } => write!(
                formatter,
                "an impulse's intensity is 0 to {MAXIMUM_PERMILLE} permille, not {intensity}"
            ),
            Self::InvalidNote { note_characters } => write!(
                formatter,
                "a note has at most {MAXIMUM_NOTE_CHARACTERS} characters and no control \
                 character, not {note_characters}"
            ),
            Self::InvalidTable { cause } => {
                write!(formatter, "the music table is refused ({})", cause.key())
            }
            Self::Engine { cause } => write!(
                formatter,
                "the tempo engine refused the step ({})",
                cause.key()
            ),
            Self::Stopped => formatter.write_str("the tempo task stopped without an answer"),
        }
    }
}

impl std::error::Error for TempoError {}

impl RouteError for TempoError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_)
            | Self::UnknownImpulse { .. }
            | Self::InvalidIntensity { .. }
            | Self::InvalidNote { .. }
            | Self::InvalidTable { .. } => StatusCode::BAD_REQUEST,
            Self::NoTable => StatusCode::CONFLICT,
            Self::Engine { .. } | Self::Stopped => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
