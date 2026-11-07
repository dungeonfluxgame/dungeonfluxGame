//! The pure rules of the emotional layer (`planning/persona-emotion.md`): the emotion list as
//! data, a fresh state at a temperament's rest, taking a model's appraisal into a state, and the
//! checks on a persona's temperament and emotional triggers.
//!
//! Nothing here reads a file or matches text. A model appraises; these functions validate every id
//! against the closed list, scale and clamp every number, and answer the new state. The host
//! (`df-table-host`) keeps the list and the state on disk.

use std::collections::HashSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::emotion::{
    Appraisal, Attention, Attitude, EmotionLevel, EmotionalTrigger, Mood, Noticed, PersonaState,
    Relationship, Temperament,
};
use super::{
    EditError, MAXIMUM_ENTRY_CHARS, MAXIMUM_LIST_ENTRIES, Persona, PersonaError, PersonaField,
    check_line, is_label,
};

/// How much of the gap to its baseline an emotion closes each turn when neither the temperament
/// nor the emotion file says.
pub const DEFAULT_DECAY_PERCENT: u8 = 20;
/// How many recent lines a character attends to when neither its temperament nor the emotion
/// file says.
pub const DEFAULT_ATTENTION_CAPACITY: u8 = 3;

/// How large an appraisal's changes are, in plain words a model reads with the persona's
/// temperament: the content's calibration of feeling, editable in `emotions.json`.
/// How characters behave and speak, in plain words a model reads with each line: the
/// persona engine's tunable conduct, editable in `emotions.json` without a rebuild.
pub const DEFAULT_CONDUCT: [&str; 9] = [
    concat!(
        "Speak like a real person, not a helper: plain words, contractions, short and long ",
        "sentences mixed, no stock pleasantries every line.",
    ),
    concat!(
        "Answer what was asked first, then add one thing of your own when it fits (a ",
        "suggestion, a question back, a remark).",
    ),
    concat!(
        "Live by your trade: a seller sells, knows their goods and prices, suggests what ",
        "fits, and wants the sale without begging for it.",
    ),
    concat!(
        "Keep your own judgment from your expertise and goals: when someone is wrong about ",
        "your trade (too little for a long road, the wrong fruit for a march) say so kindly",
        " and suggest better instead of agreeing to please.",
    ),
    concat!(
        "An ordinary detail about your own life or work that anyone in your place would ",
        "know (how long you have done it, where you buy, your routine) is answered plainly,",
        " the way a real person would.",
    ),
    concat!(
        "Let feeling show in how you speak, in proportion to it: irritation turns clipped ",
        "or sharp, at 70 and above it is anger that can refuse a sale or tell someone to ",
        "leave, at 90 and above it is rage, loud and cold; fear turns placating or curt; ",
        "sadness quiets you; joy and pride warm you; amusement can tease; disgust recoils; ",
        "embarrassment fumbles.",
    ),
    concat!(
        "Attraction makes you warmer, flustered or playful and it may flirt back, but it ",
        "stays suggestive and never explicit; a boundary in the persona or an unwelcome ",
        "advance is refused plainly.",
    ),
    concat!(
        "Respect is earned back slowly: after rudeness, an apology softens you but does not",
        " erase it at once.",
    ),
    concat!(
        "Do not tack a selling point onto every answer: a detail you already said in this ",
        "conversation (how you pack, where it comes from) is not said again unless asked.",
    ),
];

