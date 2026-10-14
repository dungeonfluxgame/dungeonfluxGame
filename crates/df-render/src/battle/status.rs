//! Status effects shown on a figure. The set is closed: the fifteen standard 2024 conditions plus
//! the common combat states. Each has a stable string id (the same id names its icon art,
//! `status-<id>`), so a server can later carry them as plain strings.
//!
//! Everything here is presentation. A status is copied from the server's view (or derived from
//! hit points and the existing knockout flag by the adapter); the renderer never applies a
//! condition by itself. [`resolve`] holds the combination rules the figure shader follows, and
//! [`pose_for`] the body language (slump, rigidity, wobble) the stage animates.

/// One condition or state. The order is the bit order sent to the shader and the icon row order.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Status {
    Blinded,
    Charmed,
    Deafened,
    Exhaustion,
    Frightened,
    Grappled,
    Incapacitated,
    Invisible,
    Paralyzed,
    Petrified,
    Poisoned,
    Prone,
    Restrained,
    Stunned,
    Unconscious,
    Bloodied,
    Burning,
    Blessed,
    Cursed,
    Concentrating,
    Dodging,
    Hidden,
    Inspired,
    Raging,
    Marked,
    Dying,
    Cover,
    Stable,
}

pub const STATUS_COUNT: usize = 28;

impl Status {
    pub const ALL: [Self; STATUS_COUNT] = [
        Self::Blinded,
        Self::Charmed,
        Self::Deafened,
        Self::Exhaustion,
        Self::Frightened,
        Self::Grappled,
        Self::Incapacitated,
        Self::Invisible,
        Self::Paralyzed,
        Self::Petrified,
        Self::Poisoned,
        Self::Prone,
        Self::Restrained,
        Self::Stunned,
        Self::Unconscious,
        Self::Bloodied,
        Self::Burning,
        Self::Blessed,
        Self::Cursed,
        Self::Concentrating,
        Self::Dodging,
        Self::Hidden,
        Self::Inspired,
        Self::Raging,
        Self::Marked,
        Self::Dying,
        Self::Cover,
        Self::Stable,
    ];

