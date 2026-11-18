//! The frozen shared contract of a scene transition. The stages in the sibling modules read and
//! write only these types; changing one is a decision of the coordinator, never of a stage.
//!
//! Characters are named by persona id (the grammar of a SAM layout label), as everywhere else in
//! the host. Everything here crosses a route or a record, so it is serializable.

use df_content::world_bible::Thing;

use crate::world::InventoryItem;
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
        /// The place designed in play the line goes back to, by id (one of the places the party
        /// has been to, checked in code), when it goes back to one.
        #[serde(default)]
        back_to: Option<String>,
    },
    /// Not a movement: a question, a social line or another action.
    NotAMove,
    NeedsClarification(Clarification),
}

/// How far something in sight is from the one who sees it, nearest first (`planning/explore.md`,
/// "Movement toward what is on screen"): a place design gives one for each thing and resident, and
/// an approach brings its target closer one step at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Distance {
    WithinReach,
    AcrossTheSquare,
    DownTheRoad,
    OnTheHorizon,
}

impl Distance {
    pub const ALL: [Self; 4] = [
        Self::WithinReach,
        Self::AcrossTheSquare,
        Self::DownTheRoad,
        Self::OnTheHorizon,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::WithinReach => "within_reach",
            Self::AcrossTheSquare => "across_the_square",
            Self::DownTheRoad => "down_the_road",
            Self::OnTheHorizon => "on_the_horizon",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|distance| distance.key() == key)
    }
}

/// A way to walk in the view when no thing or person on the plate names where to: closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Ahead,
    Left,
    Right,
    Behind,
}

impl Direction {
    pub const ALL: [Self; 4] = [Self::Ahead, Self::Left, Self::Right, Self::Behind];

    pub fn key(self) -> &'static str {
        match self {
            Self::Ahead => "ahead",
            Self::Left => "left",
            Self::Right => "right",
            Self::Behind => "behind",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|direction| direction.key() == key)
    }
}

/// How the movers go on a move (the intent judge's reading of the line, a closed set): walking,
/// running, which covers more ground a step, or sneaking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pace {
    #[default]
    Walk,
    Run,
    Sneak,
}

impl Pace {
    pub const ALL: [Self; 3] = [Self::Walk, Self::Run, Self::Sneak];

    pub fn key(self) -> &'static str {
        match self {
            Self::Walk => "walk",
            Self::Run => "run",
            Self::Sneak => "sneak",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|pace| pace.key() == key)
    }
}

/// How far one thing or resident of a design is, by its id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetDistance {
    pub id: String,
    pub distance: Distance,
}

/// How far `id` is among `distances`, when they give it.
pub fn distance_of(distances: &[TargetDistance], id: &str) -> Option<Distance> {
    distances
        .iter()
        .find(|known| known.id == id)
        .map(|known| known.distance)
}

/// One step of an approach along a line of sight, as the design is asked for it: the target (a
/// thing or resident id of the place before, or a direction's key, which the step's design names a
/// thing by), what it is in words, how far it was before the step and the step's number from 1;
/// step 0 is a turn to face it from where the movers stand (a look that way), no step at all. The
/// pace is how the movers go: a run covers one band more a step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproachAsk {
    pub target: String,
    pub toward: String,
    pub distance: Distance,
    pub step: u32,
    #[serde(default)]
    pub pace: Pace,
    /// The movers back away from the target, keeping it in view: it grows farther.
    #[serde(default)]
    pub away: bool,
    /// How the hero stands in the step's picture (crouched low, sitting on the wall), in words
    /// with no names: kept from an act until a move or an act changes it.
    #[serde(default)]
    pub pose: Option<String>,
}

/// How far back from the viewer a character stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Depth {
    Foreground,
    Middle,
    Background,
}

impl Depth {
    pub const ALL: [Self; 3] = [Self::Foreground, Self::Middle, Self::Background];

    pub fn key(self) -> &'static str {
        match self {
            Self::Foreground => "foreground",
            Self::Middle => "middle",
            Self::Background => "background",
        }
    }
}

/// Which part of the picture a character stands in, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Centre,
    Right,
}

impl Side {
    pub const ALL: [Self; 3] = [Self::Left, Self::Centre, Self::Right];

    pub fn key(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Centre => "centre",
            Self::Right => "right",
        }
    }
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
    /// What a trader has to sell, each with its count; empty for one who trades nothing.
    #[serde(default)]
    pub wares: Vec<InventoryItem>,
    /// The coin a trader holds to pay and give change with.
    #[serde(default)]
    pub coin: Option<InventoryItem>,
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
    /// How far each thing and resident is, as the design gives it (one each at most).
    #[serde(default)]
    pub distances: Vec<TargetDistance>,
}

/// How a mover goes through a sight on the way: on the move, or stopped to take it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gait {
    Walking,
    Pausing,
}

impl Gait {
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
    pub pose: Gait,
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