pub const DEFAULT_APPRAISAL_GUIDE: [&str; 8] = [
    concat!(
        "Almost every line moves the listener a little: give at least one emotion change ",
        "unless the line was truly empty of feeling.",
    ),
    concat!(
        "Interest, courtesy or praise of their work: joy, trust or pride up 2 to 8, rapport",
        " toward the speaker up 1 to 5.",
    ),
    concat!(
        "Haggling or doubting their goods: irritation up 2 to 10 (more for low patience); ",
        "pride may rise as they defend their goods.",
    ),
    concat!(
        "Rudeness, an insult, a threat or mishandling their goods: irritation up 10 to 30, ",
        "trust down 5 to 15, rapport down 5 to 20.",
    ),
    "An apology or making amends: irritation down 5 to 15, rapport up 2 to 8.",
    "A sale, a fair deal or a promise of custom: joy up 5 to 12, rapport up 3 to 8.",
    concat!(
        "Scale every change by the temperament: a reactive character moves more, a patient ",
        "or steady one less.",
    ),
    "toward is the speaker of the line just said; give their rapport change there.",
];
/// The most recent lines any character can attend to.
pub const MAXIMUM_ATTENTION_CAPACITY: u8 = 5;
/// The reactivity of a normal person: a change is scaled by reactivity over this.
pub const NEUTRAL_REACTIVITY: u8 = 50;
/// The largest value of an emotion level, a temperament trait and a decay percentage.
pub const MAXIMUM_LEVEL: u8 = 100;
/// The most characters a character holds a standing toward; a new name beyond it is not kept.
pub const MAXIMUM_RELATIONSHIPS: usize = 32;
/// The most characters of the gist of one noticed line, of a focus and of a topic.
pub const MAXIMUM_GIST_CHARS: usize = 200;

/// The rapport at or above which a character is friendly, when the emotion file does not say.
const DEFAULT_FRIENDLY_AT_LEAST: i8 = 30;
/// The rapport at or below which a character is hostile, when the emotion file does not say.
const DEFAULT_HOSTILE_AT_MOST: i8 = -30;

/// The built-in emotions in the order of `DEFAULT_EMOTIONS`: id, label, valence, arousal and the
/// bubble tone id.
const DEFAULT_TABLE: [(&str, &str, i8, u8, &str); 11] = [
    ("joy", "Joy", 80, 60, "ally"),
    ("trust", "Trust", 60, 30, "ally"),
    ("irritation", "Irritation", -60, 70, "danger"),
    ("fear", "Fear", -70, 80, "danger"),
    ("sadness", "Sadness", -70, 20, "mystic"),
    ("surprise", "Surprise", 10, 80, "mystic"),
    ("pride", "Pride", 50, 50, "npc"),
    ("amusement", "Amusement", 70, 50, "ally"),
    ("attraction", "Attraction", 60, 70, "ally"),
    ("disgust", "Disgust", -70, 50, "danger"),
    ("embarrassment", "Embarrassment", -30, 60, "mystic"),
];

/// One emotion a campaign offers: how it reads and how it colours the mood and the bubble.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmotionDefinition {
    pub id: String,
    pub label: String,
    /// -100 (sour) to 100 (glad).
    pub valence: i8,
    /// 0 (calm) to 100 (agitated).
    pub arousal: u8,
    /// The bubble or voice tone id a line in this emotion is shown with.
    pub tone: String,
}

/// The rapport that moves a standing: at or above `friendly_at_least` a character is friendly, at
/// or below `hostile_at_most` hostile, and indifferent between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RapportThresholds {
    pub friendly_at_least: i8,
    pub hostile_at_most: i8,
}

impl Default for RapportThresholds {
    fn default() -> Self {
        Self {
            friendly_at_least: DEFAULT_FRIENDLY_AT_LEAST,
            hostile_at_most: DEFAULT_HOSTILE_AT_MOST,
        }
    }
}

/// A setting of the emotion file, for naming where a check failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmotionSetting {
    Valence,
    Arousal,
    DecayPercent,
    AttentionCapacity,
    RapportThreshold,
}

impl EmotionSetting {
    pub const ALL: [Self; 5] = [
        Self::Valence,
        Self::Arousal,
        Self::DecayPercent,
        Self::AttentionCapacity,
        Self::RapportThreshold,
    ];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Valence => "valence",
            Self::Arousal => "arousal",
            Self::DecayPercent => "decay_percent",
            Self::AttentionCapacity => "attention_capacity",
            Self::RapportThreshold => "rapport_threshold",
        }
    }

    /// The setting with serde name `key`.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|setting| setting.key() == key)
    }
}

