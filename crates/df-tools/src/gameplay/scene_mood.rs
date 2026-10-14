//! The mood of the scene on the shared display, derived from server story state only.
//!
//! This module is pure: the same inputs and the same explicit clock give the same answer, and it
//! reads no wall clock, no file and no network. It never reads what a player typed or said and
//! never infers a player's feelings: it reads the story's own record of the exchange (how the
//! last line landed socially, how each present character feels toward the hero, the tempo
//! phase, the setting) and decides how the characters and the camera should look. The image
//! service turns the answer into pictures; the picture is cosmetic and never changes an
//! outcome, a roll or a timing.
//!
//! # Hysteresis
//!
//! * A social exchange is a strong signal and changes the look at once; the look is then held
//!   for [`Hysteresis::exchange_hold`] so tempo drift cannot undo it.
//! * An ambient change (tempo, setting, recent table kinds) must persist for
//!   [`Hysteresis::settle`] before it becomes the target, so a flickering input never flaps.
//! * A character's expression follows the last exchange with them and relaxes to neutral after
//!   [`Hysteresis::expression_hold`] without another.
//! * "Worth a re-render" means the target differs from what is on screen: the scene look
//!   (mood, shot) or a character's expression.

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use super::{
    adventure::dossier::{Attitude, Effect},
    social::{Approach, EmotionLevel, EmotionalState},
};

/// How the room feels as a whole.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Mood {
    Calm,
    Warm,
    Tense,
    Menacing,
    Playful,
    Flirtatious,
    /// Charged, sensual and adult in tone. Rendered as `Flirtatious` under the restrained rating.
    Seductive,
    Grim,
    /// An interrogation that has turned cruel. Rendered as `Menacing` under the restrained rating.
    Brutal,
    Triumphant,
    Fearful,
}

impl Mood {
    #[cfg(test)]
    pub(super) const ALL: [Self; 11] = [
        Self::Calm,
        Self::Warm,
        Self::Tense,
        Self::Menacing,
        Self::Playful,
        Self::Flirtatious,
        Self::Seductive,
        Self::Grim,
        Self::Brutal,
        Self::Triumphant,
        Self::Fearful,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Calm => "calm",
            Self::Warm => "warm",
            Self::Tense => "tense",
            Self::Menacing => "menacing",
            Self::Playful => "playful",
            Self::Flirtatious => "flirtatious",
            Self::Seductive => "seductive",
            Self::Grim => "grim",
            Self::Brutal => "brutal",
            Self::Triumphant => "triumphant",
            Self::Fearful => "fearful",
        }
    }
}

/// Where the camera stands, relative to where the cast stands.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Shot {
    WideEstablishing,
    MediumConversation,
    CloseOnSpeaker,
    OverShoulderTowardSpeaker,
    LowAngle,
    HighAngle,
}

impl Shot {
    #[cfg(test)]
    pub(super) const ALL: [Self; 6] = [
        Self::WideEstablishing,
        Self::MediumConversation,
        Self::CloseOnSpeaker,
        Self::OverShoulderTowardSpeaker,
        Self::LowAngle,
        Self::HighAngle,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::WideEstablishing => "wide_establishing",
            Self::MediumConversation => "medium_two_shot",
            Self::CloseOnSpeaker => "close_on_speaker",
            Self::OverShoulderTowardSpeaker => "over_shoulder",
            Self::LowAngle => "low_angle",
            Self::HighAngle => "high_angle",
        }
    }
}

/// A shift of the light relative to the reference painting.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Lighting {
    Unchanged,
    Warmer,
    Colder,
    HarderShadows,
    Flicker,
}

impl Lighting {
    #[cfg(test)]
    pub(super) const ALL: [Self; 5] = [
        Self::Unchanged,
        Self::Warmer,
        Self::Colder,
        Self::HarderShadows,
        Self::Flicker,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::Warmer => "warmer",
            Self::Colder => "colder",
            Self::HarderShadows => "harder_shadows",
            Self::Flicker => "flicker",
        }
    }
}

/// What a character's face shows.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Expression {
    Neutral,
    Smiling,
    Amused,
    Flirtatious,
    /// Sensual and adult in tone. Rendered as `Flirtatious` under the restrained rating, and
    /// never rendered at all for a character whose art derives from a real person's likeness.
    Seductive,
    Angry,
    Contemptuous,
    Afraid,
    Pained,
    Sorrowful,
}

