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
    /// Stays put and throws a melee weapon at the target.
    Thrown,
    /// A practical fire projectile, with warm spill at impact.
    Bolt,
    /// Three luminous darts; individual destinations arrive in `Beat::Missiles`.
    Missiles,
    /// A bead and spherical fire burst; area membership comes from the server.
    Burst,
}

impl AttackStyle {
    /// Selects the presentation style from the resolved mode and weapon id.
    pub fn for_weapon(ranged: bool, weapon: &str) -> Self {
        if !ranged {
            Self::Melee
        } else if matches!(
            weapon,
            "javelin" | "spear" | "dagger" | "handaxe" | "hand-axe" | "hand_axe"
        ) {
            Self::Thrown
        } else {
            Self::Ranged
        }
    }
}

/// What is in the air between attacker and target. A thrown weapon is the weapon
/// itself (selected by weapon id via [`AttackStyle::for_weapon`]); an arrow is a
/// bow shot. Presentation only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectileKind {
    Arrow,
    ThrownWeapon,
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

/// One committed strike or save, in presentation order. No roll or damage is calculated here.
#[derive(Clone, Debug, PartialEq)]
pub struct StrikeDisplay {
    pub target: TokenId,
    pub result: AttackResult,
    pub target_hit_points_after: Option<u32>,
    /// Attack roll or saving throw, including the server's half-damage explanation.
    pub card: Option<RollCard>,
    pub roll: Option<ResolvedRoll>,
}

/// Authoritative display fixture or server-projected dice; `None` d20 means automatic damage.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedRoll {
    pub d20: Option<u32>,
    pub damage_faces: Vec<u32>,
    /// Breakdown without the damage line, revealed only after the d20 face dwell.
    pub attack_card: Option<RollCard>,
}

/// One visible dice seat for technique timelines. Faces remain hidden while tumbling.
#[derive(Clone, Debug, PartialEq)]
pub struct PresentationDice {
    pub faces: Vec<u32>,
    pub damage: bool,
    pub settled: bool,
}

/// One server fact to stage. The director turns a beat into an ordered timeline.
#[derive(Clone, Debug, PartialEq)]
pub enum Beat {
    /// At most four resolved strikes by one actor. `action_surge_at` is the zero-based index
    /// before which to show the flourish (normally 2). This grants no actions or attacks.
    Flurry {
        attacker: TokenId,
        strikes: Vec<StrikeDisplay>,
        action_surge_at: Option<u32>,
    },
    /// Exactly three server-resolved darts, in fan order; targets may repeat.
    Missiles {
        caster: TokenId,
        darts: Vec<StrikeDisplay>,
    },
    /// All and only the server-supplied area targets react after the bead lands. Radius is
    /// cosmetic metres; neither geometry nor target membership is resolved by the client.
    AreaEffect {
        caster: TokenId,
        centre: Cell,
        radius_m: f32,
        targets: Vec<StrikeDisplay>,
    },
    /// A committed miss caused by Shield: the ward precedes the miss caption.
    Shield {
        attacker: TokenId,
        defender: TokenId,
        display: Option<Box<StrikeDisplay>>,
    },
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

/// The side seat for the die, "left" or "right": the one whose centre (21% or 79% across) is
/// farthest from the nearest fighter's screen x; `None` when no fighter is on screen.
pub fn dice_side(fighters_x: &[f32]) -> Option<&'static str> {
    let clearance = |seat: f32| {
        fighters_x
            .iter()
            .map(|x| (x - seat).abs())
            .fold(f32::INFINITY, f32::min)
    };
    if fighters_x.is_empty() {
        return None;
    }
    Some(if clearance(0.21) >= clearance(0.79) {
        "left"
    } else {
        "right"
    })
}

/// The picture every HUD surface shows for one figure: the health variant (the same
/// picture the HP plate uses) first, then the portrait picture. `None` only when the
/// figure has no picture at all, which is the only case that may fall back to an initial.
/// The variant is already a served URL; the portrait is resolved by the caller through
/// the art contract, so this stays free of any fetching rule.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn hud_picture(variant: Option<&str>, portrait: Option<String>) -> Option<String> {
    let variant = variant.filter(|text| {
        !text.is_empty()
            && text
                .bytes()
                .all(|byte| !byte.is_ascii_whitespace() && !b"\")';".contains(&byte))
    });
    variant.map(str::to_owned).or(portrait)
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
    pub presentation_dice: Option<PresentationDice>,
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

/// A combatant's hit-point change at one presentation sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HitPointChange {
    pub sequence: u64,
    pub before: u32,
    pub after: u32,
}

/// Keeps a display's presented cursor from moving backwards within one fight.
pub const fn advance_presented_sequence(current: u64, reported: u64) -> u64 {
    if reported > current {
        reported
    } else {
        current
    }
}

/// Keeps the view's active combatant from replacing the beat being presented.
pub const fn hold_server_turn(
    pending_cues: bool,
    latest_sequence: u64,
    presented_sequence: u64,
) -> bool {
    pending_cues || latest_sequence > presented_sequence
}

