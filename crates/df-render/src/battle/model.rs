//! The stage model: what the battlefield is told to show. It mirrors the earlier build's stage
//! protocol (scene, tokens, highlights, camera commands, effects) as Rust types. Everything here
//! is staging. The server owns every outcome; a client builds these values from the server's
//! audience-safe combat view and never decides a hit, a miss or damage itself.

use super::{camera::ShotName, grid::Cell, status::Status};

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

/// How a combatant delivers an attack on screen. Staging only: the server decides the outcome.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AttackStyle {
    /// Steps up beside the target and strikes.
    #[default]
    Melee,
    /// Stays put, draws and looses an arrow that flies to the target.
    Ranged,
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
    pub attack: AttackStyle,
    pub voice: Option<super::sound::VoiceSet>,
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
    /// A strike that ends the encounter, as reported by the server.
    FightEndingStrike {
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
    /// An attack that ends the encounter, as reported by the server.
    FightEndingAttack {
        attacker: TokenId,
        target: TokenId,
        result: AttackResult,
        target_hit_points_after: Option<u32>,
    },
    /// The server moved a figure to a cell: it walks the path of cells there (a straight eased
    /// glide when no path is open), with the camera holding its start and end in frame.
    Move {
        actor: TokenId,
        from: Cell,
        to: Cell,
    },
    Heal {
        source: TokenId,
        target: TokenId,
        amount: u32,
        target_hit_points_after: Option<u32>,
    },
    /// The start of a fight: everyone's d20 tumbles and lands, then the camera visits each
    /// combatant in turn order. `rolls` are the server's recorded rolls, first to act first.
    Initiative {
        rolls: Vec<InitiativeRoll>,
    },
}

/// One recorded initiative roll. Copied from the server; nothing is rolled here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitiativeRoll {
    pub token: TokenId,
    pub die: u32,
    pub modifier: i32,
    pub total: i32,
}

/// 1 -> "1st", 2 -> "2nd", 11 -> "11th", 22 -> "22nd".
pub fn ordinal(number: u32) -> String {
    let suffix = match (number % 100, number % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    format!("{number}{suffix}")
}

/// The phone's header line for a player's own initiative roll, for example
/// `Initiative 14 + 1 = 15 \u{2014} you act 2nd`.
pub fn initiative_line(die: u32, modifier: i32, total: i32, order: u32) -> String {
    let sign = if modifier >= 0 { '+' } else { '\u{2212}' };
    format!(
        "Initiative {die} {sign} {} = {total} \u{2014} you act {}",
        modifier.unsigned_abs(),
        ordinal(order)
    )
}

/// A die above a figure during the initiative roll. It tumbles for `tumble_s` seconds after it
/// appears and then shows its total.
#[derive(Clone, Debug, PartialEq)]
pub struct DieView {
    pub token: TokenId,
    pub face: u32,
    pub modifier: i32,
    pub total: i32,
    pub age_s: f32,
    pub tumble_s: f32,
    /// Set once the die has landed on its number.
    pub landed: bool,
}

/// The big order badge that slams in beside a figure at its roll-call stop.
#[derive(Clone, Debug, PartialEq)]
pub struct BadgeView {
    pub token: TokenId,
    pub order: u32,
    pub name: String,
    pub total: i32,
    pub age_s: f32,
}

/// Everything the HUD shows for the initiative moment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InitiativeShow {
    pub banner: Option<String>,
    pub dice: Vec<DieView>,
    pub badges: Vec<BadgeView>,
    /// While set, the round tracker shows only these tokens, in this order: it fills in as the
    /// roll-call visits each combatant.
    pub tracker: Option<Vec<TokenId>>,
}

/// What the HUD should show at the moment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudState {
    /// The roll panel the table reads while a blow resolves; clears itself after a few seconds.
    pub roll_card: Option<RollCard>,
    pub active: Option<TokenId>,
    /// A question the table is waiting on (a reaction), shown in place of the turn label.
    pub prompt: Option<String>,
    pub round: u32,
    /// Turn order, first to act first. Empty means the order tokens were supplied in.
    pub initiative: Vec<TokenId>,
    pub finishing_blow: Option<FinishingBlow>,
    pub roll_for_initiative: InitiativeShow,
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
    /// Seconds since it appeared; see [`super::float_text`] for how long it lives.
    pub age_s: f32,
    /// How long it stays fully shown after it has risen, in seconds.
    pub hold_s: f32,
}

/// What a camera shot is centred on.
#[derive(Clone, Debug, PartialEq)]
pub enum CameraFocus {
    /// The shot's own authored target.
    Scene,
    Token(TokenId),
    /// The resting shot, solved from everyone on the field and the cells that matter this turn
    /// (see `rest_shot`): wide, raised, holding both sides clear of the HUD cards.
    Wide,
    /// The resting shot leaned toward one figure (a short ease at the start of its turn).
    Lean(TokenId),
    Between(TokenId, TokenId),
    /// The shot for an action: the actor alone (a self action) or the actor and its target, both
    /// kept fully in frame for the whole beat, from the side of the line the camera already holds,
    /// clear of every other figure. `stand` is where the actor will stand to act, if not where it
    /// stands now. The stage solves it from where everyone actually is (see `framing`).
    Action {
        actor: TokenId,
        target: Option<TokenId>,
        stand: Option<Cell>,
    },
}

/// A camera request, as in the earlier `CameraCommand`. `dolly` scales the shot's distance.
#[derive(Clone, Debug, PartialEq)]
pub struct CameraCommand {
    pub shot: ShotName,
    pub focus: CameraFocus,
    pub dolly: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinals_follow_english_including_the_teens() {
        let words: Vec<String> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 101, 111]
            .map(ordinal)
            .into();
        assert_eq!(
            words,
            [
                "1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "23rd",
                "101st", "111th"
            ]
        );
    }

    #[test]
    fn the_phone_line_reads_like_the_example() {
        assert_eq!(
            initiative_line(14, 1, 15, 2),
            "Initiative 14 + 1 = 15 \u{2014} you act 2nd"
        );
        assert_eq!(
            initiative_line(9, -1, 8, 3),
            "Initiative 9 \u{2212} 1 = 8 \u{2014} you act 3rd"
        );
    }
}