impl Expression {
    #[cfg(test)]
    pub(super) const ALL: [Self; 10] = [
        Self::Neutral,
        Self::Smiling,
        Self::Amused,
        Self::Flirtatious,
        Self::Seductive,
        Self::Angry,
        Self::Contemptuous,
        Self::Afraid,
        Self::Pained,
        Self::Sorrowful,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Smiling => "smiling",
            Self::Amused => "amused",
            Self::Flirtatious => "flirtatious",
            Self::Seductive => "seductive",
            Self::Angry => "angry",
            Self::Contemptuous => "contemptuous",
            Self::Afraid => "afraid",
            Self::Pained => "pained",
            Self::Sorrowful => "sorrowful",
        }
    }
}

pub(super) use crate::health_tier::HealthTier;

/// What a character shows: a mood face or a health face, never both. [`Self::AUTHORED`] is the
/// authored cut-out itself, which costs nothing to show.
///
/// Priority, not multiplication: at Bloodied or worse the health face wins and the expression is
/// neutral; above that the conversation's mood face wins and the tier is healthy; a hurt
/// character with no mood shows the hurt face.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct Face {
    pub(super) expression: Expression,
    pub(super) tier: HealthTier,
}

impl Face {
    pub(super) const AUTHORED: Self = Self {
        expression: Expression::Neutral,
        tier: HealthTier::Healthy,
    };

    /// The face of a character whose conversation shows `mood` and whose health is `tier`.
    pub(super) fn resolve(mood: Expression, tier: HealthTier) -> Self {
        match tier {
            HealthTier::Bloodied | HealthTier::Critical | HealthTier::Down => Self {
                expression: Expression::Neutral,
                tier,
            },
            HealthTier::Hurt if mood == Expression::Neutral => Self {
                expression: Expression::Neutral,
                tier,
            },
            HealthTier::Hurt | HealthTier::Healthy => Self {
                expression: mood,
                tier: HealthTier::Healthy,
            },
        }
    }
}

/// The scene as the camera frames it: the part of the answer that re-renders the backdrop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Look {
    pub(super) mood: Mood,
    pub(super) shot: Shot,
    pub(super) lighting: Lighting,
}

impl Look {
    /// The painting as authored.
    pub(super) const AUTHORED: Self = Self {
        mood: Mood::Calm,
        shot: Shot::WideEstablishing,
        lighting: Lighting::Unchanged,
    };

    pub(super) fn of(mood: Mood) -> Self {
        let (shot, lighting) = match mood {
            Mood::Calm => (Shot::WideEstablishing, Lighting::Unchanged),
            Mood::Warm => (Shot::MediumConversation, Lighting::Warmer),
            Mood::Tense => (Shot::OverShoulderTowardSpeaker, Lighting::Colder),
            Mood::Menacing => (Shot::LowAngle, Lighting::HarderShadows),
            Mood::Playful => (Shot::MediumConversation, Lighting::Warmer),
            Mood::Flirtatious => (Shot::CloseOnSpeaker, Lighting::Warmer),
            Mood::Seductive => (Shot::CloseOnSpeaker, Lighting::Flicker),
            Mood::Grim => (Shot::HighAngle, Lighting::Colder),
            Mood::Brutal => (Shot::LowAngle, Lighting::HarderShadows),
            Mood::Triumphant => (Shot::LowAngle, Lighting::Warmer),
            Mood::Fearful => (Shot::HighAngle, Lighting::Flicker),
        };
        Self {
            mood,
            shot,
            lighting,
        }
    }

    /// Worth a repaint: the mood or the shot differs (the lighting follows the mood).
    fn differs_from(self, other: Self) -> bool {
        self.mood != other.mood || self.shot != other.shot
    }
}

/// The tempo phase of the story, as the display's tempo cue reports it.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum TempoLevel {
    Calm,
    Build,
    Anticipation,
    Peak,
    Release,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Tempo {
    pub(super) phase: TempoLevel,
    pub(super) intensity_permille: u16,
}

