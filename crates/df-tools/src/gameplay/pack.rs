//! Gameplay pack and tuning loader (design: `planning/gameplay-tuning.md`, sections 3 to 5).
//!
//! Two checked-in JSON files describe the Lantern Wharf harbor journey: `pack.json` holds
//! scenario content and `tuning.json` holds gameplay numbers. [`load`] reads each file once under
//! a size cap, hashes the exact bytes it parsed, refuses unknown or duplicate keys and wrong types,
//! enforces the bounds table, checks the cross-file references and reports the values that differ
//! from the compiled SRD 5.2.1 baseline. Only an absent optional tempo section selects the
//! explicitly shipped presentation profile. For required gameplay files and fields, a missing
//! directory, missing file or any violation is a typed [`PackError`] naming the file and the JSON
//! location.
//!
//! The SHA-256 values are development revisions of the exact file bytes. They are not a published
//! content-pack version or a rights-backed identity (decision record D01).
use df_render::art_contract;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

/// File name of the scenario content inside the pack directory.
pub(super) const PACK_FILE_NAME: &str = "pack.json";
/// File name of the gameplay numbers inside the pack directory.
pub(super) const TUNING_FILE_NAME: &str = "tuning.json";
/// Largest accepted size of either file, in bytes.
pub(super) const MAX_FILE_BYTES: u64 = 256 * 1024;
/// Required value of `schema` in `pack.json`.
pub(super) const PACK_SCHEMA: &str = "df-gameplay-pack/1";
/// Required value of `schema` in `tuning.json`.
pub(super) const TUNING_SCHEMA: &str = "df-gameplay-tuning/1";

pub(super) const TEXT_CHARACTERS_MAX: usize = 1024;
const LABEL_CHARACTERS_MAX: usize = 96;
const ID_CHARACTERS_MAX: usize = 64;
const DIE_SIDES: &[u8] = &[4, 6, 8, 10, 12];
const ABILITY_SCORE_CAP: i64 = 20;

/// Items the compiled journey can place in an inventory (`journey::CONTENT_ENTRIES`).
pub(super) const COMPILED_ITEMS: &[&str] = &[
    "chain-mail",
    "greatsword",
    "flail",
    "javelin",
    "dungeoneer-pack",
    "spear",
    "shortbow",
    "arrow",
    "gaming-set",
    "healers-kit",
    "quiver",
    "travelers-clothes",
    "gold-piece",
    "scimitar",
    "club",
    "studded-leather-armor",
    "longbow",
    "shortsword",
];
/// Compiled items that may be configured as a weapon.
const COMPILED_WEAPONS: &[&str] = &[
    "greatsword",
    "flail",
    "javelin",
    "spear",
    "shortbow",
    "scimitar",
    "club",
];
/// Scene art the gameplay server serves (`public_assets.rs`), without the leading slash.
pub(super) const SERVED_SCENE_ASSETS: &[&str] = &[
    "assets/ui/scenes/mara-harbor-v4.png",
    "assets/concept-art/scene-campfire-under-stars.webp",
    "assets/concept-art/vell-avatar.webp",
    "assets/concept-art/scene-tavern-barkeep-talk-rain.webp",
];
/// Single-option creation groups and the one option id the compiled validator accepts for each.
const FIXED_GROUPS: &[(&str, &str)] = &[
    ("class", "fighter-1"),
    ("background", "soldier"),
    ("equipment", "fighter-a-soldier-a"),
    ("style-masteries", "defense-greatsword-flail-javelin"),
    ("allied-ties", "join-order"),
];
const ARRAY_GROUP: &str = "array";
const ARRAY_TEMPLATE_PLACEHOLDERS: &[&str] =
    &["0", "1", "2", "3", "4", "5", "bonus_str", "bonus_con"];

/// Which of the two files an error or value belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SourceFile {
    Pack,
    Tuning,
}

impl SourceFile {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Pack => PACK_FILE_NAME,
            Self::Tuning => TUNING_FILE_NAME,
        }
    }
}

/// Broad class of a JSON parse failure, taken from the parser rather than from message text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ParseErrorKind {
    /// Not well-formed JSON (including trailing data after the document).
    Syntax,
    /// The document ended early.
    UnexpectedEnd,
    /// Well-formed JSON that does not match the schema: unknown key, duplicate key, missing key,
    /// wrong type or a number outside the field's integer type.
    Data,
}

/// Why a reference between or inside the files does not resolve.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CrossRefReason {
    PackIdMismatch,
    DuplicateId,
    UnknownGroup,
    DuplicateGroup,
    MissingGroup,
    UnknownOption,
    TemplateMisplaced,
    ArrayWithoutOption,
    OptionWithoutArray,
    UnknownPickValue,
    UnknownItem,
    WeaponNotEquipped,
    AssetNotServed,
    DuplicateScene,
    MissingScene,
    RestedNarrationMismatch,
}

impl fmt::Display for CrossRefReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PackIdMismatch => "tuning names a different pack than pack.json",
            Self::DuplicateId => "duplicate entry",
            Self::UnknownGroup => "creation group is not supported by the compiled validator",
            Self::DuplicateGroup => "creation group appears more than once",
            Self::MissingGroup => "required creation group is missing",
            Self::UnknownOption => "option is not accepted by the compiled validator",
            Self::TemplateMisplaced => {
                "description_template belongs only to the ability array group"
            }
            Self::ArrayWithoutOption => {
                "ability array has no matching creation option in pack.json"
            }
            Self::OptionWithoutArray => {
                "creation option has no matching ability array in tuning.json"
            }
            Self::UnknownPickValue => "value is not accepted by the compiled validator",
            Self::UnknownItem => "item is not in the compiled item set",
            Self::WeaponNotEquipped => "hero weapon has no starting equipment row in pack.json",
            Self::AssetNotServed => "asset is not in the served scene allowlist",
            Self::DuplicateScene => "scene key appears more than once",
            Self::MissingScene => "required scene is missing",
            Self::RestedNarrationMismatch => {
                "narration_rested is required for victory and defeat and forbidden elsewhere"
            }
        })
    }
}

/// Typed refusal to load the pack. Every variant names the file; `Display` never prints file
/// contents beyond a short, truncated parser message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PackError {
    Script(super::beat_script::ScriptError),
    /// The file (or its directory) does not exist.
    Missing {
        file: SourceFile,
        path: PathBuf,
    },
    /// The file exists but could not be read.
    Io {
        file: SourceFile,
        path: PathBuf,
        kind: io::ErrorKind,
    },
    /// The file exceeds [`MAX_FILE_BYTES`].
    TooLarge {
        file: SourceFile,
        bytes: u64,
        limit: u64,
    },
    /// Not valid for the schema: syntax, unknown key, duplicate key, missing key, wrong type.
    /// `path` is the JSON path (`.` for the document root).
    Parse {
        file: SourceFile,
        path: String,
        line: usize,
        column: usize,
        kind: ParseErrorKind,
        message: String,
    },
    /// The `schema` marker is not the version this loader reads.
    Schema {
        file: SourceFile,
        found: String,
        expected: &'static str,
    },
    /// A number or text length is outside its range.
    Bounds {
        file: SourceFile,
        path: String,
        actual: i64,
        min: i64,
        max: i64,
    },
    /// A number is not one of a fixed set of allowed values.
    NotAllowed {
        file: SourceFile,
        path: String,
        actual: i64,
        allowed: &'static [u8],
    },
    /// Text that must have a given form (identifier, locale, placeholders) does not.
    Malformed {
        file: SourceFile,
        path: String,
        expected: &'static str,
    },
    /// A reference does not resolve.
    CrossRef {
        file: SourceFile,
        path: String,
        reason: CrossRefReason,
    },
}

impl fmt::Display for PackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Script(error) => error.fmt(formatter),
            Self::Missing { file, path } => write!(
                formatter,
                "gameplay file {} is missing (expected at {})",
                file.name(),
                path.display()
            ),
            Self::Io { file, path, kind } => write!(
                formatter,
                "gameplay file {} could not be read at {}: {kind}",
                file.name(),
                path.display()
            ),
            Self::TooLarge { file, bytes, limit } => write!(
                formatter,
                "gameplay file {} is {bytes} bytes, above the {limit} byte limit",
                file.name()
            ),
            Self::Parse {
                file,
                path,
                line,
                column,
                message,
                ..
            } => write!(
                formatter,
                "gameplay file {} is invalid at {path} (line {line}, column {column}): {message}",
                file.name()
            ),
            Self::Schema {
                file,
                found,
                expected,
            } => write!(
                formatter,
                "gameplay file {} has schema {found:?}, expected {expected:?}",
                file.name()
            ),
            Self::Bounds {
                file,
                path,
                actual,
                min,
                max,
            } => write!(
                formatter,
                "gameplay file {} value at {path} is {actual}, outside {min}..={max}",
                file.name()
            ),
            Self::NotAllowed {
                file,
                path,
                actual,
                allowed,
            } => write!(
                formatter,
                "gameplay file {} value at {path} is {actual}, not one of {allowed:?}",
                file.name()
            ),
            Self::Malformed {
                file,
                path,
                expected,
            } => write!(
                formatter,
                "gameplay file {} value at {path} must be {expected}",
                file.name()
            ),
            Self::CrossRef { file, path, reason } => {
                write!(
                    formatter,
                    "gameplay file {} at {path}: {reason}",
                    file.name()
                )
            }
        }
    }
}

impl std::error::Error for PackError {}

/// SHA-256 of the exact bytes of one file: a development revision, not a published version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Revision([u8; 32]);

impl Revision {
    pub(super) fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    pub(super) const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hexadecimal form (64 characters).
    pub(super) fn to_hex(self) -> String {
        self.as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .iter()
            .try_for_each(|byte| write!(formatter, "{byte:02x}"))
    }
}

// ---------------------------------------------------------------------------------------------
// tuning.json
// ---------------------------------------------------------------------------------------------

/// Typed `tuning.json`. Ability score order is STR, DEX, CON, INT, WIS, CHA.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct TuningFile {
    pub(super) schema: String,
    /// Id of the pack these numbers belong to; must equal `pack.json`'s `id`.
    pub(super) pack: String,
    pub(super) hero: HeroTuning,
    #[serde(default)]
    pub(super) bandit: BanditTuning,
    #[serde(default)]
    pub(super) combat: CombatTuning,
    /// Absence retains the immutable legacy adjacent-fight profile for existing packs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) tactical: Option<TacticalEncounterTuning>,
    #[serde(default)]
    pub(super) rest: RestTuning,
    #[serde(default)]
    pub(super) narrative: NarrativeTuning,
    /// Missing section explicitly selects the shipped presentation profile.
    #[serde(default)]
    pub(super) tempo: TempoTuning,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct HeroTuning {
    pub(super) armor_class: u8,
    pub(super) proficiency_bonus: u8,
    /// Hit points before the Constitution modifier is added.
    pub(super) base_hit_points: u8,
    pub(super) ability_arrays: Vec<AbilityArray>,
    pub(super) background_bonus: BackgroundBonus,
    pub(super) second_wind: SecondWindTuning,
    pub(super) weapon: WeaponTuning,
}

