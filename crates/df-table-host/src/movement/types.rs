//! The frozen shared contract of a scene transition. The stages in the sibling modules read and
//! write only these types; changing one is a decision of the coordinator, never of a stage.
//!
//! Characters are named by persona id (the grammar of a SAM layout label), as everywhere else in
//! the host. Everything here crosses a route or a record, so it is serializable.

use serde::{Deserialize, Serialize};

/// The closed list of ways a party moves, as Jev's label list names them. `key` is the word a
/// content file, a record and the command line use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveKind {
    Walk,
    Enter,
    Run,
    Climb,
    Descend,
    Sneak,
}

impl MoveKind {
    pub const ALL: [Self; 6] = [
        Self::Walk,
        Self::Enter,
        Self::Run,
        Self::Climb,
        Self::Descend,
        Self::Sneak,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Walk => "walk",
            Self::Enter => "enter",
            Self::Run => "run",
            Self::Climb => "climb",
            Self::Descend => "descend",
            Self::Sneak => "sneak",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

/// A character who can move: a persona id and the name the line may use for them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyMember {
    pub id: String,
    pub display_name: String,
}

/// What a line of movement means: who goes, how, and where, with the line as it was said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveIntent {
    pub kind: MoveKind,
    /// The persona ids that move; those not listed stay where they are. Never empty: a line that
    /// names nobody moves the whole party, listed here one by one.
    pub who: Vec<String>,
    /// The destination in the words of the line ("the town square", "the hidden door").
    pub to: String,
    pub line: String,
}

/// Jev's reading of one line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reading", rename_all = "snake_case")]
pub enum Interpreted {
    Move { intent: MoveIntent },
    /// Not a movement: a question, a social line or another action.
    NotAMove,
    NeedsClarification,
}

/// How far back from the viewer a character stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Depth {
    Foreground,
    Middle,
    Background,
}

/// Which part of the picture a character stands in, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Centre,
    Right,
}

/// Where a character stands and what they are doing there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub character: String,
    pub depth: Depth,
    pub side: Side,
    /// What the character is doing, in a few words ("leans on the rail"); never dialogue.
    pub doing: String,
}

/// The narrative engine's design of the scene a move arrives in. The place and mood are words for
/// the picture; `placements` has one entry for each character who is in the scene, and
/// `left_behind` names the characters who stayed in the one before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneDesign {
    pub place: String,
    pub mood: String,
    pub elements: Vec<String>,
    pub placements: Vec<Placement>,
    pub left_behind: Vec<String>,
}

/// What keeps one character looking the same in every plate: their look in words, and a library
/// picture of them when one exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceCard {
    pub character: String,
    pub look: String,
    pub portrait: Option<String>,
}

/// What the image service is asked to paint: the prompt, and the cards of the characters in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlateBrief {
    pub prompt: String,
    pub references: Vec<ReferenceCard>,
}

/// The steps of a transition, in order. `Live` is the end: the new plate is on the scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionStage {
    Interpreting,
    Designing,
    Briefing,
    Generating,
    Locating,
    Placing,
    Live,
}

impl TransitionStage {
    pub const ALL: [Self; 7] = [
        Self::Interpreting,
        Self::Designing,
        Self::Briefing,
        Self::Generating,
        Self::Locating,
        Self::Placing,
        Self::Live,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Interpreting => "interpreting",
            Self::Designing => "designing",
            Self::Briefing => "briefing",
            Self::Generating => "generating",
            Self::Locating => "locating",
            Self::Placing => "placing",
            Self::Live => "live",
        }
    }

    /// The stage after this one, or `None` after `Live`.
    pub fn next(self) -> Option<Self> {
        Self::ALL
            .iter()
            .position(|stage| *stage == self)
            .and_then(|at| Self::ALL.get(at + 1))
            .copied()
    }
}

/// Why a stage did not finish: the stage, the stable key of its typed error, and its words. The
/// old scene stays up when a transition fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionFailure {
    pub stage: TransitionStage,
    pub key: String,
    pub message: String,
}

/// One record of a transition, for the audit line and for replaying a report: what stage it was
/// when, in milliseconds on the transition's own clock, and what happened in a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionRecord {
    pub transition: u64,
    pub stage: TransitionStage,
    pub at_ms: u64,
    pub detail: String,
}