/// Why an emotion file is not a usable list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum EmotionConfigError {
    /// The list has no emotion.
    NoEmotions,
    /// An emotion id is empty, too long or has a character other than a letter, a digit, `-`
    /// or `_`.
    InvalidId { id: String },
    /// Two emotions have the same id.
    DuplicateId { id: String },
    /// A text of an emotion is blank, has a control character or is too long.
    InvalidText { id: String },
    /// A number is outside its range.
    OutOfRange { setting: EmotionSetting },
    /// The hostile threshold is not below the friendly one.
    ThresholdsCross,
}

impl EmotionConfigError {
    /// The error's serde tag.
    pub fn key(&self) -> &'static str {
        match self {
            Self::NoEmotions => "no_emotions",
            Self::InvalidId { .. } => "invalid_id",
            Self::DuplicateId { .. } => "duplicate_id",
            Self::InvalidText { .. } => "invalid_text",
            Self::OutOfRange { .. } => "out_of_range",
            Self::ThresholdsCross => "thresholds_cross",
        }
    }
}

impl fmt::Display for EmotionConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoEmotions => write!(formatter, "the emotion list has no emotion"),
            Self::InvalidId { id } => write!(
                formatter,
                "{id:?} is not an emotion id: use letters, digits, '-' or '_'"
            ),
            Self::DuplicateId { id } => {
                write!(
                    formatter,
                    "the emotion list has two emotions with the id {id:?}"
                )
            }
            Self::InvalidText { id } => write!(
                formatter,
                concat!(
                    "the label or tone of the emotion {id:?} is blank, too long or has a ",
                    "control character"
                ),
                id = id
            ),
            Self::OutOfRange { setting } => {
                write!(
                    formatter,
                    "the emotion setting {} is out of range",
                    setting.key()
                )
            }
            Self::ThresholdsCross => write!(
                formatter,
                "the hostile rapport threshold must be below the friendly one"
            ),
        }
    }
}

impl std::error::Error for EmotionConfigError {}

/// The emotions a campaign offers and the numbers that govern them. The built-in value
/// ([`Default`]) lists `DEFAULT_EMOTIONS`; a campaign's `persona/emotions.json` replaces any part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EmotionConfig {
    pub emotions: Vec<EmotionDefinition>,
    /// How much of the gap to its baseline an emotion closes each turn, when the temperament
    /// does not say.
    pub decay_percent: u8,
    /// How many recent lines a character attends to, when the temperament does not say.
    pub attention_capacity: u8,
    pub rapport: RapportThresholds,
    /// How large a model's appraisal changes are, in plain words.
    pub appraisal_guide: Vec<String>,
    /// How characters behave and speak, in plain words.
    pub conduct: Vec<String>,
}

impl Default for EmotionConfig {
    fn default() -> Self {
        Self {
            emotions: DEFAULT_TABLE
                .iter()
                .map(|(id, label, valence, arousal, tone)| EmotionDefinition {
                    id: (*id).to_owned(),
                    label: (*label).to_owned(),
                    valence: *valence,
                    arousal: *arousal,
                    tone: (*tone).to_owned(),
                })
                .collect(),
            decay_percent: DEFAULT_DECAY_PERCENT,
            attention_capacity: DEFAULT_ATTENTION_CAPACITY,
            rapport: RapportThresholds::default(),
            appraisal_guide: DEFAULT_APPRAISAL_GUIDE.map(str::to_owned).to_vec(),
            conduct: DEFAULT_CONDUCT.map(str::to_owned).to_vec(),
        }
    }
}

fn out_of_range(setting: EmotionSetting) -> EmotionConfigError {
    EmotionConfigError::OutOfRange { setting }
}