/// One selectable standard array. `scores` are before the background bonus.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct AbilityArray {
    pub(super) id: String,
    pub(super) scores: [u8; 6],
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct BackgroundBonus {
    pub(super) strength: u8,
    pub(super) constitution: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct SecondWindTuning {
    pub(super) uses_max: u8,
    pub(super) die_sides: u8,
    pub(super) flat_bonus: u8,
    pub(super) recharge_per_short_rest: u8,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct WeaponTuning {
    #[serde(alias = "label")]
    pub(super) item: String,
    pub(super) damage_dice: u8,
    pub(super) damage_sides: u8,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct BanditTuning {
    pub(super) hit_points: u8,
    pub(super) armor_class: u8,
    pub(super) abilities: AbilityScores,
    pub(super) attack_bonus: i8,
    pub(super) damage_modifier: u8,
    pub(super) weapon: AntagonistWeaponTuning,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct AntagonistWeaponTuning {
    #[serde(rename = "label", alias = "item")]
    pub(super) item: String,
    pub(super) damage_dice: u8,
    pub(super) damage_sides: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct AbilityScores {
    pub(super) strength: u8,
    pub(super) dexterity: u8,
    pub(super) constitution: u8,
    pub(super) intelligence: u8,
    pub(super) wisdom: u8,
    pub(super) charisma: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct CombatTuning {
    /// Campaign ticks in one combat round.
    pub(super) round_ticks: u32,
    /// Lowest natural d20 face that is a critical hit.
    pub(super) critical_threshold: u8,
    /// Combat log entries sent to clients.
    pub(super) combat_log_entries: u8,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct TacticalEncounterTuning {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) enemies: Vec<TacticalEnemy>,
    /// Freeplay only: the tactical engine is on so a hero's attack on a scene person can start a
    /// real fight, but the pack has no roster, so no random encounter can ever start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) scene_assault: Option<SceneAssaultTuning>,
}
/// The scene-assault-only variant of the tactical block.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct SceneAssaultTuning {
    /// Id of the authored battle scene (a grid the server holds) the fights are played on.
    pub(super) battle_scene: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct TacticalEnemy {
    pub(super) srd_id: String,
    pub(super) count: u8,
    pub(super) ammunition: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct RestTuning {
    pub(super) short_rest_seconds: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct NarrativeTuning {
    pub(super) intervention_budget: u8,
}

/// Presentation-only tuning; deliberately excluded from standard-rules departures.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct TempoTuning {
    /// Room, opening, dialogue, combat, victory, defeat, rested, destination.
    pub(super) base_intensity_permille: TempoBases,
    /// Story, scene, reveal, combat, miss, hit, critical, graze, hero-low, drop,
    /// victory, defeat, rest, arrival. Positive attack impulses apply to either side.
    pub(super) impulses_permille: TempoImpulses,
    pub(super) inertia_half_life_millis: u32,
    pub(super) decay_half_life_millis: u32,
    pub(super) fatigue_half_life_millis: u32,
    pub(super) fatigue_charge_permille: u16,
    pub(super) fatigue_attenuation_permille: u16,
    pub(super) phase_thresholds_permille: [u16; 3],
    pub(super) hysteresis_permille: u16,
    /// How long the display's music holds a release (the win or the rest after a fight)
    /// before it settles to the calm bed. Sent in a Release cue's `hold_millis`.
    #[serde(
        default = "default_release_hold_millis",
        skip_serializing_if = "is_default_release_hold"
    )]
    pub(super) release_hold_millis: u32,
}
const fn default_release_hold_millis() -> u32 {
    14_000
}
/// The default is left out when a tuning is written, so tunings and generated adventures that
/// never mention the hold keep their exact bytes (and revisions).
const fn is_default_release_hold(millis: &u32) -> bool {
    *millis == default_release_hold_millis()
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct TempoBases {
    pub(super) room: u16,
    pub(super) opening: u16,
    pub(super) dialogue: u16,
    pub(super) combat: u16,
    pub(super) victory: u16,
    pub(super) defeat: u16,
    pub(super) rest: u16,
    pub(super) arrival: u16,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct TempoImpulses {
    pub(super) story_begun: i16,
    pub(super) scene_changed: i16,
    pub(super) antagonist_revealed: i16,
    pub(super) combat_begun: i16,
    pub(super) miss: i16,
    pub(super) hit: i16,
    pub(super) critical: i16,
    pub(super) graze: i16,
    pub(super) hero_low: i16,
    pub(super) combatant_dropped: i16,
    pub(super) victory: i16,
    pub(super) defeat: i16,
    pub(super) short_rest: i16,
    pub(super) arrival: i16,
}

impl Default for TempoTuning {
    fn default() -> Self {
        Self {
            base_intensity_permille: TempoBases {
                room: 60,
                opening: 260,
                dialogue: 480,
                combat: 720,
                victory: 200,
                defeat: 300,
                rest: 80,
                arrival: 60,
            },
            impulses_permille: TempoImpulses {
                story_begun: 180,
                scene_changed: 30,
                antagonist_revealed: 180,
                combat_begun: 220,
                miss: 20,
                hit: 65,
                critical: 150,
                graze: 40,
                hero_low: 180,
                combatant_dropped: 100,
                victory: -200,
                defeat: -80,
                short_rest: -140,
                arrival: -80,
            },
            inertia_half_life_millis: 2000,
            decay_half_life_millis: 6000,
            fatigue_half_life_millis: 20000,
            fatigue_charge_permille: 60,
            fatigue_attenuation_permille: 500,
            phase_thresholds_permille: [200, 450, 650],
            hysteresis_permille: 20,
            release_hold_millis: default_release_hold_millis(),
        }
    }
}
impl TempoTuning {
    pub(super) fn policy(self) -> df_tempo::presentation::PresentationPolicy {
        df_tempo::presentation::PresentationPolicy {
            inertia_half_life_millis: self.inertia_half_life_millis,
            decay_half_life_millis: self.decay_half_life_millis,
            fatigue_half_life_millis: self.fatigue_half_life_millis,
            fatigue_charge_permille: self.fatigue_charge_permille,
            fatigue_attenuation_permille: self.fatigue_attenuation_permille,
            thresholds: self.phase_thresholds_permille,
            hysteresis_permille: self.hysteresis_permille,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// pack.json
// ---------------------------------------------------------------------------------------------

/// Typed `pack.json`.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct PackFile {
    pub(super) schema: String,
    pub(super) id: String,
    pub(super) locale: String,
    pub(super) room_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) reward: Option<ErrandReward>,
    /// Absence is the versioned canonical legacy journey, with unchanged stored bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) beat_script: Option<super::beat_script::BeatScript>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) campaign: Option<CampaignIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) campaign_epilogue: Option<FinaleProse>,
    pub(super) creation: CreationPack,
    pub(super) equipment: Vec<EquipmentRow>,
    #[serde(rename = "antagonist", alias = "bandit")]
    #[serde(default, skip_serializing_if = "BanditPack::is_absent")]
    pub(super) bandit: BanditPack,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) destination_label: Option<String>,
    pub(super) offer_labels: OfferLabels,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) courier_clue: String,
    /// Appended to scene narration after a space; contains the `{name}` placeholder once.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) escort_support_suffix: String,
    pub(super) scenes: Vec<Scene>,
    /// Phrases that give the private clue away. A listener who has not been told the clue
    /// must never hear them, whoever is speaking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) secret_terms: Vec<String>,
    /// Who each speaking character is, by the name a scene shows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) speakers: Vec<SpeakerProfile>,
    /// The music each place asks for, by place id. A place with no block gets its music chosen
    /// from the closed labels by Jev (see `place_music`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) music: Vec<super::place_music::PackMusic>,
}
impl PackFile {
    pub(super) fn is_freeplay(&self) -> bool {
        self.beat_script
            .as_ref()
            .is_some_and(|script| script.mode == super::beat_script::ScriptMode::Freeplay)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct ErrandReward {
    pub(super) gold_per_hero: u32,
    pub(super) favour: Option<String>,
}

/// One character on stage in a scene.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct SceneCastMember {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) role: CastRole,
    /// The cut-out figure's asset stem: the portrait's stem with `portrait-` replaced by
    /// `sprite-`.
    pub(super) sprite: String,
    pub(super) portrait: String,
    pub(super) voice: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) gender: Option<Gender>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) age: Option<Age>,
    /// The SRD 5.2.1 stat block this person fights with when a hero starts a scene assault
    /// (an id from `df_rules::SRD_521_CREATURES` or `df_rules::SCENE_COMMONER`). Absent means
    /// the person has no combat profile and cannot be attacked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) stat_block: Option<String>,
}

/// Whether a scene person's stat block is a creature this build compiles from SRD 5.2.1.
pub(super) fn scene_stat_block_known(id: &str) -> bool {
    df_rules::SRD_521_CREATURES
        .iter()
        .chain([&df_rules::SCENE_COMMONER])
        .any(|creature| creature.id == id)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum CastRole {
    Patron,
    Antagonist,
    Contact,
    Bystander,
}

/// The pronoun set a character is cast and voiced with.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Gender {
    He,
    She,
    They,
    It,
}

/// How old a character sounds.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Age {
    Young,
    Adult,
    Elder,
}

impl Gender {
    pub(super) fn from_table(text: &str) -> Option<Self> {
        match text {
            "he" => Some(Self::He),
            "she" => Some(Self::She),
            "they" => Some(Self::They),
            "it" => Some(Self::It),
            _ => None,
        }
    }
}

impl Age {
    pub(super) fn from_table(text: &str) -> Option<Self> {
        match text {
            "young" => Some(Self::Young),
            "adult" => Some(Self::Adult),
            "elder" => Some(Self::Elder),
            _ => None,
        }
    }
}

/// What a language model needs to voice one speaking character in character, and what voice
/// casting needs to pick a voice without reading the free-text `voice` phrase.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct SpeakerProfile {
    /// Equals the scene speaker label; it does not authorize disclosure of a personal name.
    pub(super) name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) gender: Option<Gender>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) age: Option<Age>,
    /// Who the character is to the party, in a short phrase.
    pub(super) role: String,
    /// How the character sounds, in a short phrase.
    pub(super) voice: String,
    /// Exact public answer to an identity question: a name, alias, role or authored deferral.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) identity_reply: Option<String>,
    /// Story-authored refusal subjects; no motive or employer fact is implied by withholding.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) withheld_subjects: Vec<super::adventure::dossier::QuestionSubject>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct CampaignIdentity {
    pub(super) id: String,
    pub(super) chapter: u8,
    pub(super) frame_sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct FinaleProse {
    pub(super) asked: String,
    pub(super) escorted: String,
    pub(super) defeat: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct CreationPack {
    pub(super) description: String,
    pub(super) groups: Vec<CreationGroup>,
    pub(super) pick_lists: PickLists,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct CreationGroup {
    pub(super) id: String,
    pub(super) label: String,
    /// Only the `array` group has a template. Placeholders: `{0}` to `{5}` are the six ability
    /// scores after the background bonus, `{bonus_str}` and `{bonus_con}` the bonuses; see
    /// [`render_array_description`].
    #[serde(default)]
    pub(super) description_template: Option<String>,
    pub(super) options: Vec<CreationOption>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct CreationOption {
    pub(super) id: String,
    pub(super) label: String,
    #[serde(default)]
    pub(super) description: Option<String>,
}

/// Selectable values for the free pick-list groups; each must be accepted by the compiled
/// validator, so a pack can only narrow them.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct PickLists {
    pub(super) alignment: Vec<String>,
    pub(super) language: Vec<String>,
    pub(super) skill: Vec<String>,
    #[serde(rename = "gaming-set")]
    pub(super) gaming_set: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct EquipmentRow {
    pub(super) item: String,
    pub(super) quantity: u8,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct BanditPack {
    pub(super) name: String,
    #[serde(default)]
    pub(super) srd_id: Option<String>,
}
impl BanditPack {
    fn is_absent(&self) -> bool {
        self == &Self::default()
    }
}

/// Player-facing label of each of the ten journey actions (JSON keys are kebab-case).
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Default)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(super) struct OfferLabels {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) create_character: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) begin_story: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) ask_courier: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) escort_courier: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) defend_courier: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) greatsword_attack: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) second_wind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) end_turn: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) short_rest: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) choose_harbor_scene: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum SceneKey {
    Room,
    Opening,
    DialogueSeal,
    DialogueEscort,
    Combat,
    Inn,
    Victory,
    Defeat,
}

impl SceneKey {
    pub(super) const ALL: [Self; 8] = [
        Self::Room,
        Self::Opening,
        Self::DialogueSeal,
        Self::DialogueEscort,
        Self::Combat,
        Self::Inn,
        Self::Victory,
        Self::Defeat,
    ];

    /// The strict, audience-safe source label carried with a scene view.
    pub(super) const fn source_label(self) -> &'static str {
        match self {
            Self::Room => "room",
            Self::Opening => "opening",
            Self::DialogueSeal => "dialogue-seal",
            Self::DialogueEscort => "dialogue-escort",
            Self::Combat => "combat",
            Self::Inn => "inn",
            Self::Victory => "victory",
            Self::Defeat => "defeat",
        }
    }

    /// Parses only the exact source labels emitted by [`Self::source_label`].
    pub(super) fn parse_source_label(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|key| key.source_label() == value)
    }

    const fn needs_rested_narration(self) -> bool {
        matches!(self, Self::Victory | Self::Defeat)
    }
}

/// Exact authored BeatScript scene key, kept separate from the broad scene category.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SceneVariantId(String);

impl SceneVariantId {
    /// Parses only the lowercase kebab-case identifiers accepted for authored scene variants.
    pub(super) fn parse(value: &str) -> Option<Self> {
        let well_formed = !value.is_empty()
            && value.len() <= ID_CHARACTERS_MAX
            && value.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            });
        well_formed.then(|| Self(value.to_owned()))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum FindingSkill {
    Perception,
    Investigation,
    Insight,
    Survival,
    Medicine,
    Arcana,
    History,
    Nature,
    Religion,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Finding {
    pub(super) id: String,
    pub(super) sentence: String,
    pub(super) skill: FindingSkill,
    pub(super) dc: u8,
    pub(super) private: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) consequence: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Scene {
    /// Runtime projection of the exact BeatScript map key; this is not serialized into pack data.
    #[serde(skip)]
    pub(super) source_variant_id: Option<String>,
    pub(super) key: SceneKey,
    pub(super) title: String,
    pub(super) narration: String,
    /// The rules wording of a scene (positions, outcomes, rest), kept apart from the story text
    /// so a story writer never touches it. Display joins it after the narration.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) rules_note: String,
    /// The characters on stage besides the heroes. Sprite and portrait are asset stems the
    /// client may not have; it falls back, so unknown ones are not an error here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) cast: Vec<SceneCastMember>,
    /// Narration once the Short Rest has completed; present for victory and defeat only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) narration_rested: Option<String>,
    pub(super) speaker: String,
    pub(super) dialogue: String,
    /// Served asset path without a leading slash.
    pub(super) asset: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) setting: Option<super::adventure::setting::SceneSetting>,
    #[serde(default)]
    pub(super) speaker_portrait: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) findings: Vec<Finding>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RestedStoryError;

