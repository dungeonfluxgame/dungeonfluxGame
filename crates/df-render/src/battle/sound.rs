//! Combat sound as data. The stage emits [`SoundEvent`]s at the exact moment a timeline step
//! happens; the page adapter turns each into a clip with the sound-effects engine. Nothing here
//! plays audio, and the choice of voice follows the token: a hero by seat figure, an enemy by its
//! figure family. [`voice_for`] is the single mapping the server's cast gender/kind replaces.

use super::model::{Side, TokenId};

/// The surface under a scene's ground, which sets the footstep sound.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Surface {
    Wood,
    Dirt,
    #[default]
    Stone,
}

impl Surface {
    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "wood" => Self::Wood,
            "dirt" => Self::Dirt,
            "stone" => Self::Stone,
            _ => return None,
        })
    }
}

/// Which family of vocalisations a combatant uses.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum VoiceSet {
    HeroMale,
    HeroFemale,
    /// Goblins and kobolds: sharp yelps and snarls.
    SmallCreature,
    /// Wolves, rats, mastiffs: growls and yelps.
    Beast,
    /// Skeletons and zombies: dry rasps.
    Undead,
    /// Bandits, guards, cultists, scouts.
    Humanoid,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SoundKind {
    Footstep,
    SwordSwing,
    BladeHit,
    Parry,
    BowDraw,
    BowRelease,
    ArrowFlight,
    ArrowImpact,
    ArrowMiss,
    Effort,
    Pain,
    Down,
    BodyFall,
    HealBreath,
    DiceRattle,
    DiceLand,
    /// A short percussive hit at each roll-call stop; the stage raises its pitch stop by stop.
    InitiativeStop,
    /// The heavier hit at the last stop.
    InitiativeFinal,
    /// The sting when the party wins a fight.
    VictorySting,
    /// The toll when the party falls.
    DefeatSting,
}

/// How graphic the combat sound is. The game is for adults, so `Mature` is the default; the
/// `Restrained` set swaps the wet, bone-crunching impacts and death screams for plainer ones. One
/// switch (the page's content rating) selects the set; no sound is ever sexualised in either.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ContentRating {
    #[default]
    Mature,
    Restrained,
}

impl ContentRating {
    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "mature" => Self::Mature,
            "restrained" => Self::Restrained,
            _ => return None,
        })
    }
}

/// One sound to play now. `token` is whose body it comes from.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundEvent {
    pub kind: SoundKind,
    pub voice: VoiceSet,
    pub surface: Surface,
    pub token: TokenId,
    /// Playback speed: 1 normally, above 1 for a higher pitch (the rising roll-call stings).
    pub rate: f32,
}

/// The voice set for a combatant. Heroes follow the figure art they stand as (`figure-hero-1` is
/// male, `figure-hero-2` and `figure-hero-3` female); enemies follow the creature family named by
/// their figure stem. Switch point: the server's cast gender and creature kind replace this.
pub fn voice_for(side: Side, figure: Option<&str>) -> VoiceSet {
    let stem = figure.unwrap_or("");
    match side {
        Side::Hero => match stem {
            "figure-hero-2" | "figure-hero-3" => VoiceSet::HeroFemale,
            _ => VoiceSet::HeroMale,
        },
        Side::Enemy => {
            if stem.contains("goblin") || stem.contains("kobold") {
                VoiceSet::SmallCreature
            } else if ["rat", "weasel", "wolf", "mastiff"]
                .iter()
                .any(|beast| stem.contains(beast))
            {
                VoiceSet::Beast
            } else if stem.contains("skeleton") || stem.contains("zombie") {
                VoiceSet::Undead
            } else {
                VoiceSet::Humanoid
            }
        }
    }
}

const WOOD_STEPS: &[&str] = &["step-wood-1", "step-wood-2"];
const DIRT_STEPS: &[&str] = &["step-dirt-1", "step-dirt-2"];
const STONE_STEPS: &[&str] = &["step-stone-1", "step-stone-2"];
const SWINGS: &[&str] = &["sword-swing-1", "sword-swing-2"];
const HITS: &[&str] = &["blade-hit-1", "blade-hit-2"];

macro_rules! voice_group {
    ($voice:expr, [$($set:ident => [$($clip:literal),+]),+ $(,)?]) => {
        match $voice {
            $(VoiceSet::$set => &[$($clip),+] as &'static [&'static str]),+
        }
    };
}

