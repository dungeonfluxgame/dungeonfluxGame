//! The frozen shared contract of a scene transition. The stages in the sibling modules read and
//! write only these types; changing one is a decision of the coordinator, never of a stage.
//!
//! Characters are named by persona id (the grammar of a SAM layout label), as everywhere else in
//! the host. Everything here crosses a route or a record, so it is serializable.

use df_content::world_bible::Thing;
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

/// How the caller says the speaker meant a line, apart from what the line says. `Move` is for a
/// line the speaker said toward going somewhere (`move act`); every other line is `Unknown`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Declared {
    #[default]
    Unknown,
    Move,
}

impl Declared {
    pub fn key(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Move => "move",
        }
    }
}

/// What a line that asks for clarification leaves unclear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unclear {
    /// Where to: the line names nothing to go to or toward.
    Destination,
    /// Anything else that stops the line being one movement.
    Other,
}

impl Unclear {
    pub const ALL: [Self; 2] = [Self::Destination, Self::Other];

    pub fn key(self) -> &'static str {
        match self {
            Self::Destination => "destination",
            Self::Other => "other",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|unclear| unclear.key() == key)
    }
}

/// Why Jev asked for clarification: what is unclear, and its reason in its own words (data for a
/// second reading and for the report, never read by code).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clarification {
    pub unclear: Unclear,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Jev's reading of one line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reading", rename_all = "snake_case")]
pub enum Interpreted {
    Move {
        intent: MoveIntent,
    },
    /// Not a movement: a question, a social line or another action.
    NotAMove,
    NeedsClarification(Clarification),
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

/// A character who belongs to the place a move arrives at and is not in the party (a fruit stand
/// vendor at a market). `id` has the grammar of a persona id and a SAM label: lowercase letters,
/// digits and `_`, starting with a letter. `look` is the concrete visible traits that let the same
/// person be recognised in later pictures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resident {
    pub id: String,
    pub display_name: String,
    pub job: String,
    pub look: String,
}

/// The narrative engine's design of the scene a move arrives in. The place and mood are words for
/// the picture; `things` are what is in sight there worth a close look, each with its look, which
/// the place offers once it is the active one; `placements` has one entry for each character who
/// is in the scene, the party members who moved and the `residents` alike, and `left_behind` names
/// the party members who stayed in the scene before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneDesign {
    pub place: String,
    pub mood: String,
    /// The one feature that sets the place apart from the places already designed, as a short
    /// lowercase id; kept with the place once the party is there.
    pub feature: String,
    pub elements: Vec<String>,
    pub things: Vec<Thing>,
    /// What the movers hold that the picture shows on or beside them, most noticeable first: the
    /// reference sheets a plate attaches when its room for pictures is short go in this order.
    pub shown_items: Vec<ShownItem>,
    pub residents: Vec<Resident>,
    pub placements: Vec<Placement>,
    pub left_behind: Vec<String>,
    /// The sights passed on the way to the place, in order, when the move asked for a walk.
    #[serde(default)]
    pub stages: Vec<RouteSight>,
    /// The look of the town the place is in, shared by the place and every sight on the way to it.
    #[serde(default)]
    pub town_look: Option<String>,
}

/// How a mover goes through a sight on the way: on the move, or stopped to take it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pose {
    Walking,
    Pausing,
}

impl Pose {
    pub const ALL: [Self; 2] = [Self::Walking, Self::Pausing];

    pub fn key(self) -> &'static str {
        match self {
            Self::Walking => "walking",
            Self::Pausing => "pausing",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|pose| pose.key() == key)
    }
}

/// One sight the party passes on the way to the place a move arrives at: an id no other sight of
/// the move has, what a camera would see of it, and how the movers go through it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteSight {
    pub id: String,
    pub sight: String,
    pub pose: Pose,
}

/// What a move is asked to do with the town it is in. The role of a kind of move is data of the
/// caller (`@walk` lines of a beats file).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TownRole {
    /// The move's place belongs to the town in force; when there is none, the design gives the
    /// town's look and it is kept.
    Seeds,
    /// The move's place belongs to the town in force, if there is one.
    Within,
    /// The move leaves the town in force, which ends.
    Leaves,
}

impl TownRole {
    pub const ALL: [Self; 3] = [Self::Seeds, Self::Within, Self::Leaves];

    pub fn key(self) -> &'static str {
        match self {
            Self::Seeds => "seeds",
            Self::Within => "within",
            Self::Leaves => "leaves",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.key() == key)
    }
}

/// What a move asks of its design beyond the place: up to `stages` sights on the way, the move's
/// role in the town, and the move's own id (the plate's name), which a town it seeds is stamped
/// with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WalkAsk {
    pub stages: usize,
    pub town: TownRole,
    pub move_id: String,
}

/// An item the picture shows: whose it is and its id among what they hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShownItem {
    pub owner: String,
    pub item: String,
}

/// What keeps one character looking the same in every plate: their look in words, and a library
/// picture of them when one exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceCard {
    pub character: String,
    pub look: String,
    pub portrait: Option<String>,
}

/// What the image service is asked to paint: the prompt, what the picture must not show, and the
/// cards of the characters in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlateBrief {
    pub prompt: String,
    pub avoid: Vec<String>,
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