/// Split authored Short Rest rules prose from the story before projecting a rested scene.
pub(super) fn rested_story_and_rules(scene: &Scene) -> Result<(String, String), RestedStoryError> {
    const HOUR_RULES: &str = "An uninterrupted hour has passed without Hit Point Dice or healing.";
    const BENEFIT_RULES: [&str; 3] = [
        "Eligible heroes recover one Second Wind use; dead or stable 0-HP heroes are not automatically revived.",
        "Knocked-out creatures are awake with 1 HP, still Prone; their held weapons remain dropped.",
        "They awaken with 1 HP, still Prone, and their held weapons remain dropped.",
    ];
    // SRD 5.2.1 pp. 18, 47, 187: stable recovery, Second Wind, and optional Hit Point Dice.
    const RULES: &str = "During the Short Rest each eligible hero chooses whether to spend Hit Point Dice to heal. Eligible heroes regain one use of Second Wind. A stable creature that isn't healed regains 1 Hit Point after 1d4 hours; a dead creature remains dead.";
    let narration = scene.narration_rested.as_deref().ok_or(RestedStoryError)?;
    if narration.trim().is_empty() {
        return Err(RestedStoryError);
    }
    if !narration.contains(HOUR_RULES)
        && !BENEFIT_RULES
            .iter()
            .any(|benefit| narration.contains(benefit))
    {
        return Ok((narration.to_owned(), RULES.to_owned()));
    }
    // Stored scenes retain their exact bytes; replace their old mechanical sentence in the view.
    let mut benefits = BENEFIT_RULES
        .into_iter()
        .filter(|benefit| narration.contains(benefit));
    let benefit_rules = benefits.next().ok_or(RestedStoryError)?;
    if narration.matches(HOUR_RULES).count() != 1
        || narration.matches(benefit_rules).count() != 1
        || benefits.next().is_some()
    {
        return Err(RestedStoryError);
    }
    let start = narration.find(HOUR_RULES).ok_or(RestedStoryError)?;
    let after_hour = start + HOUR_RULES.len();
    let benefits = narration[after_hour..]
        .find(benefit_rules)
        .map(|offset| after_hour + offset)
        .ok_or(RestedStoryError)?;
    let end = benefits + benefit_rules.len();
    let story = format!(
        "{}The party spends an hour catching its breath.{}",
        &narration[..start],
        &narration[end..]
    );
    Ok((story, RULES.to_owned()))
}

// ---------------------------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------------------------

/// A pack and tuning that parsed, passed every bound and cross-reference, and the revisions of
/// the exact bytes they were parsed from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LoadedPack {
    pub(super) pack: PackFile,
    pub(super) tuning: TuningFile,
    pub(super) pack_revision: Revision,
    pub(super) tuning_revision: Revision,
    /// JSON key paths of "standard" values that differ from the compiled SRD 5.2.1 baseline, in
    /// a fixed order. Computed by the loader; the files cannot declare themselves standard.
    pub(super) departures: Vec<String>,
    /// Original base assets remain available for authored receipts after presentation substitution.
    pub(super) source_scene_assets: Vec<(SceneKey, String)>,
    /// Native startup substitutes unavailable script paintings only in view projection.
    pub(super) script_scene_assets: BTreeMap<String, String>,
}

/// Load `pack.json` and `tuning.json` from `directory`.
///
/// Each file is read once under [`MAX_FILE_BYTES`]; the revision is the SHA-256 of those bytes
/// and the same bytes are parsed. There is no fallback: a missing directory or file is
/// [`PackError::Missing`]. This does blocking file I/O and is intended for server startup.
pub(super) fn load(directory: &Path) -> Result<LoadedPack, PackError> {
    let pack_bytes = read_file(SourceFile::Pack, &directory.join(PACK_FILE_NAME))?;
    let tuning_bytes = read_file(SourceFile::Tuning, &directory.join(TUNING_FILE_NAME))?;
    load_from_bytes(&pack_bytes, &tuning_bytes)
}

/// Validate and load the two documents from memory. Same checks as [`load`], used by tests and
/// by callers that already hold the bytes.
pub(super) fn load_from_bytes(
    pack_bytes: &[u8],
    tuning_bytes: &[u8],
) -> Result<LoadedPack, PackError> {
    super::beat_script::preflight(pack_bytes).map_err(PackError::Script)?;
    let mut pack: PackFile = parse(SourceFile::Pack, pack_bytes)?;
    require_schema(SourceFile::Pack, &pack.schema, PACK_SCHEMA)?;
    migrate_scene_inventory_models(&mut pack)?;
    migrate_retained_harbor_cast(&mut pack, Revision::of(pack_bytes));
    let tuning: TuningFile = parse(SourceFile::Tuning, tuning_bytes)?;
    require_schema(SourceFile::Tuning, &tuning.schema, TUNING_SCHEMA)?;
    validate_pack(&pack)?;
    pack.beat_script
        .as_ref()
        .unwrap_or(super::beat_script::canonical())
        .validate(&pack)
        .map_err(PackError::Script)?;
    validate_tuning(&tuning, pack.is_freeplay())?;
    validate_cross_references(&pack, &tuning)?;
    if let Some(script) = &mut pack.beat_script {
        for (id, scene) in &mut script.scenes {
            scene.source_variant_id = Some(id.clone());
        }
    }
    let mut departures = departures(&pack, &tuning);
    // A scene-assault-only pack stays on the standard rules profile until a fight happens; the
    // fight's own disclosure carries the tactical departures (see `journey::scene_fight`).
    if tuning
        .tactical
        .as_ref()
        .is_some_and(|tactical| tactical.scene_assault.is_none())
    {
        departures.extend(
            df_rules::tactical::DEPARTURES
                .iter()
                .map(|departure| (*departure).to_owned()),
        );
    }
    let source_scene_assets = pack
        .scenes
        .iter()
        .map(|scene| (scene.key, scene.asset.clone()))
        .collect();
    Ok(LoadedPack {
        source_scene_assets,
        script_scene_assets: BTreeMap::new(),
        pack,
        tuning,
        pack_revision: Revision::of(pack_bytes),
        tuning_revision: Revision::of(tuning_bytes),
        departures,
    })
}

/// Fill the ability-array template for one array. `raw_scores` are the array's scores before the
/// background bonus; `{0}` to `{5}` receive the scores after the Strength and Constitution bonus.
pub(super) fn render_array_description(
    template: &str,
    raw_scores: [u8; 6],
    bonus: BackgroundBonus,
) -> String {
    let mut scores = raw_scores.map(u16::from);
    scores[0] += u16::from(bonus.strength);
    scores[2] += u16::from(bonus.constitution);
    let mut text = template.to_owned();
    for (index, score) in scores.iter().enumerate() {
        text = text.replace(&format!("{{{index}}}"), &score.to_string());
    }
    text.replace("{bonus_str}", &bonus.strength.to_string())
        .replace("{bonus_con}", &bonus.constitution.to_string())
}

fn read_file(file: SourceFile, path: &Path) -> Result<Vec<u8>, PackError> {
    let classify = |error: io::Error| match error.kind() {
        io::ErrorKind::NotFound => PackError::Missing {
            file,
            path: path.to_owned(),
        },
        kind => PackError::Io {
            file,
            path: path.to_owned(),
            kind,
        },
    };
    let handle = File::open(path).map_err(classify)?;
    let declared = handle.metadata().map_err(classify)?.len();
    if declared > MAX_FILE_BYTES {
        return Err(PackError::TooLarge {
            file,
            bytes: declared,
            limit: MAX_FILE_BYTES,
        });
    }
    // The cap applies to the bytes actually read too, in case the file grew after the check.
    let mut bytes = Vec::new();
    handle
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(classify)?;
    Ok(bytes)
}

fn parse<T: DeserializeOwned>(file: SourceFile, bytes: &[u8]) -> Result<T, PackError> {
    let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if length > MAX_FILE_BYTES {
        return Err(PackError::TooLarge {
            file,
            bytes: length,
            limit: MAX_FILE_BYTES,
        });
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|error| parse_error(file, error.path().to_string(), error.inner()))?;
    deserializer
        .end()
        .map_err(|error| parse_error(file, ".".to_owned(), &error))?;
    Ok(value)
}

fn parse_error(file: SourceFile, path: String, error: &serde_json::Error) -> PackError {
    let kind = match error.classify() {
        serde_json::error::Category::Data => ParseErrorKind::Data,
        serde_json::error::Category::Eof => ParseErrorKind::UnexpectedEnd,
        serde_json::error::Category::Io | serde_json::error::Category::Syntax => {
            ParseErrorKind::Syntax
        }
    };
    super::refusal_detail::cause(
        "pack-parse",
        format!(
            "{kind:?} file={file:?} line={} column={}",
            error.line(),
            error.column()
        ),
    );
    PackError::Parse {
        file,
        path,
        line: error.line(),
        column: error.column(),
        kind,
        message: truncated(&error.to_string()),
    }
}

/// Parser messages can echo a value from the file; keep only a short prefix.
fn truncated(message: &str) -> String {
    const MESSAGE_CHARACTERS_MAX: usize = 160;
    let mut short: String = message.chars().take(MESSAGE_CHARACTERS_MAX).collect();
    if short.len() < message.len() {
        short.push('…');
    }
    short
}

fn require_schema(file: SourceFile, found: &str, expected: &'static str) -> Result<(), PackError> {
    if found == expected {
        return Ok(());
    }
    Err(PackError::Schema {
        file,
        found: truncated(found),
        expected,
    })
}

// ---------------------------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------------------------

fn range(file: SourceFile, path: &str, actual: i64, min: i64, max: i64) -> Result<(), PackError> {
    if (min..=max).contains(&actual) {
        return Ok(());
    }
    super::refusal_detail::cause(
        "pack-bounds",
        format!("OutOfRange file={file:?} actual={actual} min={min} max={max}"),
    );
    Err(PackError::Bounds {
        file,
        path: path.to_owned(),
        actual,
        min,
        max,
    })
}

fn one_of(
    file: SourceFile,
    path: &str,
    actual: u8,
    allowed: &'static [u8],
) -> Result<(), PackError> {
    if allowed.contains(&actual) {
        return Ok(());
    }
    super::refusal_detail::cause(
        "pack-value",
        format!("NotAllowed file={file:?} actual={actual}"),
    );
    Err(PackError::NotAllowed {
        file,
        path: path.to_owned(),
        actual: i64::from(actual),
        allowed,
    })
}

/// Length in characters, bounded by the file size cap, so the conversion cannot overflow.
fn length(value: usize) -> i64 {
    value as i64
}

fn text(
    file: SourceFile,
    path: &str,
    value: &str,
    min: usize,
    max: usize,
) -> Result<(), PackError> {
    range(
        file,
        path,
        length(value.chars().count()),
        length(min),
        length(max),
    )
}

fn malformed(file: SourceFile, path: &str, expected: &'static str) -> PackError {
    super::refusal_detail::cause(
        "pack-validation",
        format!("Malformed file={file:?} expected={expected}"),
    );
    PackError::Malformed {
        file,
        path: path.to_owned(),
        expected,
    }
}

fn cross_reference(file: SourceFile, path: &str, reason: CrossRefReason) -> PackError {
    super::refusal_detail::cause("pack-cross-reference", format!("{reason:?} file={file:?}"));
    PackError::CrossRef {
        file,
        path: path.to_owned(),
        reason,
    }
}

/// Lowercase kebab-case identifier of 1 to 64 characters.
fn identifier(file: SourceFile, path: &str, value: &str) -> Result<(), PackError> {
    text(file, path, value, 1, ID_CHARACTERS_MAX)?;
    let well_formed = value.split('-').all(|part| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    });
    if well_formed {
        return Ok(());
    }
    Err(malformed(file, path, "a lowercase kebab-case identifier"))
}

/// Fail on the first repeated value, reporting the location of the repeat.
fn require_unique<'a>(
    file: SourceFile,
    path_of: impl Fn(usize) -> String,
    values: impl Iterator<Item = &'a str>,
) -> Result<(), PackError> {
    let mut seen: Vec<&str> = Vec::new();
    for (index, value) in values.enumerate() {
        if seen.contains(&value) {
            return Err(cross_reference(
                file,
                &path_of(index),
                CrossRefReason::DuplicateId,
            ));
        }
        seen.push(value);
    }
    Ok(())
}

/// True when every `{...}` in `text` names an allowed placeholder and no brace is stray.
fn placeholders_are_defined(text: &str, allowed: &[&str]) -> bool {
    let mut rest = text;
    while let Some(position) = rest.find(['{', '}']) {
        let Some(opened) = rest[position..].strip_prefix('{') else {
            return false;
        };
        let Some((name, tail)) = opened.split_once('}') else {
            return false;
        };
        if !allowed.contains(&name) {
            return false;
        }
        rest = tail;
    }
    true
}

// ---------------------------------------------------------------------------------------------
// tuning.json validation
// ---------------------------------------------------------------------------------------------