impl SoundEvent {
    /// The clips this event may play; the engine picks one without repeating the last. The
    /// restrained set replaces the graphic impacts and death cries with the plainer pack clips.
    pub fn clips(&self, rating: ContentRating) -> &'static [&'static str] {
        if rating == ContentRating::Restrained {
            match self.kind {
                SoundKind::BladeHit | SoundKind::ArrowImpact => return &["hit"],
                SoundKind::Down => {
                    return SoundEvent {
                        kind: SoundKind::Pain,
                        ..self.clone()
                    }
                    .clips(rating);
                }
                _ => {}
            }
        }
        match self.kind {
            SoundKind::Footstep => match self.surface {
                Surface::Wood => WOOD_STEPS,
                Surface::Dirt => DIRT_STEPS,
                Surface::Stone => STONE_STEPS,
            },
            SoundKind::SwordSwing => SWINGS,
            SoundKind::BladeHit => HITS,
            SoundKind::Parry => &["weapon-parry"],
            SoundKind::BowDraw => &["bow-draw"],
            SoundKind::BowRelease => &["bow-release"],
            SoundKind::ArrowFlight => &["arrow-flight"],
            SoundKind::ArrowImpact => &["arrow-impact"],
            SoundKind::ArrowMiss => &["arrow-miss"],
            SoundKind::Effort => voice_group!(self.voice, [
                HeroMale => ["voice-male-effort", "voice-male-effort-2"],
                HeroFemale => ["voice-female-effort", "voice-female-effort-2"],
                SmallCreature => ["voice-small-effort"],
                Beast => ["voice-beast-effort"],
                Undead => ["voice-undead-effort"],
                Humanoid => ["voice-humanoid-effort"],
            ]),
            SoundKind::Pain => voice_group!(self.voice, [
                HeroMale => ["voice-male-pain"],
                HeroFemale => ["voice-female-pain"],
                SmallCreature => ["voice-small-pain"],
                Beast => ["voice-beast-pain"],
                Undead => ["voice-undead-pain"],
                Humanoid => ["voice-humanoid-pain"],
            ]),
            SoundKind::Down => voice_group!(self.voice, [
                HeroMale => ["voice-male-down"],
                HeroFemale => ["voice-female-down"],
                SmallCreature => ["voice-small-down"],
                Beast => ["voice-beast-down"],
                Undead => ["voice-undead-down"],
                Humanoid => ["voice-humanoid-down"],
            ]),
            SoundKind::BodyFall => &["body-fall"],
            SoundKind::HealBreath => &["heal-breath"],
            SoundKind::DiceRattle => &["dice-rattle"],
            SoundKind::DiceLand | SoundKind::InitiativeFinal => &["dice-land"],
            SoundKind::InitiativeStop => &["bubble-pop"],
            SoundKind::VictorySting => &["victory-fanfare"],
            SoundKind::DefeatSting => &["defeat-toll"],
        }
    }
}

impl SoundKind {
    /// Level against the rest of the pack: steps sit under everything, voices a little under
    /// weapons. Applied before the engine's own ducking under narration.
    pub fn gain(self) -> f32 {
        match self {
            Self::Footstep => 0.5,
            Self::ArrowFlight | Self::ArrowMiss => 0.7,
            Self::Effort | Self::Pain | Self::Down | Self::HealBreath => 0.85,
            _ => 1.0,
        }
    }
}

/// Every clip the battlefield can ask for, which the served pack must hold.
pub const ALL_CLIPS: &[&str] = &[
    "step-wood-1",
    "step-wood-2",
    "step-dirt-1",
    "step-dirt-2",
    "step-stone-1",
    "step-stone-2",
    "sword-swing-1",
    "sword-swing-2",
    "blade-hit-1",
    "blade-hit-2",
    "weapon-parry",
    "bow-draw",
    "bow-release",
    "arrow-flight",
    "arrow-impact",
    "arrow-miss",
    "voice-male-effort",
    "voice-male-effort-2",
    "voice-male-pain",
    "voice-male-down",
    "voice-female-effort",
    "voice-female-effort-2",
    "voice-female-pain",
    "voice-female-down",
    "voice-small-effort",
    "voice-small-pain",
    "voice-small-down",
    "voice-beast-effort",
    "voice-beast-pain",
    "voice-beast-down",
    "voice-undead-effort",
    "voice-undead-pain",
    "voice-undead-down",
    "voice-humanoid-effort",
    "voice-humanoid-pain",
    "voice-humanoid-down",
    "body-fall",
    "heal-breath",
    // Reused from the original pack for the initiative roll.
    "dice-rattle",
    "dice-land",
    "bubble-pop",
    // The stings that end a fight, from the display's sound effects.
    "victory-fanfare",
    "defeat-toll",
    // The plainer impact used by the restrained set; part of the original pack.
    "hit",
];

