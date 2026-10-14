//! The shot director: stages one story scene as a closed, validated shot spec (PLAY-V09).
//!
//! The server already knows everything a film still of a scene needs: the place and what kind of
//! place it is, who speaks and what they do for a living, which heroes stand in the party, the
//! time of day the story text implies. This module turns those facts into a [`ShotSpec`] with
//! plain rules and a seeded random stream, no model: every choice is an entry of a closed
//! vocabulary (`shot/place_families.json` and the enums below), so the spec can only ever
//! describe a staging the prompt module has a fixed phrase for, and the same scene always gets
//! the same staging. The spec carries no player text and no model output.
//!
//! The camera stands with the party and well back: heroes are small (about a fifth of the frame
//! wide at most), the speaker is the lit mid-ground focal point, and the side of the speaker that
//! faces the middle of the frame is kept calm for the speech bubble ([`PlateLayout`] says where).

use std::{collections::BTreeMap, sync::OnceLock};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{
    imagery::LifeStage,
    pack::{Age, Gender, SceneKey},
};

#[path = "scene_graph.rs"]
pub(super) mod scene_graph;

const VOCABULARY_JSON: &str = include_str!("shot/place_families.json");
const VOCABULARY_SCHEMA: &str = "df-shot-vocabulary/1";

/// The most heroes a plate shows: reference art exists for three.
pub(super) const MAXIMUM_HEROES: usize = 3;
const MAXIMUM_EXTRAS: usize = 3;
const MAXIMUM_DRESSING: usize = 2;
const MAXIMUM_LIGHTS: usize = 3;

/// Where the speaker's head is asked for, as a fraction of the frame. The model lands heads higher
/// than asked: about nine percent of the frame height in PLAY-V03, about fourteen in the pulled-back
/// plates of PLAY-V09 (asking for 46% down gave 30 to 33%), so the layout the display uses is
/// raised by [`HEAD_RAISE_BIAS_PERMILLE`] from this.
const SPEAKER_ASKED_Y_PERMILLE: u16 = 560;
const HEAD_RAISE_BIAS_PERMILLE: u16 = 180;
const SPEAKER_RIGHT_X_PERMILLE: u16 = 700;
const SPEAKER_LEFT_X_PERMILLE: u16 = 300;
/// The model drifts the speaker toward the middle of the frame (asked 64% across, landed 60 to 62;
/// asked 36, landed 42 to 50), so the layout is moved this far toward the middle from the asked x.
const HEAD_DRIFT_PERMILLE: u16 = 60;
/// The head box the display keeps clear: generous, because heads land within about five percent
/// of the asked position horizontally.
const HEAD_WIDTH_PERMILLE: u16 = 160;
const HEAD_HEIGHT_PERMILLE: u16 = 180;
/// How far toward the middle of the frame the calm space for the bubble reaches, as a multiple
/// of the head box width, and how far above the head it reaches in permille of the frame height.
const CALM_WIDTH_PERMILLE: u16 = 260;
const CALM_RISE_PERMILLE: u16 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum TimeOfDay {
    Dawn,
    Morning,
    Midday,
    Afternoon,
    Dusk,
    Night,
}