impl EmotionConfig {
    /// That the list and its numbers are usable.
    pub fn check(&self) -> Result<(), EmotionConfigError> {
        if self.emotions.is_empty() {
            return Err(EmotionConfigError::NoEmotions);
        }
        let mut seen = HashSet::new();
        for emotion in &self.emotions {
            if !is_label(&emotion.id) {
                return Err(EmotionConfigError::InvalidId {
                    id: emotion
                        .id
                        .chars()
                        .take(super::MAXIMUM_ID_CHARS + 1)
                        .collect(),
                });
            }
            if !seen.insert(emotion.id.as_str()) {
                return Err(EmotionConfigError::DuplicateId {
                    id: emotion.id.clone(),
                });
            }
            let texts_fit = [&emotion.label, &emotion.tone].into_iter().all(|text| {
                !text.trim().is_empty()
                    && check_line(PersonaField::Temperament, text, MAXIMUM_ENTRY_CHARS, true)
                        .is_ok()
            });
            if !texts_fit {
                return Err(EmotionConfigError::InvalidText {
                    id: emotion.id.clone(),
                });
            }
            if !(-100..=100).contains(&emotion.valence) {
                return Err(out_of_range(EmotionSetting::Valence));
            }
            if emotion.arousal > MAXIMUM_LEVEL {
                return Err(out_of_range(EmotionSetting::Arousal));
            }
        }
        if self.decay_percent > MAXIMUM_LEVEL {
            return Err(out_of_range(EmotionSetting::DecayPercent));
        }
        if !(1..=MAXIMUM_ATTENTION_CAPACITY).contains(&self.attention_capacity) {
            return Err(out_of_range(EmotionSetting::AttentionCapacity));
        }
        let RapportThresholds {
            friendly_at_least,
            hostile_at_most,
        } = self.rapport;
        if ![friendly_at_least, hostile_at_most]
            .iter()
            .all(|threshold| (-100..=100).contains(threshold))
        {
            return Err(out_of_range(EmotionSetting::RapportThreshold));
        }
        if hostile_at_most >= friendly_at_least {
            return Err(EmotionConfigError::ThresholdsCross);
        }
        Ok(())
    }

    /// The definition of emotion `id`.
    pub fn definition(&self, id: &str) -> Option<&EmotionDefinition> {
        self.emotions.iter().find(|emotion| emotion.id == id)
    }

    /// Whether `id` is on the list.
    pub fn knows(&self, id: &str) -> bool {
        self.definition(id).is_some()
    }

    /// The ids, in list order.
    pub fn ids(&self) -> Vec<&str> {
        self.emotions
            .iter()
            .map(|emotion| emotion.id.as_str())
            .collect()
    }

    /// The attitude a rapport amounts to.
    pub fn attitude_for(&self, rapport: i8) -> Attitude {
        if rapport >= self.rapport.friendly_at_least {
            Attitude::Friendly
        } else if rapport <= self.rapport.hostile_at_most {
            Attitude::Hostile
        } else {
            Attitude::Indifferent
        }
    }
}

/// One of the numbers of a [`Temperament`], for the typed edits and the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemperamentField {
    Reactivity,
    Patience,
    Warmth,
    Pride,
    DecayPercent,
    AttentionCapacity,
    Distractibility,
}

impl TemperamentField {
    pub const ALL: [Self; 7] = [
        Self::Reactivity,
        Self::Patience,
        Self::Warmth,
        Self::Pride,
        Self::DecayPercent,
        Self::AttentionCapacity,
        Self::Distractibility,
    ];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Reactivity => "reactivity",
            Self::Patience => "patience",
            Self::Warmth => "warmth",
            Self::Pride => "pride",
            Self::DecayPercent => "decay_percent",
            Self::AttentionCapacity => "attention_capacity",
            Self::Distractibility => "distractibility",
        }
    }

    /// The field with serde name `key`.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|field| field.key() == key)
    }
}

impl Temperament {
    /// Whether the temperament says nothing yet.
    pub fn is_blank(&self) -> bool {
        *self == Self::default()
    }

    /// Sets `field` to `value`.
    pub(super) fn set(&mut self, field: TemperamentField, value: u8) {
        let slot = match field {
            TemperamentField::Reactivity => &mut self.reactivity,
            TemperamentField::Patience => &mut self.patience,
            TemperamentField::Warmth => &mut self.warmth,
            TemperamentField::Pride => &mut self.pride,
            TemperamentField::DecayPercent => &mut self.decay_percent,
            TemperamentField::AttentionCapacity => &mut self.attention_capacity,
            TemperamentField::Distractibility => &mut self.distractibility,
        };
        *slot = Some(value);
    }

