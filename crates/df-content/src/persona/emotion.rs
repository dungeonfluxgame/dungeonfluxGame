//! The emotional layer of a persona: temperament (stable), emotion, mood and attention (live,
//! short-term) and attitude (live, per other character). Frozen shared contract of
//! `planning/persona-emotion.md`: the stages that store, apply and prompt with these read and write
//! only these types.
//!
//! No text is matched to find a feeling. A model appraises what was said and returns `Appraisal`;
//! code validates it against the closed emotion list, clamps it by the temperament and keeps the
//! state. Emotion and temperament are presentation and wording aids; only attitude, through the
//! rules, can change a number.

use serde::{Deserialize, Serialize};

/// The emotions a persona can feel, by id. The list is data (`persona/emotions.json` next to the
/// personas, with this one as the built-in default), so a campaign can add or drop one; code
/// validates every id against it.
pub const DEFAULT_EMOTIONS: [&str; 8] = [
    "joy",
    "trust",
    "irritation",
    "fear",
    "sadness",
    "surprise",
    "pride",
    "amusement",
];

/// One emotion at an intensity, 0 (absent) to 100 (overwhelming).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmotionLevel {
    pub emotion: String,
    pub level: u8,
}

/// The stable side of a persona: where its feelings rest and how it reacts. Every field is
/// optional in a file (a persona without a temperament is steady and even), values are 0 to 100.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Temperament {
    /// The levels its emotions fade back to; an emotion not listed rests at 0.
    pub baseline: Vec<EmotionLevel>,
    /// How strongly an event moves it: a change is scaled by this over 50 (50 is a normal person).
    pub reactivity: Option<u8>,
    /// How much it puts up with before irritation builds.
    pub patience: Option<u8>,
    pub warmth: Option<u8>,
    pub pride: Option<u8>,
    /// How much of the gap to the baseline closes each turn, in percent.
    pub decay_percent: Option<u8>,
    /// How many recent lines it can attend to at once, 1 to 5.
    pub attention_capacity: Option<u8>,
    /// How easily its attention is pulled to whatever was said last, 0 (steady) to 100.
    pub distractibility: Option<u8>,
}

/// How a character stands toward another, on the 2024 rules' attitude scale. The server owns the
/// live value; the rules own what it changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attitude {
    Friendly,
    Indifferent,
    Hostile,
}

/// A character's standing toward one other character: the attitude and a finer rapport from -100
/// to 100 that moves the attitude when it crosses a threshold the content sets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relationship {
    pub toward: String,
    pub attitude: Attitude,
    pub rapport: i8,
}

/// One recent line a character registered: who said it and what it amounted to, in a few words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Noticed {
    pub speaker: String,
    pub gist: String,
}

/// What a character is attending to: the character and topic in focus, and the recent lines it
/// registered (at most its capacity). A line answers what is in focus first.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Attention {
    pub focus: Option<String>,
    pub topic: Option<String>,
    pub noticed: Vec<Noticed>,
}

/// The overall colour of a state, derived from the emotions by the content's mapping: valence
/// from -100 (sour) to 100 (glad), arousal 0 (calm) to 100 (agitated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Mood {
    pub valence: i8,
    pub arousal: u8,
}

/// The live emotional state of one persona, kept by the host.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PersonaState {
    pub id: String,
    /// How many lines the character has taken part in; decay counts in turns.
    pub turn: u32,
    pub emotions: Vec<EmotionLevel>,
    pub mood: Mood,
    pub attention: Attention,
    pub relationships: Vec<Relationship>,
}

/// A change to one emotion, in points (-100 to 100), before the temperament scales it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmotionDelta {
    pub emotion: String,
    pub delta: i8,
}

/// How a model says an event moved one character, returned in the same call that words their line.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Appraisal {
    pub emotions: Vec<EmotionDelta>,
    /// The character this appraisal changes their standing toward, and by how much.
    pub toward: Option<String>,
    pub rapport_delta: Option<i8>,
    pub attention: Attention,
    /// How the line should read, as a bubble or voice tone id the content defines.
    pub tone: Option<String>,
}

/// A trigger that works on feeling. `when_state` is checked by code against the live levels (no
/// text); `when_event` is a statement a model judges, like any trigger. Either may `set` deltas and
/// switch the character to a behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmotionalTrigger {
    pub id: String,
    #[serde(default)]
    pub when_state: Option<StateCondition>,
    #[serde(default)]
    pub when_event: Option<String>,
    #[serde(default)]
    pub sets: Vec<EmotionDelta>,
    #[serde(default)]
    pub behavior: Option<String>,
}

/// A numeric condition on the live state: this emotion at or above a level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateCondition {
    pub emotion: String,
    pub at_least: u8,
}