    /// The stable lowercase id, also the icon stem suffix.
    pub fn id(self) -> &'static str {
        match self {
            Self::Blinded => "blinded",
            Self::Charmed => "charmed",
            Self::Deafened => "deafened",
            Self::Exhaustion => "exhaustion",
            Self::Frightened => "frightened",
            Self::Grappled => "grappled",
            Self::Incapacitated => "incapacitated",
            Self::Invisible => "invisible",
            Self::Paralyzed => "paralyzed",
            Self::Petrified => "petrified",
            Self::Poisoned => "poisoned",
            Self::Prone => "prone",
            Self::Restrained => "restrained",
            Self::Stunned => "stunned",
            Self::Unconscious => "unconscious",
            Self::Bloodied => "bloodied",
            Self::Burning => "burning",
            Self::Blessed => "blessed",
            Self::Cursed => "cursed",
            Self::Concentrating => "concentrating",
            Self::Dodging => "dodging",
            Self::Hidden => "hidden",
            Self::Inspired => "inspired",
            Self::Raging => "raging",
            Self::Marked => "marked",
            Self::Dying => "dying",
            Self::Cover => "cover",
            Self::Stable => "stable",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.id() == id)
    }

    /// The icon art stem under `assets/ui/generated/`.
    pub fn icon_stem(self) -> String {
        format!("status-{}", self.id())
    }

    /// Two letters for the fallback chip drawn while the icon art is missing.
    pub fn initials(self) -> &'static str {
        match self {
            Self::Blinded => "Bl",
            Self::Charmed => "Ch",
            Self::Deafened => "Df",
            Self::Exhaustion => "Ex",
            Self::Frightened => "Fr",
            Self::Grappled => "Gr",
            Self::Incapacitated => "In",
            Self::Invisible => "Iv",
            Self::Paralyzed => "Pa",
            Self::Petrified => "Pe",
            Self::Poisoned => "Po",
            Self::Prone => "Pr",
            Self::Restrained => "Rs",
            Self::Stunned => "St",
            Self::Unconscious => "Un",
            Self::Bloodied => "Bd",
            Self::Burning => "Bu",
            Self::Blessed => "Bs",
            Self::Cursed => "Cu",
            Self::Concentrating => "Co",
            Self::Dodging => "Do",
            Self::Hidden => "Hi",
            Self::Inspired => "Is",
            Self::Raging => "Ra",
            Self::Marked => "Ma",
            Self::Dying => "Dy",
            Self::Cover => "Cv",
            Self::Stable => "Sy",
        }
    }

    /// The word shown beside the icon, for tooltips and the accessible name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Blinded => "Blinded",
            Self::Charmed => "Charmed",
            Self::Deafened => "Deafened",
            Self::Exhaustion => "Exhaustion",
            Self::Frightened => "Frightened",
            Self::Grappled => "Grappled",
            Self::Incapacitated => "Incapacitated",
            Self::Invisible => "Invisible",
            Self::Paralyzed => "Paralyzed",
            Self::Petrified => "Petrified",
            Self::Poisoned => "Poisoned",
            Self::Prone => "Prone",
            Self::Restrained => "Restrained",
            Self::Stunned => "Stunned",
            Self::Unconscious => "Unconscious",
            Self::Bloodied => "Bloodied",
            Self::Burning => "Burning",
            Self::Blessed => "Blessed",
            Self::Cursed => "Cursed",
            Self::Concentrating => "Concentrating",
            Self::Dodging => "Dodging",
            Self::Hidden => "Hidden",
            Self::Inspired => "Inspired",
            Self::Raging => "Raging",
            Self::Marked => "Marked",
            Self::Dying => "Dying",
            Self::Cover => "In cover",
            Self::Stable => "Stable",
        }
    }

    /// Fallback chip colour (CSS hex), grouped by feel: hostile effects warm or sickly, boons
    /// golden, mental effects violet, physical states steel.
    pub fn chip_colour(self) -> &'static str {
        match self {
            Self::Poisoned => "#5c9a3c",
            Self::Burning | Self::Raging | Self::Bloodied | Self::Dying => "#b8402e",
            Self::Blessed | Self::Inspired => "#c9a23a",
            Self::Charmed => "#c9607f",
            Self::Cursed | Self::Frightened | Self::Marked => "#7a52a8",
            Self::Paralyzed | Self::Stunned | Self::Concentrating => "#4f86b8",
            Self::Petrified | Self::Restrained | Self::Grappled | Self::Cover => "#7a7e86",
            _ => "#5d6470",
        }
    }

    fn bit(self) -> u32 {
        1 << (self as u32)
    }
}

/// A set of statuses as a bit set; iteration follows [`Status::ALL`].
#[derive(Clone, Copy, Default, Eq, Hash, PartialEq)]
pub struct StatusSet(u32);

impl std::fmt::Debug for StatusSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.iter().map(Status::id)).finish()
    }
}

/// Bits per shader mask float: 14 bits stay exact in an `f32` and in a `u32` conversion.
const MASK_SPLIT: u32 = 14;

impl StatusSet {
    pub const EMPTY: Self = Self(0);

    pub fn of(statuses: &[Status]) -> Self {
        statuses
            .iter()
            .fold(Self::EMPTY, |set, status| set.with(*status))
    }

    pub fn with(self, status: Status) -> Self {
        Self(self.0 | status.bit())
    }

    pub fn without(self, status: Status) -> Self {
        Self(self.0 & !status.bit())
    }

    pub fn contains(self, status: Status) -> bool {
        self.0 & status.bit() != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    pub fn iter(self) -> impl Iterator<Item = Status> {
        Status::ALL
            .into_iter()
            .filter(move |status| self.contains(*status))
    }

    fn only(self, allowed: &[Status]) -> Self {
        Self(self.0 & Self::of(allowed).0)
    }

    /// The low and high halves as exact floats for the shader's per-instance data.
    pub fn shader_words(self) -> [f32; 2] {
        let low = self.0 & ((1 << MASK_SPLIT) - 1);
        let high = self.0 >> MASK_SPLIT;
        [low as f32, high as f32]
    }
}

/// Statuses that recolour or restyle the figure's body.
const BODY: [Status; 15] = [
    Status::Poisoned,
    Status::Petrified,
    Status::Frightened,
    Status::Charmed,
    Status::Cursed,
    Status::Paralyzed,
    Status::Raging,
    Status::Burning,
    Status::Hidden,
    Status::Unconscious,
    Status::Incapacitated,
    Status::Exhaustion,
    Status::Bloodied,
    Status::Blinded,
    Status::Dying,
];
/// Statuses drawn along the silhouette's edge.
const EDGE: [Status; 5] = [
    Status::Burning,
    Status::Paralyzed,
    Status::Charmed,
    Status::Raging,
    Status::Hidden,
];
/// Statuses drawn as shapes and particles around the figure.
const OVERLAY: [Status; 12] = [
    Status::Poisoned,
    Status::Burning,
    Status::Stunned,
    Status::Charmed,
    Status::Blessed,
    Status::Inspired,
    Status::Cursed,
    Status::Restrained,
    Status::Grappled,
    Status::Deafened,
    Status::Concentrating,
    Status::Dying,
];

/// Which treatments the figure shader applies after the combination rules.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Resolved {
    pub body: StatusSet,
    pub edge: StatusSet,
    pub overlay: StatusSet,
    pub invisible: bool,
}