/// What the setting itself suggests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SceneTone {
    Neutral,
    /// A hearth, a tea house, a shop: the room leans warm.
    Cozy,
    /// A graveyard gate, a cellar, a study under the chapel: the room leans grim.
    Grim,
}

/// Where the story stands, from the scene the pack selected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Beat {
    Conversation,
    Confrontation,
    Victory,
    Defeat,
}

/// One character on stage and how the story stands with them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CastState {
    /// The stable id of the character in the scene's cast.
    pub(super) id: String,
    pub(super) attitude: Option<Attitude>,
    /// An antagonist-side character: it can resist a threat and never warms to talk alone.
    pub(super) opposes: bool,
    pub(super) speaking: bool,
    pub(super) tier: HealthTier,
}

/// How the last typed line landed, as the story's social layer recorded it. Carries no words.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ExchangeSignal {
    /// Grows with every recorded exchange; a signal is handled once.
    pub(super) sequence: u64,
    pub(super) npc_id: String,
    pub(super) approach: Approach,
    pub(super) effect: Effect,
    /// The NPC's attitude toward the hero after the exchange.
    pub(super) attitude: Attitude,
    pub(super) opposes: bool,
    pub(super) emotional_state: EmotionalState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TableKind {
    Speech,
    Action,
}

/// Everything the mood reads for one decision.
#[derive(Clone, Copy, Debug)]
pub(super) struct MoodInputs<'a> {
    pub(super) tempo: Option<Tempo>,
    pub(super) tone: SceneTone,
    pub(super) beat: Beat,
    pub(super) cast: &'a [CastState],
    pub(super) exchange: Option<&'a ExchangeSignal>,
    /// The kinds (never the text) of the latest table lines, oldest first.
    pub(super) recent_kinds: &'a [TableKind],
}

/// The timing of the hysteresis.
#[derive(Clone, Copy, Debug)]
pub(super) struct Hysteresis {
    pub(super) exchange_hold: Duration,
    pub(super) settle: Duration,
    pub(super) expression_hold: Duration,
}

impl Default for Hysteresis {
    fn default() -> Self {
        Self {
            exchange_hold: Duration::from_secs(45),
            settle: Duration::from_secs(10),
            expression_hold: Duration::from_secs(90),
        }
    }
}

/// How many recent approaches the mood remembers.
const RECENT_APPROACHES: usize = 3;

/// The mood the story's own tempo, setting and table suggest with no fresh exchange.
pub(super) fn ambient_mood(inputs: &MoodInputs<'_>) -> Mood {
    match inputs.beat {
        Beat::Victory => return Mood::Triumphant,
        Beat::Defeat => return Mood::Grim,
        Beat::Conversation | Beat::Confrontation => {}
    }
    let phase = inputs.tempo.map_or(TempoLevel::Calm, |tempo| tempo.phase);
    let actions = inputs
        .recent_kinds
        .iter()
        .filter(|kind| **kind == TableKind::Action)
        .count();
    let opposing_speaker = inputs
        .cast
        .iter()
        .any(|member| member.speaking && member.opposes);
    let base = match phase {
        TempoLevel::Calm => Mood::Calm,
        TempoLevel::Build if actions >= 3 => Mood::Tense,
        TempoLevel::Build => Mood::Calm,
        TempoLevel::Anticipation => Mood::Tense,
        TempoLevel::Peak if opposing_speaker => Mood::Menacing,
        TempoLevel::Peak => Mood::Tense,
        TempoLevel::Release => Mood::Warm,
    };
    match (base, inputs.tone) {
        (Mood::Calm, SceneTone::Grim) => Mood::Grim,
        (Mood::Calm, SceneTone::Cozy) => Mood::Warm,
        (other, _) => other,
    }
}