fn validate_tuning(tuning: &TuningFile, freeplay: bool) -> Result<(), PackError> {
    if let Some(tactical) = &tuning.tactical {
        let mut total = 0_u16;
        let scene_assault = tactical.scene_assault.as_ref();
        if let Some(scene_assault) = scene_assault
            && (!freeplay
                || !tactical.enemies.is_empty()
                || !super::journey::tactical::holds_scene(&scene_assault.battle_scene))
        {
            return Err(malformed(
                SourceFile::Tuning,
                "tactical.scene_assault",
                "a freeplay pack with no roster enemies and a battle scene the server holds",
            ));
        }
        if scene_assault.is_none() && (tactical.enemies.is_empty() || tactical.enemies.len() > 4) {
            return Err(malformed(
                SourceFile::Tuning,
                "tactical.enemies",
                "one to four source enemy groups",
            ));
        }
        for enemy in &tactical.enemies {
            if !(1..=4).contains(&enemy.count)
                || enemy.ammunition > 100
                || df_rules::tactical::CombatantSpec::creature(
                    3,
                    &enemy.srd_id,
                    df_rules::tactical::Cell(0, 0),
                    enemy.ammunition,
                )
                .is_err()
            {
                return Err(malformed(
                    SourceFile::Tuning,
                    "tactical.enemies",
                    "supported SRD source ids, count 1..4 and ammunition 0..100",
                ));
            }
            total += u16::from(enemy.count);
        }
        if total > 4 {
            return Err(malformed(
                SourceFile::Tuning,
                "tactical.enemies",
                "at most four enemies",
            ));
        }
        let baseline = df_rules::local_journey::SRD_521_BASELINE;
        let hero = &tuning.hero;
        if hero.armor_class != 17
            || hero.proficiency_bonus != 2
            || hero.base_hit_points != 11
            || hero.background_bonus.strength != 2
            || hero.background_bonus.constitution != 1
            || hero.second_wind.uses_max != 2
            || hero.second_wind.die_sides != 10
            || hero.second_wind.flat_bonus != 1
            || hero.second_wind.recharge_per_short_rest != 1
            || hero.weapon.item != "greatsword"
            || hero.weapon.damage_dice != 2
            || hero.weapon.damage_sides != 6
            // A scene-assault-only pack states no combat or rest section; its rounds are the
            // SRD baseline (see `combat()` and `rest()`).
            || (scene_assault.is_none()
                && (tuning.combat.round_ticks != 6
                    || tuning.combat.critical_threshold != 20
                    || tuning.rest.short_rest_seconds != 3600))
            || hero.ability_arrays.len() != baseline.creation.ability_arrays.len()
            || hero.ability_arrays.iter().any(|array| {
                !baseline
                    .creation
                    .ability_arrays
                    .iter()
                    .any(|source| source.id == array.id && source.scores == array.scores)
            })
        {
            return Err(malformed(
                SourceFile::Tuning,
                "tactical",
                "tactical fights require the supported SRD hero profiles and six-second rounds",
            ));
        }
    }
    const FILE: SourceFile = SourceFile::Tuning;
    identifier(FILE, "pack", &tuning.pack)?;
    validate_tempo(tuning.tempo)?;
    let hero = &tuning.hero;
    range(
        FILE,
        "hero.armor_class",
        i64::from(hero.armor_class),
        10,
        30,
    )?;
    range(
        FILE,
        "hero.proficiency_bonus",
        i64::from(hero.proficiency_bonus),
        2,
        6,
    )?;
    range(
        FILE,
        "hero.base_hit_points",
        i64::from(hero.base_hit_points),
        1,
        60,
    )?;
    range(
        FILE,
        "hero.background_bonus.strength",
        i64::from(hero.background_bonus.strength),
        0,
        4,
    )?;
    range(
        FILE,
        "hero.background_bonus.constitution",
        i64::from(hero.background_bonus.constitution),
        0,
        4,
    )?;
    range(
        FILE,
        "hero.ability_arrays",
        length(hero.ability_arrays.len()),
        1,
        8,
    )?;
    for (index, array) in hero.ability_arrays.iter().enumerate() {
        validate_ability_array(index, array, hero.background_bonus)?;
        range(
            FILE,
            &format!("hero.ability_arrays[{index}].scores[2]"),
            i64::from(hero.base_hit_points)
                + i64::from(df_rules::local_journey::ability_modifier(
                    array.scores[2] + hero.background_bonus.constitution,
                )),
            1,
            65,
        )?;
    }
    require_unique(
        FILE,
        |index| format!("hero.ability_arrays[{index}].id"),
        hero.ability_arrays.iter().map(|array| array.id.as_str()),
    )?;
    let wind = hero.second_wind;
    range(
        FILE,
        "hero.second_wind.uses_max",
        i64::from(wind.uses_max),
        0,
        5,
    )?;
    one_of(
        FILE,
        "hero.second_wind.die_sides",
        wind.die_sides,
        DIE_SIDES,
    )?;
    range(
        FILE,
        "hero.second_wind.flat_bonus",
        i64::from(wind.flat_bonus),
        0,
        10,
    )?;
    range(
        FILE,
        "hero.second_wind.recharge_per_short_rest",
        i64::from(wind.recharge_per_short_rest),
        0,
        i64::from(wind.uses_max),
    )?;
    validate_weapon(
        "hero.weapon",
        &hero.weapon.item,
        hero.weapon.damage_dice,
        hero.weapon.damage_sides,
    )?;

    if freeplay {
        if tuning.bandit != BanditTuning::default()
            || tuning.combat != CombatTuning::default()
            || tuning.rest != RestTuning::default()
            || tuning.narrative != NarrativeTuning::default()
            || tuning
                .tactical
                .as_ref()
                .is_some_and(|tactical| tactical.scene_assault.is_none())
        {
            return Err(malformed(
                FILE,
                "freeplay",
                "creation tuning without encounter or quest sections",
            ));
        }
        return Ok(());
    }
    let bandit = &tuning.bandit;
    range(
        FILE,
        "bandit.hit_points",
        i64::from(bandit.hit_points),
        1,
        200,
    )?;
    range(
        FILE,
        "bandit.armor_class",
        i64::from(bandit.armor_class),
        1,
        30,
    )?;
    let abilities = bandit.abilities;
    for (path, score) in [
        ("bandit.abilities.strength", abilities.strength),
        ("bandit.abilities.dexterity", abilities.dexterity),
        ("bandit.abilities.constitution", abilities.constitution),
        ("bandit.abilities.intelligence", abilities.intelligence),
        ("bandit.abilities.wisdom", abilities.wisdom),
        ("bandit.abilities.charisma", abilities.charisma),
    ] {
        range(FILE, path, i64::from(score), 1, ABILITY_SCORE_CAP)?;
    }
    range(
        FILE,
        "bandit.attack_bonus",
        i64::from(bandit.attack_bonus),
        -5,
        20,
    )?;
    range(
        FILE,
        "bandit.damage_modifier",
        i64::from(bandit.damage_modifier),
        0,
        10,
    )?;
    validate_weapon(
        "bandit.weapon",
        &bandit.weapon.item,
        bandit.weapon.damage_dice,
        bandit.weapon.damage_sides,
    )?;

    range(
        FILE,
        "combat.round_ticks",
        i64::from(tuning.combat.round_ticks),
        1,
        600,
    )?;
    range(
        FILE,
        "combat.critical_threshold",
        i64::from(tuning.combat.critical_threshold),
        15,
        20,
    )?;
    range(
        FILE,
        "combat.combat_log_entries",
        i64::from(tuning.combat.combat_log_entries),
        1,
        32,
    )?;
    range(
        FILE,
        "rest.short_rest_seconds",
        i64::from(tuning.rest.short_rest_seconds),
        60,
        86_400,
    )?;
    range(
        FILE,
        "narrative.intervention_budget",
        i64::from(tuning.narrative.intervention_budget),
        1,
        64,
    )
}

fn validate_ability_array(
    index: usize,
    array: &AbilityArray,
    bonus: BackgroundBonus,
) -> Result<(), PackError> {
    const FILE: SourceFile = SourceFile::Tuning;
    identifier(FILE, &format!("hero.ability_arrays[{index}].id"), &array.id)?;
    for (position, score) in array.scores.iter().enumerate() {
        let path = format!("hero.ability_arrays[{index}].scores[{position}]");
        range(FILE, &path, i64::from(*score), 3, 18)?;
        let bonus_applied = match position {
            0 => i64::from(bonus.strength),
            2 => i64::from(bonus.constitution),
            _ => 0,
        };
        range(
            FILE,
            &path,
            i64::from(*score) + bonus_applied,
            3,
            ABILITY_SCORE_CAP,
        )?;
    }
    Ok(())
}

fn validate_weapon(
    prefix: &str,
    item: &str,
    damage_dice: u8,
    damage_sides: u8,
) -> Result<(), PackError> {
    const FILE: SourceFile = SourceFile::Tuning;
    if prefix == "bandit.weapon" {
        text(FILE, &format!("{prefix}.label"), item, 1, 32)?;
    } else if !COMPILED_WEAPONS.contains(&item) {
        return Err(cross_reference(
            FILE,
            &format!("{prefix}.item"),
            CrossRefReason::UnknownItem,
        ));
    }
    range(
        FILE,
        &format!("{prefix}.damage_dice"),
        i64::from(damage_dice),
        1,
        2,
    )?;
    one_of(
        FILE,
        &format!("{prefix}.damage_sides"),
        damage_sides,
        DIE_SIDES,
    )
}

// ---------------------------------------------------------------------------------------------
// pack.json validation
// ---------------------------------------------------------------------------------------------

fn validate_pack(pack: &PackFile) -> Result<(), PackError> {
    if let Some(campaign) = &pack.campaign {
        if super::adventure::campaign_store::validate_id(&campaign.id).is_err() {
            return Err(malformed(
                SourceFile::Pack,
                "campaign.id",
                "a bounded campaign id",
            ));
        }
        range(
            SourceFile::Pack,
            "campaign.chapter",
            i64::from(campaign.chapter),
            1,
            10,
        )?;
        if campaign.frame_sha256.len() != 64
            || !campaign.frame_sha256.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err(malformed(
                SourceFile::Pack,
                "campaign.frame_sha256",
                "64 hexadecimal characters",
            ));
        }
    }
    if let Some(epilogue) = &pack.campaign_epilogue {
        for (key, value) in [
            ("asked", &epilogue.asked),
            ("escorted", &epilogue.escorted),
            ("defeat", &epilogue.defeat),
        ] {
            text(
                SourceFile::Pack,
                &format!("campaign_epilogue.{key}"),
                value,
                1,
                220,
            )?;
        }
    }

    const FILE: SourceFile = SourceFile::Pack;
    identifier(FILE, "id", &pack.id)?;
    if pack.locale != "en" {
        return Err(malformed(
            FILE,
            "locale",
            "\"en\" (the only supported locale)",
        ));
    }
    text(FILE, "room_code", &pack.room_code, 1, 32)?;
    if !pack
        .room_code
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(malformed(
            FILE,
            "room_code",
            "ASCII letters and digits only",
        ));
    }
    validate_creation(&pack.creation)?;
    validate_equipment(&pack.equipment)?;
    if !pack.is_freeplay() {
        text(FILE, "bandit.name", &pack.bandit.name, 1, 48)?;
        if let Some(id) = &pack.bandit.srd_id {
            if !df_rules::SRD_521_CREATURES.iter().any(|c| c.id == id) {
                return Err(malformed(
                    FILE,
                    "antagonist.srd_id",
                    "known SRD 5.2.1 creature id",
                ));
            }
            text(FILE, "antagonist.name", &pack.bandit.name, 1, 28)?;
        }
        if let Some(label) = &pack.destination_label {
            text(FILE, "destination_label", label, 1, 40)?;
        }
    }
    validate_offer_labels(&pack.offer_labels, pack.is_freeplay())?;
    if !pack.is_freeplay() {
        range(
            FILE,
            "courier_clue",
            length(pack.courier_clue.len()),
            1,
            512,
        )?;
        text(
            FILE,
            "escort_support_suffix",
            &pack.escort_support_suffix,
            1,
            256,
        )?;
        let name_placeholders = pack.escort_support_suffix.matches("{name}").count();
        if name_placeholders != 1
            || !placeholders_are_defined(&pack.escort_support_suffix, &["name"])
        {
            return Err(malformed(
                FILE,
                "escort_support_suffix",
                "text with exactly one {name} placeholder and no other braces",
            ));
        }
    }
    validate_scenes(&pack.scenes, pack.is_freeplay())?;
    validate_speaking_notes(pack)?;
    validate_music(pack)
}

fn validate_music(pack: &PackFile) -> Result<(), PackError> {
    const FILE: SourceFile = SourceFile::Pack;
    range(FILE, "music", length(pack.music.len()), 0, 64)?;
    for (index, block) in pack.music.iter().enumerate() {
        text(
            FILE,
            &format!("music[{index}].place_id"),
            &block.place_id,
            1,
            96,
        )?;
        block.spec().map_err(|_| {
            malformed(
                FILE,
                &format!("music[{index}]"),
                "closed music labels and bounded free text",
            )
        })?;
    }
    require_unique(
        FILE,
        |index| format!("music[{index}].place_id"),
        pack.music.iter().map(|block| block.place_id.as_str()),
    )
}