/// Picks among a group's clips without playing the same one twice in a row. Deterministic for a
/// seed, so a replay of the same fight sounds the same.
#[derive(Debug)]
pub struct VariationPicker {
    state: u32,
    last: Vec<(&'static str, usize)>,
}

impl VariationPicker {
    pub fn new(seed: u32) -> Self {
        Self {
            state: seed | 1,
            last: Vec::new(),
        }
    }

    fn next(&mut self) -> u32 {
        // xorshift32: small, fast and good enough to vary a footstep.
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        self.state
    }

    /// `None` only for an empty group.
    pub fn pick(&mut self, group: &'static [&'static str]) -> Option<&'static str> {
        let first = *group.first()?;
        if group.len() == 1 {
            return Some(first);
        }
        let previous = self
            .last
            .iter()
            .find(|(key, _)| *key == first)
            .map(|(_, index)| *index);
        let mut index = self.next() as usize % group.len();
        if Some(index) == previous {
            index = (index + 1 + self.next() as usize % (group.len() - 1)) % group.len();
        }
        match self.last.iter_mut().find(|(key, _)| *key == first) {
            Some(entry) => entry.1 = index,
            None => self.last.push((first, index)),
        }
        group.get(index).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: SoundKind, voice: VoiceSet, surface: Surface) -> SoundEvent {
        SoundEvent {
            kind,
            voice,
            surface,
            token: TokenId::new("t"),
            rate: 1.0,
        }
    }

    #[test]
    fn heroes_follow_their_figure_and_enemies_their_creature_family() {
        assert_eq!(
            voice_for(Side::Hero, Some("figure-hero-1")),
            VoiceSet::HeroMale
        );
        assert_eq!(
            voice_for(Side::Hero, Some("figure-hero-2")),
            VoiceSet::HeroFemale
        );
        assert_eq!(
            voice_for(Side::Hero, Some("figure-hero-3")),
            VoiceSet::HeroFemale
        );
        assert_eq!(voice_for(Side::Hero, None), VoiceSet::HeroMale);
        let enemy = |stem| voice_for(Side::Enemy, Some(stem));
        assert_eq!(enemy("figure-ant-goblin-warrior"), VoiceSet::SmallCreature);
        assert_eq!(enemy("figure-ant-kobold-warrior"), VoiceSet::SmallCreature);
        assert_eq!(enemy("figure-ant-wolf"), VoiceSet::Beast);
        assert_eq!(enemy("figure-ant-giant-rat"), VoiceSet::Beast);
        assert_eq!(enemy("figure-ant-mastiff"), VoiceSet::Beast);
        assert_eq!(enemy("figure-ant-skeleton"), VoiceSet::Undead);
        assert_eq!(enemy("figure-ant-zombie"), VoiceSet::Undead);
        assert_eq!(enemy("figure-ant-bandit"), VoiceSet::Humanoid);
        assert_eq!(enemy("figure-ant-cultist"), VoiceSet::Humanoid);
        assert_eq!(voice_for(Side::Enemy, None), VoiceSet::Humanoid);
    }

    #[test]
    fn every_group_clip_is_in_the_pack_list_and_the_list_has_no_duplicates() {
        let voices = [
            VoiceSet::HeroMale,
            VoiceSet::HeroFemale,
            VoiceSet::SmallCreature,
            VoiceSet::Beast,
            VoiceSet::Undead,
            VoiceSet::Humanoid,
        ];
        let kinds = [
            SoundKind::Footstep,
            SoundKind::SwordSwing,
            SoundKind::BladeHit,
            SoundKind::Parry,
            SoundKind::BowDraw,
            SoundKind::BowRelease,
            SoundKind::ArrowFlight,
            SoundKind::ArrowImpact,
            SoundKind::ArrowMiss,
            SoundKind::Effort,
            SoundKind::Pain,
            SoundKind::Down,
            SoundKind::BodyFall,
            SoundKind::HealBreath,
            SoundKind::DiceRattle,
            SoundKind::DiceLand,
            SoundKind::InitiativeStop,
            SoundKind::InitiativeFinal,
            SoundKind::VictorySting,
            SoundKind::DefeatSting,
        ];
        let mut used = std::collections::BTreeSet::new();
        for rating in [ContentRating::Mature, ContentRating::Restrained] {
            for kind in kinds {
                for voice in voices {
                    for surface in [Surface::Wood, Surface::Dirt, Surface::Stone] {
                        let clips = event(kind, voice, surface).clips(rating);
                        assert!(!clips.is_empty());
                        for clip in clips {
                            assert!(ALL_CLIPS.contains(clip), "{clip} missing from ALL_CLIPS");
                            used.insert(*clip);
                        }
                    }
                }
            }
        }
        assert_eq!(
            used.len(),
            ALL_CLIPS.len(),
            "a listed clip is never asked for"
        );
        let unique: std::collections::BTreeSet<_> = ALL_CLIPS.iter().collect();
        assert_eq!(unique.len(), ALL_CLIPS.len());
    }

    #[test]
    fn the_restrained_set_swaps_graphic_impacts_and_death_cries_for_plain_ones() {
        let mature = |kind, voice| event(kind, voice, Surface::Stone).clips(ContentRating::Mature);
        let plain =
            |kind, voice| event(kind, voice, Surface::Stone).clips(ContentRating::Restrained);
        assert_eq!(
            mature(SoundKind::BladeHit, VoiceSet::HeroMale),
            &["blade-hit-1", "blade-hit-2"]
        );
        assert_eq!(plain(SoundKind::BladeHit, VoiceSet::HeroMale), &["hit"]);
        assert_eq!(plain(SoundKind::ArrowImpact, VoiceSet::Beast), &["hit"]);
        assert_eq!(
            mature(SoundKind::Down, VoiceSet::Undead),
            &["voice-undead-down"]
        );
        assert_eq!(
            plain(SoundKind::Down, VoiceSet::Undead),
            plain(SoundKind::Pain, VoiceSet::Undead)
        );
        // Everything else is the same in both sets.
        assert_eq!(
            mature(SoundKind::SwordSwing, VoiceSet::HeroMale),
            plain(SoundKind::SwordSwing, VoiceSet::HeroMale)
        );
        assert_eq!(ContentRating::default(), ContentRating::Mature);
        assert_eq!(
            ContentRating::from_key("restrained"),
            Some(ContentRating::Restrained)
        );
        assert_eq!(ContentRating::from_key("PG"), None);
    }

    #[test]
    fn each_surface_has_its_own_footsteps_with_at_least_two_variations() {
        let steps = |surface| {
            event(SoundKind::Footstep, VoiceSet::HeroMale, surface).clips(ContentRating::Mature)
        };
        let (wood, dirt, stone) = (
            steps(Surface::Wood),
            steps(Surface::Dirt),
            steps(Surface::Stone),
        );
        for group in [wood, dirt, stone] {
            assert!(group.len() >= 2);
        }
        assert!(wood.iter().all(|clip| clip.contains("wood")));
        assert!(dirt.iter().all(|clip| clip.contains("dirt")));
        assert!(stone.iter().all(|clip| clip.contains("stone")));
    }

    #[test]
    fn the_picker_never_repeats_a_clip_and_is_deterministic() {
        let group = &["a", "b", "c"][..];
        let two = &["x", "y"][..];
        let run = |seed| {
            let mut picker = VariationPicker::new(seed);
            (0..500)
                .flat_map(|_| [picker.pick(group).unwrap(), picker.pick(two).unwrap()])
                .collect::<Vec<_>>()
        };
        let picks = run(7);
        assert_eq!(picks, run(7));
        for lane in 0..2 {
            let lane_picks: Vec<_> = picks.iter().skip(lane).step_by(2).collect();
            assert!(lane_picks.windows(2).all(|pair| pair[0] != pair[1]));
        }
        assert!(["a", "b", "c"].iter().all(|clip| picks.contains(clip)));
        let mut picker = VariationPicker::new(0);
        assert_eq!(picker.pick(&["only"]), Some("only"));
        assert_eq!(picker.pick(&[]), None);
    }

    #[test]
    fn the_pack_ships_every_clip_and_the_server_allowlist_names_it() {
        let allowlist = include_str!("../../../df-tools/src/gameplay/startup_configuration.rs");
        let folder = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/ui/generated/sfx");
        for clip in ALL_CLIPS {
            assert!(
                allowlist.contains(&format!("\"{clip}\",")),
                "{clip} not in SFX_NAMES"
            );
            let bytes = std::fs::metadata(format!("{folder}/{clip}.mp3"))
                .unwrap_or_else(|error| panic!("{clip}.mp3: {error}"))
                .len();
            assert!(
                (2_000..200_000).contains(&bytes),
                "{clip}.mp3 is {bytes} bytes"
            );
        }
    }
}