/// The scene mood and the speaker's expression one exchange gives. `recent` holds the approaches
/// of earlier exchanges (not this one), oldest first.
pub(super) fn exchange_reading(signal: &ExchangeSignal, recent: &[Approach]) -> (Mood, Expression) {
    if signal.emotional_state.afraid != EmotionLevel::None {
        return (Mood::Fearful, Expression::Afraid);
    }
    if signal.emotional_state.never_forgives.is_some()
        || signal.emotional_state.resentful == EmotionLevel::Much
    {
        return (Mood::Menacing, Expression::Angry);
    }
    if signal.emotional_state.drawn_to != EmotionLevel::None {
        return (Mood::Flirtatious, Expression::Flirtatious);
    }
    if signal.emotional_state.fond != EmotionLevel::None {
        return (Mood::Warm, Expression::Smiling);
    }
    if signal.emotional_state.wary != EmotionLevel::None {
        return (Mood::Tense, Expression::Neutral);
    }
    let again = recent
        .iter()
        .filter(|earlier| **earlier == signal.approach)
        .count()
        >= 1;
    let friendly = signal.attitude == Attitude::Friendly;
    match signal.approach {
        Approach::Insult => (
            if signal.attitude == Attitude::Hostile {
                Mood::Menacing
            } else {
                Mood::Tense
            },
            if friendly {
                Expression::Pained
            } else {
                Expression::Angry
            },
        ),
        Approach::Intimidation => match signal.effect {
            Effect::Warm => (
                if again { Mood::Brutal } else { Mood::Fearful },
                Expression::Afraid,
            ),
            Effect::Cool => (
                if again { Mood::Brutal } else { Mood::Menacing },
                Expression::Angry,
            ),
            Effect::Steady => (
                if again { Mood::Brutal } else { Mood::Menacing },
                if signal.opposes {
                    Expression::Contemptuous
                } else {
                    Expression::Afraid
                },
            ),
        },
        Approach::Flirting => match signal.effect {
            Effect::Warm if friendly => (
                if again {
                    Mood::Seductive
                } else {
                    Mood::Flirtatious
                },
                if again {
                    Expression::Seductive
                } else {
                    Expression::Flirtatious
                },
            ),
            Effect::Warm | Effect::Steady => (Mood::Playful, Expression::Amused),
            Effect::Cool => (Mood::Tense, Expression::Contemptuous),
        },
        Approach::Flattery => match signal.effect {
            Effect::Warm => (Mood::Warm, Expression::Smiling),
            Effect::Steady => (Mood::Playful, Expression::Amused),
            Effect::Cool => (Mood::Tense, Expression::Contemptuous),
        },
        Approach::Kindness | Approach::Apology => (
            Mood::Warm,
            if signal.attitude == Attitude::Hostile {
                Expression::Neutral
            } else {
                Expression::Smiling
            },
        ),
        Approach::Bribe => match signal.effect {
            Effect::Warm => (Mood::Tense, Expression::Amused),
            Effect::Steady => (Mood::Tense, Expression::Neutral),
            Effect::Cool => (Mood::Tense, Expression::Angry),
        },
        Approach::Deception => match signal.effect {
            Effect::Cool => (Mood::Tense, Expression::Contemptuous),
            Effect::Warm | Effect::Steady => (Mood::Tense, Expression::Neutral),
        },
        Approach::Question
        | Approach::Introduction
        | Approach::Refusal
        | Approach::Persuasion
        | Approach::Bargain
        | Approach::Other => (
            Mood::Calm,
            match signal.attitude {
                Attitude::Friendly => Expression::Smiling,
                Attitude::Indifferent => Expression::Neutral,
                Attitude::Hostile => Expression::Contemptuous,
            },
        ),
    }
}

/// What the speaking character's face shows at the story's end beats with no exchange to say
/// otherwise: the patron's relief or grief, the enemy's pain or scorn.
fn beat_expression(beat: Beat, member: &CastState) -> Expression {
    if !member.speaking {
        return Expression::Neutral;
    }
    match (beat, member.opposes) {
        (Beat::Victory, true) => Expression::Pained,
        (Beat::Victory, false) => Expression::Smiling,
        (Beat::Defeat, true) => Expression::Contemptuous,
        (Beat::Defeat, false) => Expression::Sorrowful,
        (Beat::Conversation | Beat::Confrontation, _) => Expression::Neutral,
    }
}

#[derive(Clone, Copy, Debug)]
struct Held {
    expression: Expression,
    set_at: Duration,
}

/// What the tracker decided for one update.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Verdict {
    /// The look the scene should have.
    pub(super) look: Look,
    /// The look differs from what is on screen in mood or shot.
    pub(super) scene_worth_rendering: bool,
    /// Characters whose face should differ from what is on screen, by cast id.
    pub(super) face_changes: Vec<(String, Face)>,
}