    /// Sets the level `emotion` rests at; a level of 0 removes the entry, since an emotion not
    /// listed rests at 0.
    pub(super) fn set_baseline(&mut self, emotion: &str, level: u8) {
        self.baseline.retain(|seen| seen.emotion != emotion);
        if level > 0 {
            self.baseline.push(EmotionLevel {
                emotion: emotion.to_owned(),
                level,
            });
        }
    }

    /// The level `emotion` rests at.
    pub fn rest_of(&self, emotion: &str) -> u8 {
        self.baseline
            .iter()
            .find(|seen| seen.emotion == emotion)
            .map_or(0, |seen| seen.level)
    }
}

impl PersonaState {
    /// The level of `emotion`; 0 when it is not in the state.
    pub fn level(&self, emotion: &str) -> u8 {
        self.emotions
            .iter()
            .find(|seen| seen.emotion == emotion)
            .map_or(0, |seen| seen.level)
    }

    /// The standing toward `toward`, if the character has one.
    pub fn relationship(&self, toward: &str) -> Option<&Relationship> {
        self.relationships.iter().find(|seen| seen.toward == toward)
    }
}

/// What taking an appraisal into a state dropped, so the host can record that a model's answer
/// had more than the state could hold. Nothing here blocks a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Dropped {
    /// Deltas whose emotion is not on the list.
    pub emotions: usize,
    /// A standing change toward a name that is not a character label, the character itself or one
    /// too many for the character to keep.
    pub relationships: usize,
    /// Noticed lines with a blank speaker or gist.
    pub noticed: usize,
}

impl Dropped {
    /// Whether the appraisal was taken whole.
    pub fn is_none(&self) -> bool {
        *self == Self::default()
    }
}

/// How much of `gap` a decay of `decay_percent` closes in one turn: the share rounded up, so an
/// emotion always reaches its baseline.
fn fade_step(gap: i32, decay_percent: u8) -> i32 {
    let closed = (gap.abs() * i32::from(decay_percent) + 99) / 100;
    closed.min(gap.abs()) * gap.signum()
}

fn level_from(value: i32) -> u8 {
    u8::try_from(value.clamp(0, i32::from(MAXIMUM_LEVEL))).unwrap_or(MAXIMUM_LEVEL)
}

fn decay_of(config: &EmotionConfig, temperament: &Temperament) -> u8 {
    temperament
        .decay_percent
        .unwrap_or(config.decay_percent)
        .min(MAXIMUM_LEVEL)
}

fn capacity_of(config: &EmotionConfig, temperament: &Temperament) -> usize {
    usize::from(
        temperament
            .attention_capacity
            .unwrap_or(config.attention_capacity)
            .clamp(1, MAXIMUM_ATTENTION_CAPACITY),
    )
}

/// The mood the levels amount to: each emotion weighs its level, and the weights are taken over
/// at least 100, so one faint emotion is a faint mood and a strong one reaches its own colour.
pub fn derive_mood(config: &EmotionConfig, emotions: &[EmotionLevel]) -> Mood {
    let mut weight = 0_i32;
    let mut valence = 0_i32;
    let mut arousal = 0_i32;
    for level in emotions {
        if let Some(definition) = config.definition(&level.emotion) {
            let level = i32::from(level.level);
            weight += level;
            valence += level * i32::from(definition.valence);
            arousal += level * i32::from(definition.arousal);
        }
    }
    let over = weight.max(i32::from(MAXIMUM_LEVEL));
    Mood {
        valence: i8::try_from((valence / over).clamp(-100, 100)).unwrap_or(0),
        arousal: level_from(arousal / over),
    }
}