fn validate_speaking_notes(pack: &PackFile) -> Result<(), PackError> {
    const FILE: SourceFile = SourceFile::Pack;
    range(FILE, "secret_terms", length(pack.secret_terms.len()), 0, 16)?;
    for (index, term) in pack.secret_terms.iter().enumerate() {
        text(FILE, &format!("secret_terms[{index}]"), term, 1, 64)?;
    }
    range(FILE, "speakers", length(pack.speakers.len()), 0, 8)?;
    for (index, speaker) in pack.speakers.iter().enumerate() {
        text(
            FILE,
            &format!("speakers[{index}].name"),
            &speaker.name,
            1,
            48,
        )?;
        text(
            FILE,
            &format!("speakers[{index}].role"),
            &speaker.role,
            1,
            96,
        )?;
        text(
            FILE,
            &format!("speakers[{index}].voice"),
            &speaker.voice,
            1,
            96,
        )?;
        if let Some(reply) = &speaker.identity_reply {
            text(
                FILE,
                &format!("speakers[{index}].identity_reply"),
                reply,
                1,
                120,
            )?;
        }
    }
    require_unique(
        FILE,
        |index| format!("speakers[{index}].name"),
        pack.speakers.iter().map(|speaker| speaker.name.as_str()),
    )?;
    Ok(())
}

fn validate_creation(creation: &CreationPack) -> Result<(), PackError> {
    const FILE: SourceFile = SourceFile::Pack;
    text(
        FILE,
        "creation.description",
        &creation.description,
        1,
        TEXT_CHARACTERS_MAX,
    )?;
    for (index, group) in creation.groups.iter().enumerate() {
        validate_group(index, group)?;
    }
    for required in FIXED_GROUPS
        .iter()
        .map(|(id, _)| *id)
        .chain([ARRAY_GROUP, "species"])
    {
        let occurrences = creation
            .groups
            .iter()
            .filter(|group| group.id == required)
            .count();
        if occurrences == 0 {
            return Err(cross_reference(
                FILE,
                "creation.groups",
                CrossRefReason::MissingGroup,
            ));
        }
        if occurrences > 1 {
            let index = creation
                .groups
                .iter()
                .enumerate()
                .filter(|(_, group)| group.id == required)
                .nth(1)
                .map_or(0, |(index, _)| index);
            return Err(cross_reference(
                FILE,
                &format!("creation.groups[{index}].id"),
                CrossRefReason::DuplicateGroup,
            ));
        }
    }
    let pick_lists = &creation.pick_lists;
    for (path, values, accepted) in [
        (
            "creation.pick_lists.alignment",
            &pick_lists.alignment,
            df_rules::local_journey::ALIGNMENTS,
        ),
        (
            "creation.pick_lists.language",
            &pick_lists.language,
            df_rules::local_journey::STANDARD_LANGUAGES,
        ),
        (
            "creation.pick_lists.skill",
            &pick_lists.skill,
            df_rules::local_journey::FIGHTER_SKILLS,
        ),
        (
            "creation.pick_lists.gaming-set",
            &pick_lists.gaming_set,
            df_rules::local_journey::GAMING_SETS,
        ),
    ] {
        validate_pick_list(path, values, accepted)?;
    }
    Ok(())
}

fn validate_group(index: usize, group: &CreationGroup) -> Result<(), PackError> {
    const FILE: SourceFile = SourceFile::Pack;
    let at = format!("creation.groups[{index}]");
    identifier(FILE, &format!("{at}.id"), &group.id)?;
    text(
        FILE,
        &format!("{at}.label"),
        &group.label,
        1,
        LABEL_CHARACTERS_MAX,
    )?;
    let fixed_option = FIXED_GROUPS
        .iter()
        .find(|(id, _)| *id == group.id)
        .map(|(_, option)| *option);
    if fixed_option.is_none() && group.id != ARRAY_GROUP && group.id != "species" {
        return Err(cross_reference(
            FILE,
            &format!("{at}.id"),
            CrossRefReason::UnknownGroup,
        ));
    }
    match (&group.description_template, group.id == ARRAY_GROUP) {
        (Some(template), true) => {
            text(
                FILE,
                &format!("{at}.description_template"),
                template,
                1,
                TEXT_CHARACTERS_MAX,
            )?;
            if !placeholders_are_defined(template, ARRAY_TEMPLATE_PLACEHOLDERS) {
                return Err(malformed(
                    FILE,
                    &format!("{at}.description_template"),
                    "text using only {0} to {5}, {bonus_str} and {bonus_con} placeholders",
                ));
            }
        }
        (None, true) => {
            return Err(cross_reference(
                FILE,
                &format!("{at}.description_template"),
                CrossRefReason::TemplateMisplaced,
            ));
        }
        (Some(_), false) => {
            return Err(cross_reference(
                FILE,
                &format!("{at}.description_template"),
                CrossRefReason::TemplateMisplaced,
            ));
        }
        (None, false) => {}
    }
    range(
        FILE,
        &format!("{at}.options"),
        length(group.options.len()),
        1,
        16,
    )?;
    for (position, option) in group.options.iter().enumerate() {
        let option_at = format!("{at}.options[{position}]");
        identifier(FILE, &format!("{option_at}.id"), &option.id)?;
        text(
            FILE,
            &format!("{option_at}.label"),
            &option.label,
            1,
            LABEL_CHARACTERS_MAX,
        )?;
        if let Some(description) = &option.description {
            text(
                FILE,
                &format!("{option_at}.description"),
                description,
                1,
                TEXT_CHARACTERS_MAX,
            )?;
        }
        if fixed_option.is_some_and(|accepted| accepted != option.id)
            || (group.id == "species"
                && df_rules::local_journey::HeroSpecies::from_choice(&option.id).is_none())
        {
            return Err(cross_reference(
                FILE,
                &format!("{option_at}.id"),
                CrossRefReason::UnknownOption,
            ));
        }
    }
    require_unique(
        FILE,
        |position| format!("{at}.options[{position}].id"),
        group.options.iter().map(|option| option.id.as_str()),
    )
}

fn validate_pick_list(path: &str, values: &[String], accepted: &[&str]) -> Result<(), PackError> {
    const FILE: SourceFile = SourceFile::Pack;
    range(FILE, path, length(values.len()), 1, 32)?;
    for (index, value) in values.iter().enumerate() {
        let at = format!("{path}[{index}]");
        identifier(FILE, &at, value)?;
        if !accepted.contains(&value.as_str()) {
            return Err(cross_reference(FILE, &at, CrossRefReason::UnknownPickValue));
        }
    }
    require_unique(
        FILE,
        |index| format!("{path}[{index}]"),
        values.iter().map(String::as_str),
    )
}

fn validate_equipment(equipment: &[EquipmentRow]) -> Result<(), PackError> {
    const FILE: SourceFile = SourceFile::Pack;
    range(FILE, "equipment", length(equipment.len()), 1, 32)?;
    for (index, row) in equipment.iter().enumerate() {
        if !COMPILED_ITEMS.contains(&row.item.as_str()) {
            return Err(cross_reference(
                FILE,
                &format!("equipment[{index}].item"),
                CrossRefReason::UnknownItem,
            ));
        }
        range(
            FILE,
            &format!("equipment[{index}].quantity"),
            i64::from(row.quantity),
            1,
            99,
        )?;
    }
    require_unique(
        FILE,
        |index| format!("equipment[{index}].item"),
        equipment.iter().map(|row| row.item.as_str()),
    )
}

fn validate_offer_labels(labels: &OfferLabels, freeplay: bool) -> Result<(), PackError> {
    for (path, label) in [
        ("offer_labels.create-character", &labels.create_character),
        ("offer_labels.begin-story", &labels.begin_story),
        ("offer_labels.ask-courier", &labels.ask_courier),
        ("offer_labels.escort-courier", &labels.escort_courier),
        ("offer_labels.defend-courier", &labels.defend_courier),
        ("offer_labels.greatsword-attack", &labels.greatsword_attack),
        ("offer_labels.second-wind", &labels.second_wind),
        ("offer_labels.end-turn", &labels.end_turn),
        ("offer_labels.short-rest", &labels.short_rest),
        (
            "offer_labels.choose-harbor-scene",
            &labels.choose_harbor_scene,
        ),
    ] {
        let entry_label = matches!(
            path,
            "offer_labels.create-character" | "offer_labels.begin-story"
        );
        text(
            SourceFile::Pack,
            path,
            label,
            usize::from(!freeplay || entry_label),
            if freeplay && !entry_label {
                0
            } else {
                LABEL_CHARACTERS_MAX
            },
        )?;
    }
    Ok(())
}

/// A scene's art: a served legacy scene asset, or generated art under the art contract (a
/// reviewed photograph's served path or a bare role token). A painted depiction stem never
/// validates, whatever the manifest says.
fn served_asset(path: &str) -> bool {
    SERVED_SCENE_ASSETS.contains(&path)
        || (super::adventure::generated_asset(path)
            && art_contract::check_content_reference(path).is_ok())
}

/// A cast member's sprite or portrait: empty, a reviewed photograph's stem or path, a role token
/// or a live path. The art contract refuses painted depiction stems and identity keys here.
fn cast_art(path: &str) -> bool {
    art_contract::check_content_reference(path).is_ok()
}