impl TimeOfDay {
    #[cfg(test)]
    pub(in crate::gameplay) const ALL: [Self; 6] = [
        Self::Dawn,
        Self::Morning,
        Self::Midday,
        Self::Afternoon,
        Self::Dusk,
        Self::Night,
    ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum Camera {
    OverShoulderLeft,
    OverShoulderRight,
    WideEstablishing,
    SpeakerClose,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum Side {
    Left,
    Right,
}

impl Side {
    fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum HeroSlot {
    LeftForeground,
    RightForeground,
    LeftMid,
    RightMid,
    BackLeft,
    BackRight,
}

impl HeroSlot {
    /// Whether the slot is on the right of the frame.
    fn is_right(self) -> bool {
        matches!(
            self,
            Self::RightForeground | Self::RightMid | Self::BackRight
        )
    }

    /// Where the hero's head is intended to be, in permille of the frame: the centre.
    fn head_centre(self) -> (u16, u16) {
        match self {
            Self::LeftForeground => (140, 520),
            Self::RightForeground => (860, 520),
            Self::LeftMid => (200, 470),
            Self::RightMid => (800, 470),
            Self::BackLeft => (110, 400),
            Self::BackRight => (890, 400),
        }
    }

    /// The hero head box the bubble's calm space must not touch, in permille of the frame.
    fn head_box(self) -> Rectangle {
        let (x, y) = self.head_centre();
        Rectangle::centred(x, y, 100, 150)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum SpeakerSlot {
    MidLeft,
    MidRight,
}

impl SpeakerSlot {
    fn side(self) -> Side {
        match self {
            Self::MidLeft => Side::Left,
            Self::MidRight => Side::Right,
        }
    }

    /// The hero slot of the same side at the same depth: the speaker stands there.
    fn blocked_hero_slot(self) -> HeroSlot {
        match self {
            Self::MidLeft => HeroSlot::LeftMid,
            Self::MidRight => HeroSlot::RightMid,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum Expression {
    Warm,
    Wary,
    Weary,
    Stern,
    Anxious,
    Amused,
    Pleased,
    Grim,
    Neutral,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum Mood {
    Tense,
    Warm,
    Grim,
    Hopeful,
    Wary,
    Quiet,
    Uneasy,
    Urgent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum ExtraZone {
    FarLeft,
    FarRight,
    Deep,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum DressingZone {
    BottomLeft,
    BottomRight,
}

/// A rectangle in permille of the frame, `right` and `bottom` exclusive. Edges are signed so a
/// calm space that reaches past the frame can be represented before it is clamped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct Rectangle {
    pub(in crate::gameplay) left: i32,
    pub(in crate::gameplay) top: i32,
    pub(in crate::gameplay) right: i32,
    pub(in crate::gameplay) bottom: i32,
}

impl Rectangle {
    fn centred(x: u16, y: u16, width: u16, height: u16) -> Self {
        let (x, y, half_width, half_height) = (
            i32::from(x),
            i32::from(y),
            i32::from(width) / 2,
            i32::from(height) / 2,
        );
        Self {
            left: x - half_width,
            top: y - half_height,
            right: x + half_width,
            bottom: y + half_height,
        }
    }

    pub(in crate::gameplay) fn intersects(self, other: Self) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }
}

/// What the display needs to put the speech bubble beside the speaker's head: the head box and
/// the side of it that stays calm, in permille of the frame (0 to 1000 both ways).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct PlateLayout {
    pub(in crate::gameplay) bubble_side: Side,
    pub(in crate::gameplay) head_x_permille: u16,
    pub(in crate::gameplay) head_y_permille: u16,
    pub(in crate::gameplay) head_width_permille: u16,
    pub(in crate::gameplay) head_height_permille: u16,
}

impl PlateLayout {
    pub(in crate::gameplay) fn head_box(&self) -> Rectangle {
        Rectangle::centred(
            self.head_x_permille,
            self.head_y_permille,
            self.head_width_permille,
            self.head_height_permille,
        )
    }

    /// The space beside the head the model is asked to keep calm for the bubble.
    pub(in crate::gameplay) fn calm_box(&self) -> Rectangle {
        let head = self.head_box();
        let reach = i32::from(CALM_WIDTH_PERMILLE);
        let (left, right) = match self.bubble_side {
            Side::Left => (head.left - reach, head.left),
            Side::Right => (head.right, head.right + reach),
        };
        Rectangle {
            left,
            top: head.top - i32::from(CALM_RISE_PERMILLE),
            right,
            bottom: head.bottom,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct SpeakerShot {
    /// What the speaker looks like at a glance, a fixed phrase by their kind of person.
    pub(in crate::gameplay) look: &'static str,
    pub(in crate::gameplay) gender: Option<Gender>,
    pub(in crate::gameplay) life_stage: LifeStage,
    pub(in crate::gameplay) task: &'static str,
    pub(in crate::gameplay) expression: Expression,
    pub(in crate::gameplay) slot: SpeakerSlot,
    pub(in crate::gameplay) layout: PlateLayout,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct HeroShot {
    /// The hero's art number, 1 to [`MAXIMUM_HEROES`]: the cut-out is `sprite-hero-<n>`.
    pub(in crate::gameplay) art: u8,
    pub(in crate::gameplay) slot: HeroSlot,
    pub(in crate::gameplay) activity: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct ExtraShot {
    pub(in crate::gameplay) name: &'static str,
    pub(in crate::gameplay) zone: ExtraZone,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct DressingShot {
    pub(in crate::gameplay) item: &'static str,
    pub(in crate::gameplay) zone: DressingZone,
}

/// The staging of one scene. Every string is an entry of the closed vocabulary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct ShotSpec {
    pub(in crate::gameplay) family: &'static str,
    pub(in crate::gameplay) place_description: &'static str,
    /// From the adventure record, so every plate of a place keeps its interior or exterior.
    pub(in crate::gameplay) exterior: bool,
    pub(in crate::gameplay) time_of_day: TimeOfDay,
    pub(in crate::gameplay) lights: Vec<&'static str>,
    pub(in crate::gameplay) camera: Camera,
    pub(in crate::gameplay) speaker: Option<SpeakerShot>,
    pub(in crate::gameplay) heroes: Vec<HeroShot>,
    /// What the other people of the scene's cast look like; at most one stands far back.
    pub(in crate::gameplay) others: Vec<&'static str>,
    pub(in crate::gameplay) extras: Vec<ExtraShot>,
    pub(in crate::gameplay) dressing: Vec<DressingShot>,
    pub(in crate::gameplay) mood: Mood,
}

/// The speaker of the scene, from the cast.
#[derive(Clone, Copy, Debug)]
pub(in crate::gameplay) struct SpeakerInput<'a> {
    /// The cut-out's asset stem, such as `sprite-patron-herbwife`.
    pub(in crate::gameplay) sprite: &'a str,
    pub(in crate::gameplay) gender: Option<Gender>,
    pub(in crate::gameplay) age: Option<Age>,
}

/// Everything the director reads about a scene.
#[derive(Clone, Copy, Debug)]
pub(in crate::gameplay) struct ShotInput<'a> {
    pub(in crate::gameplay) scene_key: SceneKey,
    pub(in crate::gameplay) place_id: &'a str,
    /// The setting kind's kebab-case name, such as `herb-shed`.
    pub(in crate::gameplay) place_kind: &'a str,
    /// The adventure record's own word: the place is outdoors (true) or an enclosed interior.
    pub(in crate::gameplay) exterior: bool,
    pub(in crate::gameplay) title: &'a str,
    /// The scene's story text (narration and the speaker's line). Only read for whole-word
    /// cues of the time of day and the speaker's expression; never copied anywhere.
    pub(in crate::gameplay) story_text: &'a str,
    pub(in crate::gameplay) speaker: Option<SpeakerInput<'a>>,
    /// The cut-out stems of the cast besides the speaker.
    pub(in crate::gameplay) others: &'a [&'a str],
    /// The art numbers of the heroes present, in party order.
    pub(in crate::gameplay) heroes: &'a [u8],
}

/// Why a scene has no shot spec. The scene then keeps its authored look.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum ShotError {
    /// The place kind belongs to no family of the vocabulary.
    UnknownPlaceKind,
    /// The speaker is a creature, not a person, or no person at all.
    NoStageablePerson,
    /// There is nobody to stage: no hero and no speaker.
    EmptyScene,
    /// The embedded vocabulary does not parse or is incomplete (a build defect).
    Vocabulary,
    /// The staging broke a rule of the closed vocabulary (a director defect): never sent.
    Rule(Invalid),
}

#[derive(Debug, Deserialize)]
struct Tagged {
    text: String,
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Family {
    table_kinds: Vec<String>,
    fixed_description: String,
    lights: Vec<String>,
    speaker_tasks: Vec<Tagged>,
    activities: Vec<Tagged>,
    extras: Vec<String>,
    dressing: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Vocabulary {
    schema: String,
    families: BTreeMap<String, Family>,
}

fn vocabulary() -> Result<&'static Vocabulary, ShotError> {
    static LOADED: OnceLock<Option<Vocabulary>> = OnceLock::new();
    LOADED
        .get_or_init(|| {
            let vocabulary: Vocabulary = serde_json::from_str(VOCABULARY_JSON).ok()?;
            let complete = vocabulary.schema == VOCABULARY_SCHEMA
                && vocabulary.families.values().all(|family| {
                    !family.table_kinds.is_empty()
                        && !family.lights.is_empty()
                        && !family.speaker_tasks.is_empty()
                        && !family.activities.is_empty()
                        && family.extras.len() >= MAXIMUM_EXTRAS
                        && family.dressing.len() >= MAXIMUM_DRESSING
                });
            complete.then_some(vocabulary)
        })
        .as_ref()
        .ok_or(ShotError::Vocabulary)
}

/// Every phrase the director can put in a spec besides the closed enums: the vocabulary's own
/// entries and the looks of people, for the prompt's closed-vocabulary check.
#[cfg(test)]
pub(in crate::gameplay) fn vocabulary_phrases() -> Vec<&'static str> {
    let mut phrases: Vec<&'static str> = PEOPLE.iter().map(|(_, person)| person.look).collect();
    if let Ok(vocabulary) = vocabulary() {
        for family in vocabulary.families.values() {
            phrases.push(&family.fixed_description);
            phrases.extend(family.speaker_tasks.iter().map(|task| task.text.as_str()));
            phrases.extend(
                family
                    .activities
                    .iter()
                    .map(|activity| activity.text.as_str()),
            );
            phrases.extend(family.extras.iter().map(String::as_str));
            phrases.extend(family.dressing.iter().map(String::as_str));
        }
    }
    phrases
}

/// Every light name any family names, for the prompt module's closed phrase table.
#[cfg(test)]
pub(in crate::gameplay) fn vocabulary_lights() -> Vec<&'static str> {
    let mut lights: Vec<&'static str> = vocabulary()
        .map(|vocabulary| {
            vocabulary
                .families
                .values()
                .flat_map(|family| family.lights.iter().map(String::as_str))
                .collect()
        })
        .unwrap_or_default();
    lights.sort_unstable();
    lights.dedup();
    lights
}

/// The family of a setting kind.
fn family_of(place_kind: &str) -> Result<(&'static str, &'static Family), ShotError> {
    vocabulary()?
        .families
        .iter()
        .find(|(name, family)| {
            family.table_kinds.iter().any(|kind| kind == place_kind)
                || (name.as_str() == "archive" && place_kind == "library-hall")
        })
        .map(|(name, family)| (name.as_str(), family))
        .ok_or(ShotError::UnknownPlaceKind)
}

/// A small deterministic random stream (SplitMix64) seeded from text.
struct Stream(u64);

impl Stream {
    fn seeded(parts: &[&str]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"df-shot/1");
        for part in parts {
            hasher.update([0]);
            hasher.update(part.as_bytes());
        }
        let digest = hasher.finalize();
        let mut seed = [0_u8; 8];
        seed.copy_from_slice(&digest[..8]);
        Self(u64::from_le_bytes(seed))
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    fn below(&mut self, count: usize) -> usize {
        let count = u64::try_from(count.max(1)).unwrap_or(1);
        usize::try_from(self.next() % count).unwrap_or(0)
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        items.get(self.below(items.len()))
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for index in (1..items.len()).rev() {
            items.swap(index, self.below(index + 1));
        }
    }
}

/// What a speaker's cut-out stem says about their kind of person: the tags that pick their task
/// and the fixed phrase that names what they are. The phrase says no clothes or colours: the
/// reference images carry those, and a description that disagrees with them (a plain tunic where
/// the art has a kerchief) made the same person drift between two scenes' plates.
struct Person {
    tags: &'static [&'static str],
    look: &'static str,
}

/// A creature or monster stem: such a scene is never staged as a conversation plate.
const CREATURE_STEMS: &[&str] = &[
    "ant-giant-rat",
    "ant-giant-weasel",
    "ant-skeleton",
    "ant-zombie",
    "ant-wolf",
    "ant-mastiff",
    "ant-goblin",
    "ant-kobold",
    "villain-dragon",
    "villain-lich",
    "villain-fiend",
    "villain-fey",
    "villain-sleeping-horror",
];

/// Matched in order by substring of the stem; the last entry matches anything.
const PEOPLE: &[(&str, Person)] = &[
    (
        "herbwife",
        Person {
            tags: &["healer"],
            look: "a herbwife",
        },
    ),
    (
        "innkeeper",
        Person {
            tags: &["keeper", "cook"],
            look: "an innkeeper",
        },
    ),
    (
        "merchant",
        Person {
            tags: &["merchant"],
            look: "a merchant",
        },
    ),
    (
        "shepherd",
        Person {
            tags: &["shepherd"],
            look: "a shepherd",
        },
    ),
    (
        "veteran",
        Person {
            tags: &["fighter", "guard"],
            look: "a veteran soldier",
        },
    ),
    (
        "acolyte",
        Person {
            tags: &["healer"],
            look: "an acolyte",
        },
    ),
    (
        "archivist",
        Person {
            tags: &["scholar", "clerk"],
            look: "an archivist",
        },
    ),
    (
        "bell-ringer",
        Person {
            tags: &["keeper"],
            look: "a bell ringer",
        },
    ),
    (
        "lamp-warden",
        Person {
            tags: &["keeper"],
            look: "a lamp warden",
        },
    ),
    (
        "ferry-clerk",
        Person {
            tags: &["clerk", "keeper"],
            look: "a ferry clerk",
        },
    ),
    (
        "miller",
        Person {
            tags: &["tradesman"],
            look: "a miller",
        },
    ),
    (
        "mapmaker",
        Person {
            tags: &["scholar"],
            look: "an old mapmaker",
        },
    ),
    (
        "river-healer",
        Person {
            tags: &["healer"],
            look: "a river healer",
        },
    ),
    (
        "shrine-keeper",
        Person {
            tags: &["keeper", "healer"],
            look: "a shrine keeper",
        },
    ),
    (
        "tinker",
        Person {
            tags: &["tradesman"],
            look: "a tinker",
        },
    ),
    (
        "watch-sergeant",
        Person {
            tags: &["guard", "fighter"],
            look: "a watch sergeant",
        },
    ),
    (
        "guard",
        Person {
            tags: &["guard", "fighter"],
            look: "a guard",
        },
    ),
    (
        "bandit",
        Person {
            tags: &["fighter"],
            look: "a fighter",
        },
    ),
    (
        "",
        Person {
            tags: &[],
            look: "a person",
        },
    ),
];

fn person_of(sprite: &str) -> Option<&'static Person> {
    let stem = sprite.rsplit('/').next().unwrap_or(sprite);
    if CREATURE_STEMS
        .iter()
        .any(|creature| stem.contains(creature))
    {
        return None;
    }
    PEOPLE
        .iter()
        .find(|(needle, _)| stem.contains(needle))
        .map(|(_, person)| person)
}

fn tags_match(entry: &[String], wanted: &[&str]) -> bool {
    entry.iter().any(|tag| wanted.contains(&tag.as_str()))
}

/// The tags a hero's art number gives: what their cut-out is (a fighter with a greatsword, an
/// archer) picks which activities suit them.
/// Whether a pose has a hand on a weapon (a hero holds only the weapon of their own sheet).
fn handles_a_weapon(activity: &str) -> bool {
    contains_any(
        activity,
        &[
            "sword", "spear", "blade", "bow", "knife", "dagger", "axe", "hilt",
        ],
    )
}

fn hero_tags(art: u8) -> &'static [&'static str] {
    match art {
        3 => &["ranger"],
        _ => &["fighter"],
    }
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn contains_word_or_phrase(text: &str, phrase: &str) -> bool {
    let words: Vec<&str> = text
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let phrase_words: Vec<&str> = phrase
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    !phrase_words.is_empty()
        && words
            .windows(phrase_words.len())
            .any(|candidate| candidate == phrase_words)
}

fn story_time(text: &str) -> Option<TimeOfDay> {
    if ["dawn", "sunrise", "first light"]
        .iter()
        .any(|cue| contains_word_or_phrase(text, cue))
    {
        Some(TimeOfDay::Dawn)
    } else if ["dusk", "sunset", "evening"]
        .iter()
        .any(|cue| contains_word_or_phrase(text, cue))
    {
        Some(TimeOfDay::Dusk)
    } else if ["night", "tonight", "moon", "dark"]
        .iter()
        .any(|cue| contains_word_or_phrase(text, cue))
    {
        Some(TimeOfDay::Night)
    } else if contains_word_or_phrase(text, "morning") {
        Some(TimeOfDay::Morning)
    } else {
        None
    }
}

fn story_expression(text: &str, mood: Mood) -> Expression {
    const RULES: &[(&[&str], Expression)] = &[
        (
            &[
                "please", "tremb", "follow", "afraid", "stiff", "can't", "hurry", "shadow",
            ],
            Expression::Anxious,
        ),
        (&["smile", "cruel", "laugh"], Expression::Amused),
        (
            &["thank", "remember", "forgot", "welcome", "warm", "fire"],
            Expression::Warm,
        ),
        (
            &["hand over", "keep it", "break you", "need", "must"],
            Expression::Stern,
        ),
        (
            &["came through", "protected", "paid", "bell"],
            Expression::Pleased,
        ),
        (
            &["strained", "tired", "rubbing", "weary"],
            Expression::Weary,
        ),
    ];
    RULES
        .iter()
        .find(|(needles, _)| contains_any(text, needles))
        .map_or_else(
            || match mood {
                Mood::Warm => Expression::Warm,
                Mood::Hopeful => Expression::Pleased,
                Mood::Grim => Expression::Grim,
                Mood::Tense => Expression::Stern,
                Mood::Wary => Expression::Wary,
                Mood::Uneasy | Mood::Urgent => Expression::Anxious,
                Mood::Quiet => Expression::Neutral,
            },
            |(_, expression)| *expression,
        )
}

fn mood_of(scene_key: SceneKey, text: &str) -> Mood {
    let mood = match scene_key {
        SceneKey::Room => Mood::Quiet,
        SceneKey::Opening => Mood::Wary,
        SceneKey::DialogueSeal => Mood::Uneasy,
        SceneKey::DialogueEscort => Mood::Urgent,
        SceneKey::Combat => Mood::Tense,
        SceneKey::Victory => Mood::Hopeful,
        SceneKey::Defeat => Mood::Grim,
        SceneKey::Inn => Mood::Warm,
    };
    if matches!(mood, Mood::Wary | Mood::Uneasy)
        && contains_any(text, &["dawn", "before", "hurry", "followed"])
    {
        Mood::Urgent
    } else {
        mood
    }
}

fn hero_order(camera: Camera) -> [HeroSlot; 5] {
    use HeroSlot::{BackLeft, BackRight, LeftForeground, LeftMid, RightForeground, RightMid};
    match camera {
        Camera::OverShoulderRight => [
            RightForeground,
            LeftForeground,
            RightMid,
            BackRight,
            BackLeft,
        ],
        Camera::OverShoulderLeft | Camera::WideEstablishing | Camera::SpeakerClose => [
            LeftForeground,
            RightForeground,
            LeftMid,
            BackLeft,
            BackRight,
        ],
    }
}

/// The speaker's layout for a slot and camera: the asked head position, lowered by the measured
/// bias so it is where the head really lands, and the inward side as the calm side.
fn layout_for(slot: SpeakerSlot, camera: Camera) -> PlateLayout {
    let asked_x = match slot {
        SpeakerSlot::MidLeft => SPEAKER_LEFT_X_PERMILLE,
        SpeakerSlot::MidRight => SPEAKER_RIGHT_X_PERMILLE,
    };
    let (width, height) = match camera {
        Camera::SpeakerClose => (HEAD_WIDTH_PERMILLE + 20, HEAD_HEIGHT_PERMILLE + 20),
        _ => (HEAD_WIDTH_PERMILLE, HEAD_HEIGHT_PERMILLE),
    };
    PlateLayout {
        bubble_side: slot.side().opposite(),
        head_x_permille: match slot {
            SpeakerSlot::MidLeft => asked_x + HEAD_DRIFT_PERMILLE,
            SpeakerSlot::MidRight => asked_x - HEAD_DRIFT_PERMILLE,
        },
        head_y_permille: SPEAKER_ASKED_Y_PERMILLE - HEAD_RAISE_BIAS_PERMILLE,
        head_width_permille: width,
        head_height_permille: height,
    }
}

/// Where one hero's head is in the plate (permille of the plate), and the side of it on which the
/// hero's speech bubble stands: toward the middle of the frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct HeroHead {
    /// The hero's art number: the party member at this position (art minus one) in the party.
    pub(in crate::gameplay) art: u8,
    pub(in crate::gameplay) layout: PlateLayout,
}

/// Where the heads are in a finished plate: the speaker's and those of heroes the vision model
/// could find (a hero seen from behind is not always found).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct PlatePlacement {
    pub(in crate::gameplay) speaker: PlateLayout,
    pub(in crate::gameplay) heroes: Vec<HeroHead>,
}

/// The layout for a head that was found in the finished plate (fractions of the picture): the
/// head box a little larger than the measured one, and the bubble on the side of it that has the
/// middle of the frame. Replaces the predicted layout, which the model does not obey closely.
pub(in crate::gameplay) fn measured_layout(
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
) -> PlateLayout {
    let permille = |fraction: f32| (fraction.clamp(0.0, 1.0) * 1000.0).round() as u16;
    let centre_x = permille((left + right) / 2.0);
    let centre_y = permille((top + bottom) / 2.0);
    let grown = |size: f32, least: u16, most: u16| permille(size * 1.3).clamp(least, most);
    PlateLayout {
        bubble_side: if centre_x >= 500 {
            Side::Left
        } else {
            Side::Right
        },
        head_x_permille: centre_x,
        head_y_permille: centre_y,
        head_width_permille: grown(right - left, 90, 240),
        head_height_permille: grown(bottom - top, 120, 300),
    }
}

/// The position the speaker's head is asked for in the prompt: the layout before the bias.
pub(in crate::gameplay) fn asked_head(layout: &PlateLayout) -> (u16, u16) {
    let x = match layout.bubble_side {
        // The bubble is on the left of a speaker standing on the right.
        Side::Left => layout.head_x_permille + HEAD_DRIFT_PERMILLE,
        Side::Right => layout.head_x_permille - HEAD_DRIFT_PERMILLE,
    };
    (x, layout.head_y_permille + HEAD_RAISE_BIAS_PERMILLE)
}

/// Stages a scene. Pure and deterministic: the same input always gives the same spec.
pub(in crate::gameplay) fn direct(input: &ShotInput<'_>) -> Result<ShotSpec, ShotError> {
    let (family_name, family) = family_of(input.place_kind)?;
    let story = input.story_text.to_ascii_lowercase();
    let scene_label = format!("{:?}", input.scene_key);
    // What belongs to the place (the camera, the sides, the props, the light, the hour) is chosen
    // from the place alone, so every plate of a place keeps it; the speaker's task and the heroes'
    // poses vary with the scene.
    let mut stream = Stream::seeded(&[input.place_id, &scene_label, input.title]);
    let mut place_stream = Stream::seeded(&[input.place_id]);
    let heroes_present: Vec<u8> = input
        .heroes
        .iter()
        .copied()
        .filter(|art| (1..=3).contains(art))
        .take(MAXIMUM_HEROES)
        .collect();
    let person = match &input.speaker {
        Some(speaker) => Some((
            speaker,
            person_of(speaker.sprite).ok_or(ShotError::NoStageablePerson)?,
        )),
        None => None,
    };
    if person.is_none() && heroes_present.is_empty() {
        return Err(ShotError::EmptyScene);
    }
    let mood = mood_of(input.scene_key, &story);
    let camera = match (&person, input.scene_key) {
        (None, _) | (Some(_), SceneKey::Combat) => Camera::WideEstablishing,
        (Some(_), SceneKey::Inn | SceneKey::Victory) => Camera::SpeakerClose,
        (Some(_), _) => {
            if place_stream.below(2) == 0 {
                Camera::OverShoulderLeft
            } else {
                Camera::OverShoulderRight
            }
        }
    };
    let speaker = person.map(|(speaker, person)| {
        let slot = match camera {
            Camera::OverShoulderLeft => SpeakerSlot::MidRight,
            Camera::OverShoulderRight => SpeakerSlot::MidLeft,
            Camera::WideEstablishing | Camera::SpeakerClose => {
                if place_stream.below(2) == 0 {
                    SpeakerSlot::MidLeft
                } else {
                    SpeakerSlot::MidRight
                }
            }
        };
        let specific: Vec<&Tagged> = family
            .speaker_tasks
            .iter()
            .filter(|task| tags_match(&task.tags, person.tags))
            .collect();
        let any: Vec<&Tagged> = family
            .speaker_tasks
            .iter()
            .filter(|task| task.tags.iter().any(|tag| tag == "any"))
            .collect();
        let pool = if specific.is_empty() { any } else { specific };
        let task = stream.pick(&pool).map_or_else(
            || family.speaker_tasks[0].text.as_str(),
            |task| task.text.as_str(),
        );
        SpeakerShot {
            look: person.look,
            gender: speaker.gender,
            life_stage: LifeStage::from_age(speaker.age),
            task,
            expression: story_expression(&story, mood),
            slot,
            layout: layout_for(slot, camera),
        }
    });
    let blocked = speaker
        .as_ref()
        .map(|speaker| speaker.slot.blocked_hero_slot());
    // The first hero stands on the left and the second on the right in every plate of a place.
    let mut slots: Vec<HeroSlot> = hero_order(camera)
        .into_iter()
        .filter(|slot| Some(*slot) != blocked)
        .collect();
    slots.truncate(heroes_present.len());
    slots.sort_by_key(|slot| slot.is_right());
    let mut order = heroes_present;
    order.sort_unstable();
    let mut used_activities: Vec<&str> = Vec::new();
    let mut heroes = Vec::with_capacity(order.len());
    for (art, slot) in order.into_iter().zip(slots) {
        let tags = hero_tags(art);
        // A hero's own weapon is the one on their sheet: no pose may put another in their hands.
        let fresh = |activity: &&Tagged| {
            !used_activities.contains(&activity.text.as_str()) && !handles_a_weapon(&activity.text)
        };
        let specific: Vec<&Tagged> = family
            .activities
            .iter()
            .filter(fresh)
            .filter(|activity| {
                !activity.tags.iter().any(|tag| tag == "any") && tags_match(&activity.tags, tags)
            })
            .collect();
        let general: Vec<&Tagged> = family
            .activities
            .iter()
            .filter(fresh)
            .filter(|activity| activity.tags.iter().any(|tag| tag == "any"))
            .collect();
        let pool = if specific.is_empty() {
            general
        } else {
            specific
        };
        let activity = stream.pick(&pool).map_or_else(
            || family.activities[0].text.as_str(),
            |activity| activity.text.as_str(),
        );
        used_activities.push(activity);
        heroes.push(HeroShot {
            art,
            slot,
            activity,
        });
    }
    let mut extra_names: Vec<&'static str> = family.extras.iter().map(String::as_str).collect();
    place_stream.shuffle(&mut extra_names);
    let mut zones = [ExtraZone::FarLeft, ExtraZone::FarRight, ExtraZone::Deep];
    place_stream.shuffle(&mut zones);
    let extras = extra_names
        .into_iter()
        .zip(zones)
        .take(2)
        .map(|(name, zone)| ExtraShot { name, zone })
        .collect();
    let mut dressing_items: Vec<&'static str> =
        family.dressing.iter().map(String::as_str).collect();
    place_stream.shuffle(&mut dressing_items);
    let dressing = dressing_items
        .into_iter()
        .zip([DressingZone::BottomLeft, DressingZone::BottomRight])
        .map(|(item, zone)| DressingShot { item, zone })
        .collect();
    let mut light_names: Vec<&'static str> = family.lights.iter().map(String::as_str).collect();
    place_stream.shuffle(&mut light_names);
    light_names.truncate(2);
    let time_of_day = story_time(&story).unwrap_or_else(|| {
        let nightly = matches!(family_name, "waterfront" | "woodland" | "watch");
        let choices: &[TimeOfDay] = if nightly {
            &[TimeOfDay::Dusk, TimeOfDay::Night]
        } else {
            &[
                TimeOfDay::Morning,
                TimeOfDay::Midday,
                TimeOfDay::Afternoon,
                TimeOfDay::Dusk,
            ]
        };
        place_stream
            .pick(choices)
            .copied()
            .unwrap_or(TimeOfDay::Dusk)
    });
    let others = input
        .others
        .iter()
        .filter_map(|sprite| person_of(sprite))
        .map(|person| person.look)
        .take(1)
        .collect();
    let spec = ShotSpec {
        family: family_name,
        place_description: family.fixed_description.as_str(),
        exterior: input.exterior,
        time_of_day,
        lights: light_names,
        camera,
        speaker,
        heroes,
        others,
        extras,
        dressing,
        mood,
    };
    spec.validate().map_err(ShotError::Rule)?;
    Ok(spec)
}

/// Why a spec breaks the staging rules.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum Invalid {
    Vocabulary,
    HeroSlot,
    Activity,
    Extras,
    Dressing,
    Lights,
    Speaker,
    BubbleRoom,
}

impl ShotSpec {
    /// Checks the spec against the closed vocabulary and the staging rules: no two heroes in a
    /// slot, nobody in the speaker's slot, every phrase from the family's lists, and nobody's
    /// head inside the space kept calm for the bubble.
    pub(in crate::gameplay) fn validate(&self) -> Result<(), Invalid> {
        let vocabulary = vocabulary().map_err(|_| Invalid::Vocabulary)?;
        let family = vocabulary
            .families
            .get(self.family)
            .ok_or(Invalid::Vocabulary)?;
        if self.place_description != family.fixed_description {
            return Err(Invalid::Vocabulary);
        }
        if !(1..=MAXIMUM_LIGHTS).contains(&self.lights.len())
            || self
                .lights
                .iter()
                .any(|light| !family.lights.iter().any(|known| known == light))
        {
            return Err(Invalid::Lights);
        }
        let blocked = self
            .speaker
            .as_ref()
            .map(|speaker| speaker.slot.blocked_hero_slot());
        for (index, hero) in self.heroes.iter().enumerate() {
            if !(1..=3).contains(&hero.art)
                || Some(hero.slot) == blocked
                || self.heroes[..index]
                    .iter()
                    .any(|other| other.slot == hero.slot || other.art == hero.art)
            {
                return Err(Invalid::HeroSlot);
            }
            if !family
                .activities
                .iter()
                .any(|known| known.text == hero.activity)
            {
                return Err(Invalid::Activity);
            }
        }
        if self.extras.len() > MAXIMUM_EXTRAS
            || self.extras.iter().enumerate().any(|(index, extra)| {
                !family.extras.iter().any(|known| known == extra.name)
                    || self.extras[..index]
                        .iter()
                        .any(|other| other.name == extra.name)
            })
        {
            return Err(Invalid::Extras);
        }
        if self.dressing.len() > MAXIMUM_DRESSING
            || self
                .dressing
                .iter()
                .any(|item| !family.dressing.iter().any(|known| known == item.item))
        {
            return Err(Invalid::Dressing);
        }
        if self.others.len() > 1 {
            return Err(Invalid::Speaker);
        }
        if let Some(speaker) = &self.speaker {
            if !family
                .speaker_tasks
                .iter()
                .any(|known| known.text == speaker.task)
            {
                return Err(Invalid::Speaker);
            }
            let layout = speaker.layout;
            if layout.bubble_side != speaker.slot.side().opposite() {
                return Err(Invalid::Speaker);
            }
            let calm = layout.calm_box();
            if self
                .heroes
                .iter()
                .any(|hero| hero.slot.head_box().intersects(calm))
            {
                return Err(Invalid::BubbleRoom);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "shot_tests.rs"]
mod tests;