/// The state of `id` at rest: every listed emotion at the temperament's baseline.
pub fn fresh_state(id: &str, config: &EmotionConfig, temperament: &Temperament) -> PersonaState {
    let emotions: Vec<EmotionLevel> = config
        .emotions
        .iter()
        .map(|definition| EmotionLevel {
            emotion: definition.id.clone(),
            level: temperament.rest_of(&definition.id).min(MAXIMUM_LEVEL),
        })
        .collect();
    PersonaState {
        id: id.to_owned(),
        turn: 0,
        mood: derive_mood(config, &emotions),
        emotions,
        attention: Attention::default(),
        relationships: Vec::new(),
    }
}

/// `state` with exactly the listed emotions, in list order, each within range: a stored state
/// survives a campaign that added or dropped an emotion, and a hand-edited level cannot escape.
fn conform(
    config: &EmotionConfig,
    temperament: &Temperament,
    state: &PersonaState,
) -> PersonaState {
    let mut conformed = state.clone();
    conformed.emotions = config
        .emotions
        .iter()
        .map(|definition| EmotionLevel {
            emotion: definition.id.clone(),
            level: if state
                .emotions
                .iter()
                .any(|seen| seen.emotion == definition.id)
            {
                state.level(&definition.id).min(MAXIMUM_LEVEL)
            } else {
                temperament.rest_of(&definition.id).min(MAXIMUM_LEVEL)
            },
        })
        .collect();
    conformed
}

fn trimmed(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| {
        text.chars()
            .filter(|character| !character.is_control())
            .take(MAXIMUM_GIST_CHARS)
            .collect()
    })
}

fn take_attention(
    capacity: usize,
    before: &Attention,
    appraised: &Attention,
    dropped: &mut Dropped,
) -> Attention {
    let mut noticed = before.noticed.clone();
    for line in &appraised.noticed {
        match (trimmed(&line.speaker), trimmed(&line.gist)) {
            (Some(speaker), Some(gist)) => noticed.push(Noticed { speaker, gist }),
            _ => dropped.noticed += 1,
        }
    }
    let excess = noticed.len().saturating_sub(capacity);
    noticed.drain(..excess);
    Attention {
        focus: appraised
            .focus
            .as_deref()
            .and_then(trimmed)
            .or_else(|| before.focus.clone()),
        topic: appraised
            .topic
            .as_deref()
            .and_then(trimmed)
            .or_else(|| before.topic.clone()),
        noticed,
    }
}

fn take_rapport(
    config: &EmotionConfig,
    state: &mut PersonaState,
    toward: &str,
    rapport_delta: i8,
    dropped: &mut Dropped,
) {
    if !is_label(toward) || toward == state.id {
        dropped.relationships += 1;
        return;
    }
    let index = match state
        .relationships
        .iter()
        .position(|seen| seen.toward == toward)
    {
        Some(index) => index,
        None if state.relationships.len() < MAXIMUM_RELATIONSHIPS => {
            state.relationships.push(Relationship {
                toward: toward.to_owned(),
                attitude: config.attitude_for(0),
                rapport: 0,
            });
            state.relationships.len() - 1
        }
        None => {
            dropped.relationships += 1;
            return;
        }
    };
    let standing = &mut state.relationships[index];
    let rapport = (i32::from(standing.rapport) + i32::from(rapport_delta)).clamp(-100, 100);
    standing.rapport = i8::try_from(rapport).unwrap_or(0);
    standing.attitude = config.attitude_for(standing.rapport);
}