/// The combination rules:
/// - petrified overrides every other colour treatment of the body (a statue is grey stone),
/// - invisible drops the body treatments but keeps the edge treatments of the other effects, so
///   a burning invisible figure still shows its rim,
/// - everything else stacks (prone and poisoned both show: prone is a pose, poisoned a colour).
pub fn resolve(set: StatusSet) -> Resolved {
    let invisible = set.contains(Status::Invisible);
    let mut body = set.only(&BODY);
    if set.contains(Status::Petrified) {
        body = StatusSet::of(&[Status::Petrified]);
    }
    if invisible {
        body = StatusSet::EMPTY;
    }
    let mut edge = set.only(&EDGE);
    if set.contains(Status::Petrified) {
        // Stone does not crackle, glow or flush: only a visible rim from fire survives.
        edge = edge.only(&[Status::Burning]);
    }
    if invisible {
        edge = edge.with(Status::Invisible);
    }
    Resolved {
        body,
        edge,
        overlay: set.only(&OVERLAY),
        invisible,
    }
}

/// Body language derived from the statuses, before any animation clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusPose {
    /// 0 standing, 1 slumped to the ground (shared by downed, prone and unconscious).
    pub slump: f32,
    /// Share of the idle sway left (0 for stone and rigid paralysis).
    pub idle: f32,
    /// Breathing gain (raging breathes heavier).
    pub breath: f32,
    pub wobble: f32,
    pub shiver: f32,
    /// How far the figure leans away from the nearest opponent, in radians.
    pub flinch: f32,
    pub dim: f32,
    pub alpha: f32,
    pub heartbeat: bool,
    pub afterimage: bool,
}

/// The pose for `set`.
pub fn pose_for(set: StatusSet) -> StatusPose {
    let has = |status| set.contains(status);
    let slump = [
        (Status::Prone, 1.0),
        (Status::Unconscious, 1.0),
        (Status::Dying, 1.0),
        (Status::Incapacitated, 0.6),
        (Status::Exhaustion, 0.3),
    ]
    .into_iter()
    .filter(|(status, _)| has(*status))
    .map(|(_, amount)| amount)
    .fold(0.0, f32::max);
    let rigid = has(Status::Petrified) || has(Status::Paralyzed);
    let unconscious = has(Status::Unconscious) || has(Status::Dying) || has(Status::Stable);
    let constrained = has(Status::Restrained) || has(Status::Grappled);
    let idle = if rigid || unconscious {
        0.0
    } else if constrained {
        0.3
    } else if has(Status::Exhaustion) || has(Status::Incapacitated) {
        0.55
    } else {
        1.0
    };
    StatusPose {
        slump,
        idle,
        breath: if has(Status::Raging) { 2.4 } else { 1.0 },
        wobble: if has(Status::Stunned) { 1.0 } else { 0.0 },
        shiver: if has(Status::Frightened) { 1.0 } else { 0.0 },
        flinch: if has(Status::Frightened) { 0.16 } else { 0.0 },
        dim: if unconscious || has(Status::Incapacitated) {
            0.45
        } else if has(Status::Exhaustion) {
            0.15
        } else {
            0.0
        },
        alpha: if has(Status::Invisible) { 0.3 } else { 1.0 },
        heartbeat: has(Status::Dying) && !has(Status::Stable),
        afterimage: has(Status::Dodging),
    }
}