/// The mood of one scene over time. [`Self::reset_scene`] starts it afresh when the scene changes.
#[derive(Debug)]
pub(super) struct MoodTracker {
    timing: Hysteresis,
    target: Look,
    shown: Look,
    candidate: Option<(Mood, Duration)>,
    exchange_until: Duration,
    handled_sequence: u64,
    recent: VecDeque<Approach>,
    held: BTreeMap<String, Held>,
    shown_faces: BTreeMap<String, Face>,
}

impl MoodTracker {
    pub(super) fn new(timing: Hysteresis) -> Self {
        Self {
            timing,
            target: Look::AUTHORED,
            shown: Look::AUTHORED,
            candidate: None,
            exchange_until: Duration::ZERO,
            handled_sequence: 0,
            recent: VecDeque::new(),
            held: BTreeMap::new(),
            shown_faces: BTreeMap::new(),
        }
    }

    /// A new scene shows its authored painting and neutral faces. `handled_sequence` is kept so
    /// an exchange of the previous scene is never replayed.
    pub(super) fn reset_scene(&mut self) {
        let handled_sequence = self.handled_sequence;
        *self = Self::new(self.timing);
        self.handled_sequence = handled_sequence;
    }

    pub(super) fn shown_look(&self) -> Look {
        self.shown
    }

    pub(super) fn mark_scene_shown(&mut self, look: Look) {
        self.shown = look;
    }

    pub(super) fn mark_face_shown(&mut self, id: &str, face: Face) {
        self.shown_faces.insert(id.to_owned(), face);
    }

    /// The newest exchange sequence this tracker has handled.
    pub(super) fn handled_sequence(&self) -> u64 {
        self.handled_sequence
    }

    pub(super) fn update(&mut self, now: Duration, inputs: &MoodInputs<'_>) -> Verdict {
        if let Some(signal) = inputs
            .exchange
            .filter(|signal| signal.sequence > self.handled_sequence)
        {
            let recent: Vec<Approach> = self.recent.iter().copied().collect();
            let (mood, expression) = exchange_reading(signal, &recent);
            self.handled_sequence = signal.sequence;
            self.recent.push_back(signal.approach);
            while self.recent.len() > RECENT_APPROACHES {
                self.recent.pop_front();
            }
            self.target = Look::of(mood);
            self.candidate = None;
            self.exchange_until = now + self.timing.exchange_hold;
            self.held.insert(
                signal.npc_id.clone(),
                Held {
                    expression,
                    set_at: now,
                },
            );
        } else if now >= self.exchange_until {
            self.follow_ambient(now, inputs);
        }
        for held in self.held.values_mut() {
            if now.saturating_sub(held.set_at) >= self.timing.expression_hold {
                *held = Held {
                    expression: Expression::Neutral,
                    set_at: now,
                };
            }
        }
        let face_changes = inputs
            .cast
            .iter()
            .filter_map(|member| {
                let held = self
                    .held
                    .get(&member.id)
                    .map_or(Expression::Neutral, |held| held.expression);
                let mood = if held != Expression::Neutral {
                    held
                } else {
                    beat_expression(inputs.beat, member)
                };
                let wanted = Face::resolve(mood, member.tier);
                let shown = self
                    .shown_faces
                    .get(&member.id)
                    .copied()
                    .unwrap_or(Face::AUTHORED);
                (wanted != shown).then(|| (member.id.clone(), wanted))
            })
            .collect();
        Verdict {
            look: self.target,
            scene_worth_rendering: self.target.differs_from(self.shown),
            face_changes,
        }
    }

    /// An ambient mood replaces the target only after it has persisted for the settle time.
    fn follow_ambient(&mut self, now: Duration, inputs: &MoodInputs<'_>) {
        let ambient = ambient_mood(inputs);
        if ambient == self.target.mood {
            self.candidate = None;
            return;
        }
        match self.candidate {
            Some((mood, since)) if mood == ambient => {
                if now.saturating_sub(since) >= self.timing.settle {
                    self.target = Look::of(ambient);
                    self.candidate = None;
                }
            }
            _ => self.candidate = Some((ambient, now)),
        }
    }
}

#[cfg(test)]
#[path = "scene_mood_tests.rs"]
mod tests;