/// Takes `appraisal` into `state` for a character with `temperament`, and answers the new state
/// with what was dropped.
///
/// The emotions first fade toward the baseline by the decay of one turn, then each change of the
/// appraisal lands at full strength scaled by the reactivity over [`NEUTRAL_REACTIVITY`], so the
/// reaction to what was just said is felt in the next line and fades after it. A change to an
/// emotion that is not on the list is dropped and counted, never an error. The attention keeps at
/// most the temperament's capacity of noticed lines, the oldest dropped first.
pub fn apply_appraisal(
    config: &EmotionConfig,
    temperament: &Temperament,
    state: &PersonaState,
    appraisal: &Appraisal,
) -> (PersonaState, Dropped) {
    let mut dropped = Dropped::default();
    let mut next = conform(config, temperament, state);
    let decay = decay_of(config, temperament);
    // An emotion this appraisal pushes does not fade this turn: a person does not calm down
    // while they are still being provoked; the rest fade toward their baseline.
    let pushed = |emotion: &str| {
        appraisal
            .emotions
            .iter()
            .any(|change| change.emotion == emotion && change.delta != 0)
    };
    for level in &mut next.emotions {
        if pushed(&level.emotion) {
            continue;
        }
        let rest = i32::from(temperament.rest_of(&level.emotion).min(MAXIMUM_LEVEL));
        let now = i32::from(level.level);
        level.level = level_from(now + fade_step(rest - now, decay));
    }
    let reactivity = i32::from(
        temperament
            .reactivity
            .unwrap_or(NEUTRAL_REACTIVITY)
            .min(MAXIMUM_LEVEL),
    );
    for change in &appraisal.emotions {
        match next
            .emotions
            .iter_mut()
            .find(|seen| seen.emotion == change.emotion)
        {
            Some(level) => {
                let scaled = (i32::from(change.delta) * reactivity
                    + i32::from(NEUTRAL_REACTIVITY) / 2 * i32::from(change.delta).signum())
                    / i32::from(NEUTRAL_REACTIVITY);
                level.level = level_from(i32::from(level.level) + scaled);
            }
            None => dropped.emotions += 1,
        }
    }
    if let (Some(toward), Some(delta)) = (appraisal.toward.as_deref(), appraisal.rapport_delta) {
        take_rapport(config, &mut next, toward, delta, &mut dropped);
    }
    next.attention = take_attention(
        capacity_of(config, temperament),
        &state.attention,
        &appraisal.attention,
        &mut dropped,
    );
    next.mood = derive_mood(config, &next.emotions);
    next.turn = state.turn.saturating_add(1);
    (next, dropped)
}

/// Sets `emotion` to `level` in `state` and derives the mood again. False, and nothing changed,
/// when the emotion is not on the list.
pub fn set_emotion(
    config: &EmotionConfig,
    temperament: &Temperament,
    state: &mut PersonaState,
    emotion: &str,
    level: u8,
) -> bool {
    if !config.knows(emotion) {
        return false;
    }
    let mut conformed = conform(config, temperament, state);
    if let Some(slot) = conformed
        .emotions
        .iter_mut()
        .find(|seen| seen.emotion == emotion)
    {
        slot.level = level.min(MAXIMUM_LEVEL);
    }
    conformed.mood = derive_mood(config, &conformed.emotions);
    *state = conformed;
    true
}

/// The emotional triggers of `persona` whose state condition holds in `state`. A trigger with
/// only an event never appears: a model judges those.
pub fn state_triggers<'a>(persona: &'a Persona, state: &PersonaState) -> Vec<&'a EmotionalTrigger> {
    persona
        .emotional_triggers()
        .iter()
        .filter(|trigger| {
            trigger
                .when_state
                .as_ref()
                .is_some_and(|condition| state.level(&condition.emotion) >= condition.at_least)
        })
        .collect()
}

fn out_of_bounds(field: PersonaField, value: i32, minimum: i32, maximum: i32) -> PersonaError {
    PersonaError::OutOfRange {
        field,
        value,
        minimum,
        maximum,
    }
}

fn check_level(field: PersonaField, value: u8) -> Result<(), PersonaError> {
    if value > MAXIMUM_LEVEL {
        return Err(out_of_bounds(
            field,
            i32::from(value),
            0,
            i32::from(MAXIMUM_LEVEL),
        ));
    }
    Ok(())
}

fn check_label(field: PersonaField, id: &str) -> Result<(), PersonaError> {
    if is_label(id) {
        Ok(())
    } else {
        Err(PersonaError::InvalidItemId {
            field,
            id: id.chars().take(super::MAXIMUM_ID_CHARS + 1).collect(),
        })
    }
}

