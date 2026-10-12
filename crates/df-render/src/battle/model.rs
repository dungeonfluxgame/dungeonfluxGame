//! The stage model: what the battlefield is told to show. It mirrors the earlier build's stage
//! protocol (scene, tokens, highlights, camera commands, effects) as Rust types. Everything here
//! is staging. The server owns every outcome; a client builds these values from the server's
//! audience-safe combat view and never decides a hit, a miss or damage itself.

use super::{camera::ShotName, grid::Cell};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TokenId(pub String);

impl TokenId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Side {
    Hero,
    Enemy,
}

/// A condition tint shown on the figure. Cosmetic only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Poisoned,
    Burning,
    Frozen,
    Blessed,
    Stunned,
}

impl Status {
    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "poisoned" => Self::Poisoned,
            "burning" => Self::Burning,
            "frozen" => Self::Frozen,
            "blessed" => Self::Blessed,
            "stunned" => Self::Stunned,
            _ => return None,
        })
    }

    pub fn tint(self) -> [f32; 3] {
        match self {
            Self::Poisoned => [0.55, 1.0, 0.45],
            Self::Burning => [1.0, 0.62, 0.35],
            Self::Frozen => [0.6, 0.85, 1.0],
            Self::Blessed => [1.0, 0.92, 0.6],
            Self::Stunned => [0.85, 0.85, 0.6],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HitPoints {
    pub current: u32,
    pub maximum: u32,
}

impl HitPoints {
    pub fn fraction(self) -> f32 {
        if self.maximum == 0 {
            return 0.0;
        }
        (self.current as f32 / self.maximum as f32).clamp(0.0, 1.0)
    }
}

/// A combatant standing on a grid cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub id: TokenId,
    pub side: Side,
    pub name: String,
    pub cell: Cell,
    /// Standing height of the visible figure in metres.
    pub height_m: f32,
    /// Asset stem of the full-body cut-out; absent or failing to load falls back to `portrait`.
    pub figure: Option<String>,
    pub portrait: Option<String>,
    pub hit_points: Option<HitPoints>,
    pub statuses: Vec<Status>,
}

pub const MIN_HEIGHT_M: f32 = 0.6;
pub const MAX_HEIGHT_M: f32 = 3.0;

impl Token {
    pub fn clamped_height(&self) -> f32 {
        if self.height_m.is_finite() {
            self.height_m.clamp(MIN_HEIGHT_M, MAX_HEIGHT_M)
        } else {
            1.8
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HighlightKind {
    Move,
    Attack,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Highlight {
    pub kind: HighlightKind,
    pub cells: Vec<Cell>,
}

/// How a staged attack ended. Always copied from the server's outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttackResult {
    Miss,
    Hit {
        damage: u32,
        critical: bool,
        downed: bool,
    },
}

/// One server fact to stage. The director turns a beat into an ordered timeline.
#[derive(Clone, Debug, PartialEq)]
pub enum Beat {
    TurnStart {
        actor: TokenId,
    },
    /// The attacker steps beside the target under the turn-focus shot. Cues that arrive in two
    /// parts (roll, then impact) stage the first half with this and the second with `Strike`.
    Approach {
        attacker: TokenId,
        target: TokenId,
    },
    /// The lunge, contact and aftermath of a blow already approached (it approaches first if not).
    Strike {
        attacker: TokenId,
        target: TokenId,
        result: AttackResult,
        target_hit_points_after: Option<u32>,
    },
    /// `Approach` followed by `Strike` as one timeline.
    Attack {
        attacker: TokenId,
        target: TokenId,
        result: AttackResult,
        /// The target's hit points once the blow lands, as the server reports them.
        target_hit_points_after: Option<u32>,
    },
    Heal {
        source: TokenId,
        target: TokenId,
        amount: u32,
        target_hit_points_after: Option<u32>,
    },
}

/// What the HUD should show at the moment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudState {
    /// The roll panel the table reads while a blow resolves; clears itself after a few seconds.
    pub roll_card: Option<RollCard>,
    pub active: Option<TokenId>,
    pub round: u32,
    /// Turn order, first to act first. Empty means the order tokens were supplied in.
    pub initiative: Vec<TokenId>,
    pub finishing_blow: Option<FinishingBlow>,
}

/// What the d20 panel shows, copied from the server's roll description.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RollCard {
    pub value: String,
    pub label: String,
    pub explanation: String,
    pub verdict: String,
    /// `hit`, `miss`, `critical` or `graze`, for styling only.
    pub tone: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FinishingBlow {
    pub hero: String,
    pub enemy: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FloatTone {
    Damage,
    Critical,
    Miss,
    Heal,
}

/// A screen-space popup anchored above a token.
#[derive(Clone, Debug, PartialEq)]
pub struct FloatingText {
    /// Unique for the life of the stage, so a view can keep one element per popup.
    pub id: u64,
    pub token: TokenId,
    pub text: String,
    pub tone: FloatTone,
    /// Seconds since it appeared; it fades out by [`FLOAT_LIFETIME_S`].
    pub age_s: f32,
}

pub const FLOAT_LIFETIME_S: f32 = 1.4;

/// What a camera shot is centred on.
#[derive(Clone, Debug, PartialEq)]
pub enum CameraFocus {
    /// The shot's own authored target.
    Scene,
    Token(TokenId),
    Between(TokenId, TokenId),
}

/// A camera request, as in the earlier `CameraCommand`. `dolly` scales the shot's distance.
#[derive(Clone, Debug, PartialEq)]
pub struct CameraCommand {
    pub shot: ShotName,
    pub focus: CameraFocus,
    pub dolly: f32,
}