fn validate_scenes(scenes: &[Scene], freeplay: bool) -> Result<(), PackError> {
    const FILE: SourceFile = SourceFile::Pack;
    for (index, scene) in scenes.iter().enumerate() {
        let at = format!("scenes[{index}]");
        text(
            FILE,
            &format!("{at}.title"),
            &scene.title,
            1,
            TEXT_CHARACTERS_MAX,
        )?;
        text(
            FILE,
            &format!("{at}.narration"),
            &scene.narration,
            1,
            TEXT_CHARACTERS_MAX,
        )?;
        text(
            FILE,
            &format!("{at}.speaker"),
            &scene.speaker,
            0,
            TEXT_CHARACTERS_MAX,
        )?;
        text(
            FILE,
            &format!("{at}.dialogue"),
            &scene.dialogue,
            0,
            TEXT_CHARACTERS_MAX,
        )?;
        text(
            FILE,
            &format!("{at}.rules_note"),
            &scene.rules_note,
            0,
            TEXT_CHARACTERS_MAX,
        )?;
        if !scene.findings.is_empty() {
            range(
                FILE,
                &format!("{at}.findings"),
                length(scene.findings.len()),
                2,
                3,
            )?;
        }
        let mut finding_ids = BTreeSet::new();
        for (finding_index, finding) in scene.findings.iter().enumerate() {
            let finding_at = format!("{at}.findings[{finding_index}]");
            identifier(FILE, &format!("{finding_at}.id"), &finding.id)?;
            if !finding_ids.insert(finding.id.as_str()) {
                return Err(cross_reference(
                    FILE,
                    &format!("{finding_at}.id"),
                    CrossRefReason::DuplicateId,
                ));
            }
            text(
                FILE,
                &format!("{finding_at}.sentence"),
                &finding.sentence,
                1,
                TEXT_CHARACTERS_MAX,
            )?;
            if !matches!(finding.dc, 5 | 10 | 15 | 20 | 25 | 30) {
                return Err(PackError::Malformed {
                    file: FILE,
                    path: format!("{finding_at}.dc"),
                    expected: "typical SRD ability check DC (5, 10, 15, 20, 25, or 30)",
                });
            }
            if let Some(consequence) = &finding.consequence {
                identifier(FILE, &format!("{finding_at}.consequence"), consequence)?;
            }
        }
        range(FILE, &format!("{at}.cast"), length(scene.cast.len()), 0, 6)?;
        for (position, actor) in scene.cast.iter().enumerate() {
            let member = format!("{at}.cast[{position}]");
            text(FILE, &format!("{member}.id"), &actor.id, 1, 64)?;
            text(FILE, &format!("{member}.name"), &actor.name, 1, 48)?;
            text(FILE, &format!("{member}.sprite"), &actor.sprite, 0, 128)?;
            text(FILE, &format!("{member}.portrait"), &actor.portrait, 0, 128)?;
            for (field, value) in [("sprite", &actor.sprite), ("portrait", &actor.portrait)] {
                if !cast_art(value) {
                    return Err(PackError::Malformed {
                        file: FILE,
                        path: format!("{member}.{field}"),
                        expected: "a reviewed -documentary-photo stem or an art-contract role token, never painted art",
                    });
                }
            }
            text(FILE, &format!("{member}.voice"), &actor.voice, 0, 96)?;
            if let Some(stat_block) = &actor.stat_block
                && !scene_stat_block_known(stat_block)
            {
                return Err(PackError::Malformed {
                    file: FILE,
                    path: format!("{member}.stat_block"),
                    expected: "an SRD 5.2.1 creature id compiled into df-rules",
                });
            }
        }
        if let Some(rested) = &scene.narration_rested {
            text(
                FILE,
                &format!("{at}.narration_rested"),
                rested,
                1,
                TEXT_CHARACTERS_MAX,
            )?;
        }
        if scene.narration_rested.is_some() != scene.key.needs_rested_narration() {
            return Err(cross_reference(
                FILE,
                &format!("{at}.narration_rested"),
                CrossRefReason::RestedNarrationMismatch,
            ));
        }
        if let Some(setting) = &scene.setting {
            identifier(FILE, &format!("{at}.setting.place_id"), &setting.place_id)?;
            identifier(FILE, &format!("{at}.setting.art_key"), &setting.art_key)?;
            text(
                FILE,
                &format!("{at}.setting.name"),
                &setting.name,
                1,
                LABEL_CHARACTERS_MAX,
            )?;
            text(
                FILE,
                &format!("{at}.setting.short"),
                &setting.short,
                1,
                LABEL_CHARACTERS_MAX,
            )?;
        }
        if scene
            .setting
            .as_ref()
            .is_some_and(|setting| !super::adventure::setting::validates(setting, &scene.asset))
        {
            return Err(PackError::Malformed {
                file: FILE,
                path: format!("{at}.setting"),
                expected: "setting with art depicting its kind",
            });
        }
        if !served_asset(&scene.asset) {
            return Err(cross_reference(
                FILE,
                &format!("{at}.asset"),
                CrossRefReason::AssetNotServed,
            ));
        }
        if !scene.speaker_portrait.is_empty() && !served_asset(&scene.speaker_portrait) {
            return Err(cross_reference(
                FILE,
                &format!("{at}.speaker_portrait"),
                CrossRefReason::AssetNotServed,
            ));
        }
    }
    if scenes.iter().any(|s| s.setting.is_some()) && scenes.iter().any(|s| s.setting.is_none()) {
        return Err(PackError::Malformed {
            file: FILE,
            path: "scenes.setting".into(),
            expected: "all scene settings, or legacy with no settings",
        });
    }
    let required = if freeplay {
        &[SceneKey::Room, SceneKey::Opening][..]
    } else {
        &SceneKey::ALL[..]
    };
    if freeplay && scenes.len() != required.len() {
        return Err(malformed(
            FILE,
            "scenes",
            "only room and opening scenes for freeplay",
        ));
    }
    for key in required.iter().copied() {
        let mut matches = scenes
            .iter()
            .enumerate()
            .filter(|(_, scene)| scene.key == key);
        if matches.next().is_none() {
            return Err(cross_reference(
                FILE,
                "scenes",
                CrossRefReason::MissingScene,
            ));
        }
        if let Some((index, _)) = matches.next() {
            return Err(cross_reference(
                FILE,
                &format!("scenes[{index}].key"),
                CrossRefReason::DuplicateScene,
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Cross-file checks and baseline comparison
// ---------------------------------------------------------------------------------------------

pub(super) fn validate_script_scene(content: &PackFile, scene: &Scene) -> Result<(), PackError> {
    if [
        &scene.title,
        &scene.narration,
        &scene.speaker,
        &scene.dialogue,
    ]
    .iter()
    .any(|text| {
        text.chars().any(char::is_control)
            || (!content.courier_clue.is_empty() && text.contains(&content.courier_clue))
    }) {
        return Err(malformed(
            SourceFile::Pack,
            "beat_script.scenes",
            "bounded public plain text without private clue payload",
        ));
    }
    let mut scenes = content.scenes.clone();
    let destination = scenes
        .iter_mut()
        .find(|s| s.key == scene.key)
        .ok_or_else(|| {
            cross_reference(
                SourceFile::Pack,
                "beat_script.scenes",
                CrossRefReason::MissingScene,
            )
        })?;
    *destination = scene.clone();
    if let Some(model) = destination
        .setting
        .as_mut()
        .and_then(|setting| setting.model.as_mut())
    {
        migrate_scene_inventory_model(model, "beat_script.scenes.setting.model")?;
    }
    validate_scenes(&scenes, content.is_freeplay())
}

/// The retained harbor pack that 62105b7a's stored checkpoints pin by hash still names the cast
/// art 613839eb retired. Only that exact source is mapped to the current photographic cutouts,
/// so its checkpoints recover with their pins intact; every other pack still meets the art
/// contract unchanged.
fn migrate_retained_harbor_cast(pack: &mut PackFile, revision: Revision) {
    if revision.to_hex() != "5c9e2ad1344484c5fcdadbaa18a72db23495c392297312580c6aae286554f3d0" {
        return;
    }
    for scene in &mut pack.scenes {
        for actor in &mut scene.cast {
            for reference in [&mut actor.sprite, &mut actor.portrait] {
                let replacement = match reference.as_str() {
                    "assets/ui/generated/sprite-courier"
                    | "assets/ui/generated/portrait-courier" => {
                        Some("assets/ui/generated/sprite-courier-documentary-photo")
                    }
                    "assets/ui/generated/sprite-bandit" | "assets/ui/generated/portrait-bandit" => {
                        Some("assets/ui/generated/sprite-bandit-documentary-photo")
                    }
                    _ => None,
                };
                if let Some(replacement) = replacement {
                    *reference = replacement.to_owned();
                }
            }
        }
    }
}

fn migrate_scene_inventory_models(pack: &mut PackFile) -> Result<(), PackError> {
    for (index, scene) in pack.scenes.iter_mut().enumerate() {
        if let Some(model) = scene
            .setting
            .as_mut()
            .and_then(|setting| setting.model.as_mut())
        {
            migrate_scene_inventory_model(model, &format!("scenes[{index}].setting.model"))?;
        }
    }
    if let Some(script) = &mut pack.beat_script {
        for (key, scene) in &mut script.scenes {
            if let Some(model) = scene
                .setting
                .as_mut()
                .and_then(|setting| setting.model.as_mut())
            {
                migrate_scene_inventory_model(
                    model,
                    &format!("beat_script.scenes.{key}.setting.model"),
                )?;
            }
        }
    }
    Ok(())
}

fn migrate_scene_inventory_model(
    model: &mut super::adventure::scene_model::SceneModel,
    path: &str,
) -> Result<(), PackError> {
    model
        .migrate_inventory_version()
        .map_err(|_| PackError::Malformed {
            file: SourceFile::Pack,
            path: path.to_owned(),
            expected: "supported versioned location inventory model",
        })
}

fn validate_cross_references(pack: &PackFile, tuning: &TuningFile) -> Result<(), PackError> {
    if !pack
        .equipment
        .iter()
        .any(|row| row.item == tuning.hero.weapon.item)
    {
        return Err(cross_reference(
            SourceFile::Tuning,
            "hero.weapon.item",
            CrossRefReason::WeaponNotEquipped,
        ));
    }
    if tuning.pack != pack.id {
        return Err(cross_reference(
            SourceFile::Tuning,
            "pack",
            CrossRefReason::PackIdMismatch,
        ));
    }
    let Some((group_index, group)) = pack
        .creation
        .groups
        .iter()
        .enumerate()
        .find(|(_, group)| group.id == ARRAY_GROUP)
    else {
        return Err(cross_reference(
            SourceFile::Pack,
            "creation.groups",
            CrossRefReason::MissingGroup,
        ));
    };
    for (index, array) in tuning.hero.ability_arrays.iter().enumerate() {
        if !group.options.iter().any(|option| option.id == array.id) {
            return Err(cross_reference(
                SourceFile::Tuning,
                &format!("hero.ability_arrays[{index}].id"),
                CrossRefReason::ArrayWithoutOption,
            ));
        }
    }
    for (index, option) in group.options.iter().enumerate() {
        if !tuning
            .hero
            .ability_arrays
            .iter()
            .any(|array| array.id == option.id)
        {
            return Err(cross_reference(
                SourceFile::Pack,
                &format!("creation.groups[{group_index}].options[{index}].id"),
                CrossRefReason::OptionWithoutArray,
            ));
        }
    }
    // A scene-assault pack with nobody to fight would offer nothing: refuse it at load.
    if tuning
        .tactical
        .as_ref()
        .is_some_and(|tactical| tactical.scene_assault.is_some())
        && !pack
            .scenes
            .iter()
            .any(|scene| scene.cast.iter().any(|actor| actor.stat_block.is_some()))
    {
        return Err(malformed(
            SourceFile::Tuning,
            "tactical.scene_assault",
            "at least one scene cast member with a stat_block",
        ));
    }
    Ok(())
}

fn same_members(actual: &[String], baseline: &[&str]) -> bool {
    actual.len() == baseline.len() && baseline.iter().all(|item| actual.iter().any(|a| a == item))
}

/// Standard-sensitive keys compared with the single compiled rules baseline.
fn departures(pack: &PackFile, tuning: &TuningFile) -> Vec<String> {
    let baseline = df_rules::local_journey::SRD_521_BASELINE;
    let creature = pack
        .bandit
        .srd_id
        .as_ref()
        .and_then(|id| df_rules::SRD_521_CREATURES.iter().find(|c| c.id == id));
    // Absence is the explicitly supported legacy harbor Bandit, not an unknown creature.
    let expected = creature.map_or(baseline.bandit, |c| c.rules);
    let hero = &tuning.hero;
    let bandit = &tuning.bandit;
    let arrays_match = hero.ability_arrays.len() == baseline.creation.ability_arrays.len()
        && baseline.creation.ability_arrays.iter().all(|expected| {
            hero.ability_arrays
                .iter()
                .any(|array| array.id == expected.id && array.scores == expected.scores)
        });
    let equipment_matches = pack.equipment.len() == baseline.equipment.len()
        && baseline.equipment.iter().all(|(item, quantity)| {
            pack.equipment
                .iter()
                .any(|row| row.item == *item && row.quantity == *quantity)
        });
    let pick_lists = &pack.creation.pick_lists;
    let checks = [
        (
            hero.armor_class != baseline.creation.armor_class,
            "hero.armor_class",
        ),
        (
            hero.proficiency_bonus != baseline.creation.proficiency_bonus,
            "hero.proficiency_bonus",
        ),
        (
            hero.base_hit_points
                != baseline.creation.base_hit_points
                    + df_rules::local_journey::HeroSpecies::Dwarf
                        .combat_traits()
                        .hit_points_per_level,
            "hero.base_hit_points",
        ),
        (!arrays_match, "hero.ability_arrays"),
        (
            hero.background_bonus.strength != baseline.creation.background_bonus.strength,
            "hero.background_bonus.strength",
        ),
        (
            hero.background_bonus.constitution != baseline.creation.background_bonus.constitution,
            "hero.background_bonus.constitution",
        ),
        (
            hero.second_wind.uses_max != baseline.second_wind.uses_max,
            "hero.second_wind.uses_max",
        ),
        (
            hero.second_wind.die_sides != baseline.second_wind.die_sides,
            "hero.second_wind.die_sides",
        ),
        (
            hero.second_wind.flat_bonus != baseline.second_wind.flat_bonus,
            "hero.second_wind.flat_bonus",
        ),
        (
            hero.second_wind.recharge_per_short_rest
                != baseline.second_wind.recharge_per_short_rest,
            "hero.second_wind.recharge_per_short_rest",
        ),
        (
            hero.weapon.damage_dice != baseline.hero_weapon.damage_dice,
            "hero.weapon.damage_dice",
        ),
        (
            hero.weapon.damage_sides != baseline.hero_weapon.damage_sides,
            "hero.weapon.damage_sides",
        ),
        (
            i32::from(bandit.hit_points) != i32::from(expected.hit_points),
            "bandit.hit_points",
        ),
        (
            i32::from(bandit.armor_class) != i32::from(expected.armor_class),
            "bandit.armor_class",
        ),
        (
            bandit.abilities.strength != expected.abilities[0],
            "bandit.abilities.strength",
        ),
        (
            bandit.abilities.dexterity != expected.abilities[1],
            "bandit.abilities.dexterity",
        ),
        (
            bandit.abilities.constitution != expected.abilities[2],
            "bandit.abilities.constitution",
        ),
        (
            bandit.abilities.intelligence != expected.abilities[3],
            "bandit.abilities.intelligence",
        ),
        (
            bandit.abilities.wisdom != expected.abilities[4],
            "bandit.abilities.wisdom",
        ),
        (
            bandit.abilities.charisma != expected.abilities[5],
            "bandit.abilities.charisma",
        ),
        (
            i32::from(bandit.attack_bonus) != expected.attack_bonus,
            "bandit.attack_bonus",
        ),
        (
            i32::from(bandit.damage_modifier) != expected.damage_modifier,
            "bandit.damage_modifier",
        ),
        (
            bandit.weapon.damage_dice != expected.weapon.damage_dice,
            "bandit.weapon.damage_dice",
        ),
        (
            bandit.weapon.damage_sides != expected.weapon.damage_sides,
            "bandit.weapon.damage_sides",
        ),
        (
            tuning.combat.critical_threshold != baseline.combat.critical_threshold,
            "combat.critical_threshold",
        ),
        (
            tuning.combat.round_ticks != baseline.combat.round_ticks,
            "combat.round_ticks",
        ),
        (
            tuning.rest.short_rest_seconds != baseline.rest.short_rest_seconds,
            "rest.short_rest_seconds",
        ),
        (
            !same_members(&pick_lists.alignment, baseline.alignments),
            "creation.pick_lists.alignment",
        ),
        (
            !same_members(&pick_lists.language, baseline.standard_languages),
            "creation.pick_lists.language",
        ),
        (
            !same_members(&pick_lists.skill, baseline.fighter_skills),
            "creation.pick_lists.skill",
        ),
        (
            !same_members(&pick_lists.gaming_set, baseline.gaming_sets),
            "creation.pick_lists.gaming-set",
        ),
        (!equipment_matches, "equipment"),
        // Weapon items change the sheet's weapon, so they are reported with the dice.
        (hero.weapon.item != "greatsword", "hero.weapon.item"),
        (
            bandit.weapon.item != creature.map_or("scimitar", |c| c.weapon_label),
            "bandit.weapon.label",
        ),
    ];
    let mut departures: Vec<String> = checks
        .into_iter()
        .filter(|(differs, key)| {
            *differs
                && !(tuning.tactical.is_some() && key.starts_with("bandit."))
                && !(pack.is_freeplay()
                    && ["bandit.", "combat.", "rest."]
                        .iter()
                        .any(|prefix| key.starts_with(prefix)))
        })
        .map(|(_, key)| key.to_owned())
        .collect();
    if tuning.tactical.is_none()
        && let Some(creature) = creature
        && !creature.exact
    {
        departures.push(format!("antagonist.simplified:{}", creature.id));
    }
    departures
}

// Startup installs one immutable revision before constructing any game state. Qualification
// deliberately leaves this empty: all mechanics then use the compiled rules baseline.
static ACTIVE: std::sync::RwLock<Option<std::sync::Arc<LoadedPack>>> = std::sync::RwLock::new(None);

pub(super) fn install(loaded: LoadedPack) -> Result<(), io::Error> {
    super::journey::living_scene::clear();
    if loaded.pack.is_freeplay() {
        if let Some(place) = loaded
            .pack
            .scenes
            .iter()
            .find(|scene| scene.key == SceneKey::Opening)
            .and_then(|scene| scene.setting.clone())
        {
            super::adventure::scene_model::install_scenes(
                &super::adventure::setting::ChapterSettings {
                    meeting: place.clone(),
                    confrontation: place.clone(),
                    destination: place,
                },
            );
        }
    }
    let setting = |key| {
        loaded
            .pack
            .scenes
            .iter()
            .find(|scene| scene.key == key)
            .and_then(|scene| scene.setting.clone())
    };
    if let (Some(meeting), Some(confrontation), Some(destination)) = (
        setting(SceneKey::Opening),
        setting(SceneKey::Combat),
        setting(SceneKey::Inn),
    ) {
        super::adventure::scene_model::install_scenes(
            &super::adventure::setting::ChapterSettings {
                meeting,
                confrontation,
                destination,
            },
        );
    }
    *ACTIVE
        .write()
        .map_err(|_| io::Error::other("gameplay pack lock poisoned"))? =
        Some(std::sync::Arc::new(loaded));
    Ok(())
}

pub(super) fn active() -> Option<std::sync::Arc<LoadedPack>> {
    #[cfg(test)]
    if let Some(loaded) = TEST_ACTIVE.with(|slot| slot.borrow().clone()) {
        return Some(loaded);
    }
    // Writers only swap a fully constructed immutable Arc. Even a poisoned lock retains
    // that revision; poisoning must never select the qualification rules fallback.
    ACTIVE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

#[cfg(test)]
thread_local! { static TEST_ACTIVE: std::cell::RefCell<Option<std::sync::Arc<LoadedPack>>> = const { std::cell::RefCell::new(None) }; }

#[cfg(test)]
pub(super) fn with_test_pack<T>(loaded: LoadedPack, run: impl FnOnce() -> T) -> T {
    struct Reset(Option<std::sync::Arc<LoadedPack>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_ACTIVE.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _reset = Reset(TEST_ACTIVE.with(|slot| slot.replace(Some(std::sync::Arc::new(loaded)))));
    run()
}

pub(super) fn with_creation<T>(
    run: impl FnOnce(&df_rules::local_journey::CreationRules<'_>) -> T,
) -> T {
    use df_rules::local_journey as rules;
    let Some(loaded) = active() else {
        return run(&rules::SRD_521_BASELINE.creation);
    };
    let hero = &loaded.tuning.hero;
    let arrays = hero
        .ability_arrays
        .iter()
        .map(|array| rules::AbilityArray {
            id: &array.id,
            scores: array.scores,
        })
        .collect::<Vec<_>>();
    run(&rules::CreationRules {
        ability_arrays: &arrays,
        background_bonus: rules::BackgroundBonus {
            strength: hero.background_bonus.strength,
            constitution: hero.background_bonus.constitution,
        },
        // This pinned pack field historically includes Dwarven Toughness (SRD 5.2.1 p. 84).
        // The native rules now apply the selected species bonus separately. Preserve the
        // legacy pack's actual HP and custom delta while passing the class base to rules.
        base_hit_points: hero.base_hit_points
            - rules::HeroSpecies::Dwarf
                .combat_traits()
                .hit_points_per_level,
        armor_class: hero.armor_class,
        proficiency_bonus: hero.proficiency_bonus,
        second_wind_uses_max: hero.second_wind.uses_max,
    })
}

#[derive(Deserialize)]
struct ArcherPackage {
    creation: CreationPack,
    hero: ArcherHero,
    equipment: Vec<EquipmentRow>,
    #[serde(flatten)]
    _source_fields: BTreeMap<String, serde::de::IgnoredAny>,
}
fn archer_package() -> Result<ArcherPackage, df_rules::local_journey::JourneyRuleError> {
    serde_json::from_str(df_rules::tactical::ARCHER_CREATION_JSON)
        .map_err(|_| df_rules::local_journey::JourneyRuleError::InvalidRules)
}
/// The archer build's ability data, as far as the creation page shows it.
#[derive(serde::Deserialize)]
struct ArcherHero {
    ability_arrays: Vec<ArcherArray>,
    background_bonus: ArcherBonus,
}
#[derive(serde::Deserialize)]
struct ArcherArray {
    id: String,
    scores: [u8; 6],
}
#[derive(serde::Deserialize)]
struct ArcherBonus {
    dexterity: u8,
    constitution: u8,
}
pub(super) struct BuildPackage {
    pub(super) id: String,
    pub(super) label: String,
    /// Ability scores after the background bonus, in STR DEX CON INT WIS CHA order. Two
    /// packages can grant the same equipment and differ only here, so the page shows them.
    pub(super) abilities: [u8; 6],
    choices: Vec<(String, String)>,
}
impl BuildPackage {
    /// The scores as the creation page shows them beside the package.
    pub(super) fn abilities_text(&self) -> String {
        let [
            strength,
            dexterity,
            constitution,
            intelligence,
            wisdom,
            charisma,
        ] = self.abilities;
        format!(
            "STR {strength} · DEX {dexterity} · CON {constitution} · INT {intelligence} · WIS {wisdom} · CHA {charisma}"
        )
    }
}
/// A melee array's scores with the Soldier bonus the creation rules add to it.
fn melee_abilities(array: &str) -> Option<[u8; 6]> {
    with_creation(|rules| {
        let mut scores = rules
            .ability_arrays
            .iter()
            .find(|candidate| candidate.id == array)?
            .scores;
        scores[0] = scores[0].checked_add(rules.background_bonus.strength)?;
        scores[2] = scores[2].checked_add(rules.background_bonus.constitution)?;
        Some(scores)
    })
}
pub(super) fn build_packages()
-> Result<Vec<BuildPackage>, df_rules::local_journey::JourneyRuleError> {
    let mut packages = with_content(|pack| {
        pack.creation
            .groups
            .iter()
            .find(|group| group.id == "array")
            .and_then(|group| {
                group
                    .options
                    .iter()
                    .map(|array| {
                        Some(BuildPackage {
                            id: format!("melee-{}", array.id),
                            label: format!("{} · Greatsword and Defense", array.label),
                            abilities: melee_abilities(&array.id)?,
                            choices: vec![("array".to_owned(), array.id.clone())],
                        })
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or(df_rules::local_journey::JourneyRuleError::InvalidRules)
    })?;
    for package in &mut packages {
        for key in ["equipment", "style-masteries"] {
            let value = with_content(|pack| {
                pack.creation
                    .groups
                    .iter()
                    .find(|group| group.id == key)
                    .and_then(|group| group.options.first())
                    .map(|option| option.id.clone())
            })
            .ok_or(df_rules::local_journey::JourneyRuleError::InvalidRules)?;
            package.choices.push((key.to_owned(), value));
        }
    }
    let source = archer_package()?;
    let mut choices = Vec::new();
    for key in ["array", "equipment", "style-masteries"] {
        let group = source
            .creation
            .groups
            .iter()
            .find(|group| group.id == key)
            .ok_or(df_rules::local_journey::JourneyRuleError::InvalidRules)?;
        let [option] = group.options.as_slice() else {
            return Err(df_rules::local_journey::JourneyRuleError::InvalidRules);
        };
        choices.push((key.to_owned(), option.id.clone()));
    }
    let array = choices
        .first()
        .and_then(|(_, id)| {
            source
                .hero
                .ability_arrays
                .iter()
                .find(|candidate| &candidate.id == id)
        })
        .ok_or(df_rules::local_journey::JourneyRuleError::InvalidRules)?;
    let mut abilities = array.scores;
    let bonus = &source.hero.background_bonus;
    abilities[1] = abilities[1].saturating_add(bonus.dexterity);
    abilities[2] = abilities[2].saturating_add(bonus.constitution);
    packages.push(BuildPackage {
        id: "archer".to_owned(),
        label: "Longbow and Archery · Fighter B + Soldier B".to_owned(),
        abilities,
        choices,
    });
    Ok(packages)
}
pub(super) fn archer_equipment()
-> Result<Vec<EquipmentRow>, df_rules::local_journey::JourneyRuleError> {
    Ok(archer_package()?.equipment)
}
pub(super) fn validate_character(
    name: &str,
    choices: &[(&str, &str)],
) -> Result<df_rules::local_journey::LegalCharacter, df_rules::local_journey::JourneyRuleError> {
    validate_character_inner(name, choices).inspect_err(|error| {
        super::refusal_detail::cause("character-validation", format!("{error:?}"));
    })
}

fn validate_character_inner(
    name: &str,
    choices: &[(&str, &str)],
) -> Result<df_rules::local_journey::LegalCharacter, df_rules::local_journey::JourneyRuleError> {
    use df_rules::local_journey::JourneyRuleError as Error;
    let mut expanded = Vec::new();
    let mut package = None;
    let mut portrait = None;
    for &(group, value) in choices {
        if group == "build-package" {
            if package.replace(value).is_some() {
                super::refusal_detail::cause(
                    "character-catalog",
                    "DuplicateGroup group=build-package".to_owned(),
                );
                return Err(Error::InvalidChoice);
            }
        } else if group == "portrait" {
            if portrait.replace(value).is_some()
                || !["portrait-hero-1", "portrait-hero-2", "portrait-hero-3"].contains(&value)
            {
                super::refusal_detail::cause(
                    "character-catalog",
                    "InvalidGroup group=portrait".to_owned(),
                );
                return Err(Error::InvalidChoice);
            }
        } else {
            expanded.push((group.to_owned(), value.to_owned()));
        }
    }
    if let Some(package) = package {
        if !active().is_some_and(|loaded| loaded.tuning.tactical.is_some())
            || expanded
                .iter()
                .any(|(key, _)| ["array", "equipment", "style-masteries"].contains(&key.as_str()))
        {
            super::refusal_detail::cause(
                "character-catalog",
                "PackageConflict group=build-package".to_owned(),
            );
            return Err(Error::InvalidChoice);
        }
        let selected = build_packages()?
            .into_iter()
            .find(|candidate| candidate.id == package)
            .ok_or_else(|| {
                super::refusal_detail::cause(
                    "character-catalog",
                    "UnknownPackage group=build-package".to_owned(),
                );
                Error::InvalidChoice
            })?;
        expanded.extend(selected.choices);
    }
    let borrowed = expanded
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let archer = borrowed
        .iter()
        .any(|(key, value)| *key == "array" && *value == "dexterous");
    if archer {
        if package != Some("archer") {
            super::refusal_detail::cause(
                "character-catalog",
                "PackageRequired group=array".to_owned(),
            );
            return Err(Error::InvalidChoice);
        }
        return df_rules::tactical::validate_archer(name, &borrowed)
            .map(|build| build.character().clone())
            .map_err(|_| Error::InvalidChoice);
    }
    let allowed = with_content(|pack| {
        borrowed.iter().all(|(group, option)| {
            if df_rules::species::choice_option_is_known(group, option) {
                return true;
            }
            let list = match *group {
                "alignment" => Some(&pack.creation.pick_lists.alignment),
                "language-1" | "language-2" => Some(&pack.creation.pick_lists.language),
                "skill-1" | "skill-2" => Some(&pack.creation.pick_lists.skill),
                "gaming-set" => Some(&pack.creation.pick_lists.gaming_set),
                _ => None,
            };
            let permitted = match list {
                Some(list) => list.iter().any(|value| value == option),
                None => pack.creation.groups.iter().any(|candidate| {
                    candidate.id == *group
                        && candidate
                            .options
                            .iter()
                            .any(|candidate| candidate.id == *option)
                }),
            };
            if !permitted {
                let group_id = match *group {
                    "alignment" => "alignment",
                    "language-1" => "language-1",
                    "language-2" => "language-2",
                    "skill-1" => "skill-1",
                    "skill-2" => "skill-2",
                    "gaming-set" => "gaming-set",
                    _ => pack
                        .creation
                        .groups
                        .iter()
                        .find(|candidate| candidate.id == *group)
                        .map_or("unknown", |candidate| candidate.id.as_str()),
                };
                super::refusal_detail::cause(
                    "character-catalog",
                    format!("InvalidSelection group={group_id}"),
                );
            }
            permitted
        })
    });
    if !allowed {
        return Err(Error::InvalidChoice);
    }
    with_creation(|rules| df_rules::local_journey::validate_character(rules, name, &borrowed))
}

/// Whether the loaded pack is freeplay with only scene assaults: it states no combat or rest
/// numbers, so its fights use the SRD baseline ones.
fn scene_assault_only(loaded: &LoadedPack) -> bool {
    loaded
        .tuning
        .tactical
        .as_ref()
        .is_some_and(|tactical| tactical.scene_assault.is_some())
}

/// The battle scene of the active pack's scene assaults, when it has them.
pub(super) fn scene_assault_battle() -> Option<String> {
    active().and_then(|loaded| {
        loaded
            .tuning
            .tactical
            .as_ref()?
            .scene_assault
            .as_ref()
            .map(|assault| assault.battle_scene.clone())
    })
}

pub(super) fn combat() -> df_rules::local_journey::CombatRules {
    active().map_or(df_rules::local_journey::SRD_521_BASELINE.combat, |loaded| {
        if scene_assault_only(&loaded) {
            return df_rules::local_journey::SRD_521_BASELINE.combat;
        }
        let value = loaded.tuning.combat;
        df_rules::local_journey::CombatRules {
            critical_threshold: value.critical_threshold,
            round_ticks: value.round_ticks,
            combat_log_entries: value.combat_log_entries,
        }
    })
}
pub(super) fn second_wind() -> df_rules::local_journey::SecondWindRules {
    active().map_or(
        df_rules::local_journey::SRD_521_BASELINE.second_wind,
        |loaded| {
            let value = loaded.tuning.hero.second_wind;
            df_rules::local_journey::SecondWindRules {
                uses_max: value.uses_max,
                die_sides: value.die_sides,
                flat_bonus: value.flat_bonus,
                recharge_per_short_rest: value.recharge_per_short_rest,
            }
        },
    )
}
pub(super) fn rest() -> df_rules::local_journey::RestRules {
    active().map_or(df_rules::local_journey::SRD_521_BASELINE.rest, |loaded| {
        if scene_assault_only(&loaded) {
            return df_rules::local_journey::SRD_521_BASELINE.rest;
        }
        df_rules::local_journey::RestRules {
            short_rest_seconds: loaded.tuning.rest.short_rest_seconds,
        }
    })
}
pub(super) fn hero_weapon() -> df_rules::local_journey::WeaponRules {
    active().map_or(
        df_rules::local_journey::SRD_521_BASELINE.hero_weapon,
        |loaded| df_rules::local_journey::WeaponRules {
            damage_dice: loaded.tuning.hero.weapon.damage_dice,
            damage_sides: loaded.tuning.hero.weapon.damage_sides,
        },
    )
}
pub(super) fn bandit() -> df_rules::local_journey::BanditRules {
    active().map_or(df_rules::local_journey::SRD_521_BASELINE.bandit, |loaded| {
        let value = &loaded.tuning.bandit;
        let abilities = value.abilities;
        df_rules::local_journey::BanditRules {
            hit_points: value.hit_points,
            armor_class: value.armor_class,
            abilities: [
                abilities.strength,
                abilities.dexterity,
                abilities.constitution,
                abilities.intelligence,
                abilities.wisdom,
                abilities.charisma,
            ],
            attack_bonus: i32::from(value.attack_bonus),
            damage_modifier: i32::from(value.damage_modifier),
            weapon: df_rules::local_journey::WeaponRules {
                damage_dice: value.weapon.damage_dice,
                damage_sides: value.weapon.damage_sides,
            },
        }
    })
}
/// Every speaking character of the active pack, for voice casting: name, gender, age and the
/// free-text voice phrase.
pub(super) fn speakers() -> Vec<SpeakerProfile> {
    with_content(|pack| pack.speakers.clone())
}
pub(super) fn narrative_budget() -> u64 {
    active().map_or(8, |loaded| {
        u64::from(loaded.tuning.narrative.intervention_budget)
    })
}

/// Qualification text is frozen separately from editable content. Startup never uses this
/// compiled fixture as a fallback for interactive play; serve() must load and install first.
pub(super) fn with_content<T>(run: impl FnOnce(&PackFile) -> T) -> T {
    if let Some(loaded) = active() {
        return run(&loaded.pack);
    }
    static QUALIFICATION: std::sync::OnceLock<PackFile> = std::sync::OnceLock::new();
    let content = QUALIFICATION.get_or_init(|| {
        // Internal invariant: this frozen compiled fixture is checked against the loader and
        // captured pre-tuning views in tests; it cannot be supplied by a running client/file.
        parse(
            SourceFile::Pack,
            include_bytes!("fixtures/harbor-journey-pack-baseline.json"),
        )
        .expect("compiled qualification content is valid")
    });
    run(content)
}

pub(super) fn source_scene_asset(
    key: SceneKey,
) -> Result<String, df_session::submission::RepositoryError> {
    match active() {
        Some(loaded) => loaded
            .source_scene_assets
            .iter()
            .find(|(scene, _)| *scene == key)
            .map(|(_, asset)| asset.clone())
            .ok_or(df_session::submission::RepositoryError::InvalidCandidate),
        None => Ok(scene(key).asset),
    }
}
pub(super) fn scene(key: SceneKey) -> Scene {
    with_content(|pack| {
        pack.scenes
            .iter()
            .find(|scene| scene.key == key)
            .expect("validated pack contains every compiled scene")
            .clone()
    })
}

pub(super) fn hero_weapon_item() -> String {
    active().map_or_else(
        || "greatsword".to_owned(),
        |loaded| loaded.tuning.hero.weapon.item.clone(),
    )
}
pub(super) fn hero_weapon_quantity() -> u64 {
    let weapon = hero_weapon_item();
    with_content(|content| {
        u64::from(
            content
                .equipment
                .iter()
                .find(|row| row.item == weapon)
                .expect("validated hero weapon has starting equipment")
                .quantity,
        )
    })
}

/// The digest-bound internal melee slot remains scimitar for every creature.
pub(super) fn bandit_weapon_item() -> String {
    "scimitar".to_owned()
}

pub(super) fn destination_label() -> String {
    with_content(|pack| {
        pack.destination_label.clone().unwrap_or_else(|| {
            scene(if pack.is_freeplay() {
                SceneKey::Opening
            } else {
                SceneKey::Inn
            })
            .title
            .trim_start_matches("The ")
            .to_owned()
        })
    })
}

pub(super) fn offer_label(kind: df_protocol::common::GameplayActionKind) -> String {
    use df_protocol::common::GameplayActionKind as Action;
    if kind == Action::StartNewAdventure {
        return "Start a new adventure".to_owned();
    }
    with_content(|pack| {
        let labels = &pack.offer_labels;
        match kind {
            Action::CreateCharacter => &labels.create_character,
            Action::BeginStory => &labels.begin_story,
            Action::AskCourier => &labels.ask_courier,
            Action::EscortCourier => &labels.escort_courier,
            Action::DefendCourier => &labels.defend_courier,
            Action::GreatswordAttack => &labels.greatsword_attack,
            Action::SecondWind => &labels.second_wind,
            Action::EndTurn => &labels.end_turn,
            Action::ShortRest => &labels.short_rest,
            Action::ChooseHarborScene => &labels.choose_harbor_scene,
            _ => unreachable!("only compiled authored offers have labels"),
        }
        .clone()
    })
}

fn validate_tempo(tempo: TempoTuning) -> Result<(), PackError> {
    let file = SourceFile::Tuning;
    for (index, value) in [
        ("room", tempo.base_intensity_permille.room),
        ("opening", tempo.base_intensity_permille.opening),
        ("dialogue", tempo.base_intensity_permille.dialogue),
        ("combat", tempo.base_intensity_permille.combat),
        ("victory", tempo.base_intensity_permille.victory),
        ("defeat", tempo.base_intensity_permille.defeat),
        ("rest", tempo.base_intensity_permille.rest),
        ("arrival", tempo.base_intensity_permille.arrival),
    ]
    .iter()
    {
        range(
            file,
            &format!("tempo.base_intensity_permille.{index}"),
            i64::from(*value),
            0,
            1000,
        )?;
    }
    for (index, value) in [
        ("story_begun", tempo.impulses_permille.story_begun),
        ("scene_changed", tempo.impulses_permille.scene_changed),
        (
            "antagonist_revealed",
            tempo.impulses_permille.antagonist_revealed,
        ),
        ("combat_begun", tempo.impulses_permille.combat_begun),
        ("miss", tempo.impulses_permille.miss),
        ("hit", tempo.impulses_permille.hit),
        ("critical", tempo.impulses_permille.critical),
        ("graze", tempo.impulses_permille.graze),
        ("hero_low", tempo.impulses_permille.hero_low),
        (
            "combatant_dropped",
            tempo.impulses_permille.combatant_dropped,
        ),
        ("victory", tempo.impulses_permille.victory),
        ("defeat", tempo.impulses_permille.defeat),
        ("short_rest", tempo.impulses_permille.short_rest),
        ("arrival", tempo.impulses_permille.arrival),
    ]
    .iter()
    {
        range(
            file,
            &format!("tempo.impulses_permille.{index}"),
            i64::from(*value),
            -1000,
            1000,
        )?;
    }
    for (name, value) in [
        ("inertia_half_life_millis", tempo.inertia_half_life_millis),
        ("decay_half_life_millis", tempo.decay_half_life_millis),
        ("fatigue_half_life_millis", tempo.fatigue_half_life_millis),
    ] {
        range(file, &format!("tempo.{name}"), i64::from(value), 1, 300000)?;
    }
    range(
        file,
        "tempo.release_hold_millis",
        i64::from(tempo.release_hold_millis),
        1_000,
        120_000,
    )?;
    tempo.policy().validate().map_err(|_| {
        malformed(
            file,
            "tempo",
            "ordered thresholds and bounded fatigue/hysteresis",
        )
    })
}

#[cfg(test)]
mod character_refusal_tests {
    use super::*;

    #[test]
    fn character_catalog_refusals_preserve_causes_without_untrusted_text() {
        let mut tuning: serde_json::Value = serde_json::from_slice(include_bytes!(
            "fixtures/harbor-journey-tuning-baseline.json"
        ))
        .unwrap();
        tuning["tactical"] =
            serde_json::json!({"enemies":[{"srd_id":"bandit","count":3,"ammunition":20}]});
        let loaded = load_from_bytes(
            include_bytes!("../../../../content/harbor-journey/pack.json"),
            &serde_json::to_vec(&tuning).unwrap(),
        )
        .unwrap();
        with_test_pack(loaded, || {
            let cases: &[(&[(&str, &str)], &str)] = &[
                (
                    &[
                        ("build-package", "secret-option"),
                        ("build-package", "secret-option"),
                    ],
                    "DuplicateGroup group=build-package",
                ),
                (
                    &[("portrait", "secret-option")],
                    "InvalidGroup group=portrait",
                ),
                (
                    &[
                        ("portrait", "portrait-hero-1"),
                        ("portrait", "portrait-hero-1"),
                    ],
                    "InvalidGroup group=portrait",
                ),
                (
                    &[("build-package", "secret-option"), ("array", "strong")],
                    "PackageConflict group=build-package",
                ),
                (
                    &[("build-package", "secret-option")],
                    "UnknownPackage group=build-package",
                ),
                (&[("array", "dexterous")], "PackageRequired group=array"),
                (
                    &[("alignment", "secret-option")],
                    "InvalidSelection group=alignment",
                ),
                (
                    &[("secret-group", "secret-option")],
                    "InvalidSelection group=unknown",
                ),
                (
                    &[("array", "secret-option")],
                    "InvalidSelection group=array",
                ),
            ];
            for (choices, expected) in cases {
                super::super::refusal_detail::begin_action();
                assert!(matches!(
                    validate_character("secret-name", choices),
                    Err(df_rules::local_journey::JourneyRuleError::InvalidChoice),
                ));
                let diagnostic = df_session::submission::take_refusal_diagnostic().unwrap();
                assert_eq!(diagnostic.step, "character-catalog");
                assert_eq!(diagnostic.cause, *expected);
                assert!(!diagnostic.cause.contains("secret"));
            }
        });
    }
}