fn check_temperament(temperament: &Temperament) -> Result<(), PersonaError> {
    let field = PersonaField::Temperament;
    if temperament.baseline.len() > MAXIMUM_LIST_ENTRIES {
        return Err(PersonaError::TooMany {
            field,
            entries: temperament.baseline.len(),
            maximum: MAXIMUM_LIST_ENTRIES,
        });
    }
    let mut seen = HashSet::new();
    for rest in &temperament.baseline {
        check_label(field, &rest.emotion)?;
        check_level(field, rest.level)?;
        if !seen.insert(rest.emotion.as_str()) {
            return Err(PersonaError::DuplicateItemId {
                field,
                id: rest.emotion.clone(),
            });
        }
    }
    for value in [
        temperament.reactivity,
        temperament.patience,
        temperament.warmth,
        temperament.pride,
        temperament.decay_percent,
        temperament.distractibility,
    ]
    .into_iter()
    .flatten()
    {
        check_level(field, value)?;
    }
    if let Some(capacity) = temperament.attention_capacity
        && !(1..=MAXIMUM_ATTENTION_CAPACITY).contains(&capacity)
    {
        return Err(out_of_bounds(
            field,
            i32::from(capacity),
            1,
            i32::from(MAXIMUM_ATTENTION_CAPACITY),
        ));
    }
    Ok(())
}

fn check_emotional_triggers(
    triggers: &[EmotionalTrigger],
    known_behaviors: &HashSet<&str>,
) -> Result<(), PersonaError> {
    let field = PersonaField::EmotionalTriggers;
    if triggers.len() > MAXIMUM_LIST_ENTRIES {
        return Err(PersonaError::TooMany {
            field,
            entries: triggers.len(),
            maximum: MAXIMUM_LIST_ENTRIES,
        });
    }
    let mut seen = HashSet::new();
    for trigger in triggers {
        check_label(field, &trigger.id)?;
        if !seen.insert(trigger.id.as_str()) {
            return Err(PersonaError::DuplicateItemId {
                field,
                id: trigger.id.clone(),
            });
        }
        if let Some(condition) = &trigger.when_state {
            check_label(field, &condition.emotion)?;
            check_level(field, condition.at_least)?;
        }
        if let Some(event) = &trigger.when_event {
            check_line(field, event, MAXIMUM_ENTRY_CHARS, true)?;
        }
        for change in &trigger.sets {
            check_label(field, &change.emotion)?;
            if !(-100..=100).contains(&change.delta) {
                return Err(out_of_bounds(field, i32::from(change.delta), -100, 100));
            }
        }
        if let Some(behavior) = &trigger.behavior
            && !known_behaviors.contains(behavior.as_str())
        {
            return Err(PersonaError::UnknownBehavior {
                behavior: behavior.clone(),
                used_by: super::BehaviorUse::Trigger {
                    id: trigger.id.clone(),
                },
            });
        }
    }
    Ok(())
}

/// That the temperament and the emotional triggers are well formed, and every behavior a trigger
/// names is one of `known_behaviors`. Emotion ids are checked for their shape only; the list is
/// data ([`check_emotion_ids`]).
pub(super) fn check_emotional(
    temperament: &Temperament,
    triggers: &[EmotionalTrigger],
    known_behaviors: &HashSet<&str>,
) -> Result<(), PersonaError> {
    check_temperament(temperament)?;
    check_emotional_triggers(triggers, known_behaviors)
}

/// That every emotion the persona's temperament and emotional triggers name is on the list.
pub fn check_emotion_ids(persona: &Persona, config: &EmotionConfig) -> Result<(), EditError> {
    let unknown = |emotion: &str| EditError::UnknownEmotion {
        emotion: emotion.to_owned(),
    };
    let temperament = persona.temperament();
    let triggers = persona.emotional_triggers();
    let named = temperament
        .baseline
        .iter()
        .map(|rest| rest.emotion.as_str())
        .chain(triggers.iter().flat_map(|trigger| {
            trigger
                .when_state
                .iter()
                .map(|condition| condition.emotion.as_str())
                .chain(trigger.sets.iter().map(|change| change.emotion.as_str()))
        }));
    match named.into_iter().find(|emotion| !config.knows(emotion)) {
        Some(emotion) => Err(unknown(emotion)),
        None => Ok(()),
    }
}