/// Projects hit points to the last change at or before the presented sequence.
pub fn hit_points_at_sequence(
    current: u32,
    presented_sequence: u64,
    changes: &[HitPointChange],
) -> u32 {
    changes
        .iter()
        .rev()
        .find(|change| change.sequence <= presented_sequence)
        .map(|change| change.after)
        .or_else(|| changes.first().map(|change| change.before))
        .unwrap_or(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_die_takes_the_side_seat_away_from_the_fighters() {
        assert_eq!(super::dice_side(&[]), None);
        // Fighters on the left half: the die goes right, and the other way round.
        assert_eq!(super::dice_side(&[0.18, 0.4]), Some("right"));
        assert_eq!(super::dice_side(&[0.62, 0.85]), Some("left"));
        // One on each side: the seat with more room wins.
        assert_eq!(super::dice_side(&[0.05, 0.6]), Some("right"));
    }

    #[test]
    fn ranged_attack_style_distinguishes_thrown_weapons_from_bows() {
        assert_eq!(
            AttackStyle::for_weapon(true, "javelin"),
            AttackStyle::Thrown
        );
        assert_eq!(AttackStyle::for_weapon(true, "spear"), AttackStyle::Thrown);
        assert_eq!(AttackStyle::for_weapon(true, "dagger"), AttackStyle::Thrown);
        assert_eq!(
            AttackStyle::for_weapon(true, "handaxe"),
            AttackStyle::Thrown
        );
        assert_eq!(
            AttackStyle::for_weapon(true, "hand-axe"),
            AttackStyle::Thrown
        );
        assert_eq!(
            AttackStyle::for_weapon(true, "longbow"),
            AttackStyle::Ranged
        );
        assert_eq!(
            AttackStyle::for_weapon(true, "light-crossbow"),
            AttackStyle::Ranged
        );
        assert_eq!(
            AttackStyle::for_weapon(false, "javelin"),
            AttackStyle::Melee
        );
    }

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
    fn battlefield_hit_points_follow_only_presented_changes_across_redraws() {
        let weasel = [HitPointChange {
            sequence: 1,
            before: 9,
            after: 7,
        }];
        let cursor = advance_presented_sequence(1, 0);
        assert_eq!(
            cursor, 1,
            "a view without a new beat cannot rewind the cursor"
        );
        assert_eq!(hit_points_at_sequence(9, cursor, &weasel), 7);

        let brakka = [
            HitPointChange {
                sequence: 4,
                before: 13,
                after: 4,
            },
            HitPointChange {
                sequence: 6,
                before: 4,
                after: 0,
            },
            HitPointChange {
                sequence: 9,
                before: 0,
                after: 0,
            },
        ];
        assert_eq!(hit_points_at_sequence(0, 4, &brakka), 4);
        assert_eq!(hit_points_at_sequence(0, 6, &brakka), 0);

        let heal = [HitPointChange {
            sequence: 6,
            before: 8,
            after: 11,
        }];
        assert_eq!(hit_points_at_sequence(11, 5, &heal), 8);
        assert_eq!(hit_points_at_sequence(11, 6, &heal), 11);
    }

    #[test]
    fn round_two_enemy_beats_keep_health_and_turn_during_redraw_between_impacts() {
        let brakka = [
            HitPointChange {
                sequence: 9,
                before: 13,
                after: 7,
            },
            HitPointChange {
                sequence: 10,
                before: 7,
                after: 1,
            },
        ];

        assert_eq!(hit_points_at_sequence(1, 9, &brakka), 7);
        assert!(hold_server_turn(false, 11, 9));
        assert_eq!(hit_points_at_sequence(1, 10, &brakka), 1);
        assert!(hold_server_turn(false, 11, 10));
        assert_eq!(hit_points_at_sequence(1, 11, &brakka), 1);
        assert!(!hold_server_turn(false, 11, 11));
    }

    #[test]
    fn hud_picture_prefers_the_health_variant_the_plate_uses() {
        // Jessa and Corin: a health variant exists, the portrait stem resolves to
        // nothing the chip can use, yet the chip must show the plate's picture.
        assert_eq!(
            hud_picture(
                Some("/gameplay/imagery/health-jessa.png"),
                Some("/assets/portrait.png".to_owned())
            ),
            Some("/gameplay/imagery/health-jessa.png".to_owned())
        );
        assert_eq!(
            hud_picture(Some("/gameplay/imagery/health-corin.png"), None),
            Some("/gameplay/imagery/health-corin.png".to_owned())
        );
        // No variant: the portrait picture stands. No picture at all: an initial may show.
        assert_eq!(
            hud_picture(None, Some("/assets/portrait.png".to_owned())),
            Some("/assets/portrait.png".to_owned())
        );
        assert_eq!(hud_picture(None, None), None);
        assert_eq!(hud_picture(Some(""), None), None);
        assert_eq!(hud_picture(Some("bad url).png"), None), None);
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