/// The statuses the adapter derives from what the server's combat view already says, and
/// nothing else: bloodied at or below half hit points (above zero), unconscious when the server
/// reports the combatant knocked out. Conditions such as prone, poisoned or stunned are never
/// guessed here; they wait for the server to carry them.
pub fn derived_from_view(hit_points: u32, maximum: u32, knocked_out: bool) -> Vec<Status> {
    let mut derived = Vec::new();
    if maximum > 0 && hit_points > 0 && hit_points * 2 <= maximum {
        derived.push(Status::Bloodied);
    }
    if knocked_out {
        derived.push(Status::Unconscious);
    }
    derived
}

/// Statuses worth an icon: everything except the ones that read better as a tile marker.
pub fn icon_statuses(set: StatusSet) -> Vec<Status> {
    if set.contains(Status::Stable) {
        return vec![Status::Stable];
    }
    if set.contains(Status::Unconscious) {
        return vec![Status::Unconscious];
    }
    set.iter()
        .filter(|status| *status != Status::Cover)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_set_is_closed_and_round_trips_by_id() {
        assert_eq!(Status::ALL.len(), STATUS_COUNT);
        let mut seen = std::collections::BTreeSet::new();
        for status in Status::ALL {
            assert_eq!(Status::from_id(status.id()), Some(status));
            assert!(seen.insert(status.id()), "duplicate id {}", status.id());
            assert!(
                status.id().bytes().all(|byte| byte.is_ascii_lowercase()),
                "{}",
                status.id()
            );
            assert_eq!(status.icon_stem(), format!("status-{}", status.id()));
            assert!(!status.initials().is_empty() && !status.label().is_empty());
            assert!(status.chip_colour().starts_with('#'));
        }
        assert_eq!(Status::from_id("frozen"), None);
        assert_eq!(Status::from_id(""), None);
        assert_eq!(Status::from_id("Poisoned"), None);
    }

    #[test]
    fn the_ids_are_exactly_the_agreed_twenty_seven() {
        let wanted = [
            "blinded",
            "charmed",
            "deafened",
            "exhaustion",
            "frightened",
            "grappled",
            "incapacitated",
            "invisible",
            "paralyzed",
            "petrified",
            "poisoned",
            "prone",
            "restrained",
            "stunned",
            "unconscious",
            "bloodied",
            "burning",
            "blessed",
            "cursed",
            "concentrating",
            "dodging",
            "hidden",
            "inspired",
            "raging",
            "marked",
            "dying",
            "cover",
            "stable",
        ];
        let ids: Vec<&str> = Status::ALL.iter().map(|status| status.id()).collect();
        assert_eq!(ids, wanted);
    }

    #[test]
    fn the_bit_set_keeps_each_status_and_splits_into_exact_shader_words() {
        let set = StatusSet::of(&[Status::Poisoned, Status::Cover, Status::Blinded]);
        assert_eq!(set.len(), 3);
        assert!(set.contains(Status::Cover) && !set.contains(Status::Prone));
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            vec![Status::Blinded, Status::Poisoned, Status::Cover]
        );
        assert_eq!(set.without(Status::Cover).len(), 2);
        let [low, high] = set.shader_words();
        // Blinded is bit 0, Poisoned bit 10, Cover bit 26 (= bit 12 of the high word).
        assert_eq!(low as u32, (1 << 10) | 1);
        assert_eq!(high as u32, 1 << 12);
        let everything = StatusSet::of(&Status::ALL);
        let [low, high] = everything.shader_words();
        assert!(low < 16_384.0 && high < 16_384.0);
    }

    #[test]
    fn petrified_overrides_every_other_colour_treatment() {
        let set = StatusSet::of(&[
            Status::Petrified,
            Status::Poisoned,
            Status::Cursed,
            Status::Charmed,
        ]);
        let resolved = resolve(set);
        assert_eq!(resolved.body, StatusSet::of(&[Status::Petrified]));
        assert!(resolved.edge.is_empty());
    }

    #[test]
    fn invisible_drops_the_body_but_keeps_the_edges_of_other_effects() {
        let set = StatusSet::of(&[Status::Invisible, Status::Burning, Status::Poisoned]);
        let resolved = resolve(set);
        assert!(resolved.invisible);
        assert!(resolved.body.is_empty());
        assert!(
            resolved.edge.contains(Status::Invisible),
            "the faint outline itself"
        );
        assert!(resolved.edge.contains(Status::Burning));
        assert!(resolved.overlay.contains(Status::Burning));
    }

    #[test]
    fn prone_and_poisoned_both_show() {
        let set = StatusSet::of(&[Status::Prone, Status::Poisoned]);
        assert!(resolve(set).body.contains(Status::Poisoned));
        let pose = pose_for(set);
        assert_eq!(pose.slump, 1.0);
        assert_eq!(pose.dim, 0.0, "prone is conscious: normal colour");
    }

    #[test]
    fn unconscious_slumps_dim_and_dying_adds_a_heartbeat() {
        let unconscious = pose_for(StatusSet::of(&[Status::Unconscious]));
        assert_eq!((unconscious.slump, unconscious.idle), (1.0, 0.0));
        assert!(unconscious.dim > 0.3);
        let dying = pose_for(StatusSet::of(&[Status::Dying, Status::Unconscious]));
        assert!(dying.heartbeat);
    }

    #[test]
    /// SRD 5.2.1 p18: a Stable creature remains Unconscious but no longer makes death saves.
    fn stable_replaces_the_dying_icon_and_heartbeat() {
        let set = StatusSet::of(&[Status::Stable, Status::Unconscious, Status::Dying]);
        assert_eq!(Status::from_id("stable"), Some(Status::Stable));
        assert_eq!(icon_statuses(set), vec![Status::Stable]);
        let pose = pose_for(set);
        assert_eq!((pose.slump, pose.idle), (1.0, 0.0));
        assert!(!pose.heartbeat);
    }

    #[test]
    fn stone_and_paralysis_are_rigid_and_restraints_constrain_motion() {
        assert_eq!(pose_for(StatusSet::of(&[Status::Petrified])).idle, 0.0);
        assert_eq!(pose_for(StatusSet::of(&[Status::Paralyzed])).idle, 0.0);
        let restrained = pose_for(StatusSet::of(&[Status::Restrained]));
        assert!(restrained.idle > 0.0 && restrained.idle < 0.5);
        assert!(pose_for(StatusSet::of(&[Status::Raging])).breath > 2.0);
    }

    #[test]
    fn stunned_frightened_and_dodging_move_and_the_pose_is_kept() {
        let set = StatusSet::of(&[
            Status::Stunned,
            Status::Frightened,
            Status::Dodging,
            Status::Prone,
        ]);
        let pose = pose_for(set);
        assert!(pose.wobble > 0.0 && pose.shiver > 0.0 && pose.afterimage);
        assert_eq!(pose.slump, 1.0);
        assert!(pose.flinch > 0.0);
    }

    #[test]
    fn invisible_is_faint() {
        assert!(pose_for(StatusSet::of(&[Status::Invisible])).alpha <= 0.3);
        assert_eq!(pose_for(StatusSet::EMPTY).alpha, 1.0);
    }

    #[test]
    fn only_bloodied_and_unconscious_are_derived_from_the_view() {
        assert_eq!(derived_from_view(12, 12, false), vec![]);
        assert_eq!(derived_from_view(7, 12, false), vec![]);
        assert_eq!(
            derived_from_view(6, 12, false),
            vec![Status::Bloodied],
            "half counts"
        );
        assert_eq!(derived_from_view(1, 12, false), vec![Status::Bloodied]);
        assert_eq!(derived_from_view(0, 12, true), vec![Status::Unconscious]);
        assert_eq!(
            derived_from_view(0, 12, false),
            vec![],
            "zero alone is the stage's fall"
        );
        assert_eq!(
            derived_from_view(3, 12, true),
            vec![Status::Bloodied, Status::Unconscious]
        );
        assert_eq!(derived_from_view(0, 0, false), vec![]);
        for status in derived_from_view(2, 12, true) {
            assert!(matches!(status, Status::Bloodied | Status::Unconscious));
        }
    }

    #[test]
    fn cover_is_a_tile_marker_not_an_icon() {
        let icons = icon_statuses(StatusSet::of(&[Status::Cover, Status::Marked]));
        assert_eq!(icons, vec![Status::Marked]);
    }

    #[test]
    fn an_unconscious_figure_keeps_only_its_skull_icon() {
        let icons = icon_statuses(StatusSet::of(&[
            Status::Unconscious,
            Status::Bloodied,
            Status::Poisoned,
        ]));
        assert_eq!(icons, vec![Status::Unconscious]);
    }
}
