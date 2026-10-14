//! Image prompts, assembled only from a closed vocabulary the server owns and vetted facts.
//!
//! Nothing a player typed or said, and nothing a model wrote, ever reaches a prompt: mood,
//! shot, lighting, expression and tier are enums that render to fixed phrases, and the
//! only free text is a [`VettedFact`] from the adventure record (a place name) or an
//! [`ArtifactSubject`] that came from the scene's own facts or passed the strict filter below.
//! This is about injection and consistency, not censorship: the phrases themselves depend on
//! the server's content rating.
//!
//! Limits that hold under both ratings: every person is an adult (the types have no younger
//! variant and a young character is described as a "young adult"), no real person's likeness
//! gets a seductive variant, and no real public figure or brand can be named.

use super::super::{
    pack,
    scene_mood::{Expression, HealthTier, Lighting, Look, Mood, Shot},
    startup_configuration::ContentRating,
};

const MAXIMUM_FACT_CHARACTERS: usize = 80;

pub(in crate::gameplay) const FILM_LOOK: &str = "Live-action fantasy film; real camera, one consistent \
    prime lens per scene; natural depth of field. Practical light sources visible in scene. Skin \
    pores, worn woven fabric and seams. One named film grade for each place and time of day \
    (Lantern Night, Overcast Day, Hearth Amber): cool shadows, warm practical highlights, fine \
    film grain. Photographic only; no painting, illustration, concept art, CGI or game art.";
const NO_TEXT: &str = "No text, letters, numbers, logos or watermarks.";
const NO_REAL_PEOPLE: &str = "No real people, celebrities or likenesses of real persons.";
const ADULTS_ONLY: &str = "Every person shown is an adult.";
const RESTRAINED_LIMITS: &str = "No gore, no nudity, no sexual content.";
const MATURE_STANCE: &str = "Mature content rating: violence, horror and desire are shown as \
    the story demands, not softened.";
const KEEP_PLACE: &str = "Keep the same place: its architecture, props, palette and perspective \
    exactly as in the reference photograph.";
pub(super) const NO_PEOPLE: &str =
    "Show no people, no figures and no silhouettes anywhere; the characters are added separately.";
const KEEP_FLOOR: &str = "Keep a wide, clear, open floor across the lower third of the frame, \
    with the floor perspective and horizon of the reference, so figures can stand on it.";
const KEEP_FRAME: &str = "Keep the exact aspect ratio and framing of the reference.";
const KEEP_PERSON: &str = "Keep exactly the same person: face, hair, beard, build, costume, \
    pose, framing, camera distance and lighting as in the reference image.";
const KEEP_TRANSPARENT: &str = "Keep the background fully transparent.";
const CHANGE_ONLY: &str = "Change only the following.";
const RECAMERA: &str = "Re-shoot the reference photograph from a changed camera and light.";
const SCENE_SETTING: &str = "The place is";
const CHARACTER_IS: &str = "The character is";
const SHOW_EXPRESSION: &str = "The face shows";
const SHOW_APPEARANCE: &str = "The character appears";

/// Why a prompt could not be built. Nothing is sent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum PromptRefusal {
    /// A seductive variant of a character whose art derives from a real person's likeness.
    RealLikenessIntimate,
}

/// A piece of a prompt: a fixed phrase of the closed vocabulary, or a vetted fact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum Part {
    Fixed(&'static str),
    Fact(String),
}

/// A finished prompt. Only this module builds one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct AssembledPrompt {
    text: String,
    parts: Vec<Part>,
}

impl AssembledPrompt {
    pub(super) fn from_parts(parts: Vec<Part>) -> Self {
        let text = parts
            .iter()
            .map(|part| match part {
                Part::Fixed(phrase) => *phrase,
                Part::Fact(fact) => fact.as_str(),
            })
            .collect::<Vec<_>>()
            .join(" ");
        Self { text, parts }
    }

    pub(in crate::gameplay) fn as_str(&self) -> &str {
        &self.text
    }

    /// Only the authenticated, gated development owner may supply this bounded direction.
    /// Append it after the scene description; retain the original identity and rating clauses.
    pub(in crate::gameplay) fn with_development_direction(mut self, direction: &str) -> Self {
        self.text.push_str("\nDevelopment picture direction (change the action only; preserve the scene, cast identities, faces, art style and content rating):\n");
        self.text.push_str(direction);
        self
    }

    #[cfg(test)]
    pub(in crate::gameplay) fn parts(&self) -> &[Part] {
        &self.parts
    }
}

/// Words that never appear in a vetted fact or an artifact subject: instructions to the model,
/// anyone younger than an adult, and brands. Matched as whole lowercase words.
const DENIED_WORDS: &[&str] = &[
    "ignore",
    "instruction",
    "instructions",
    "prompt",
    "system",
    "assistant",
    "developer",
    "override",
    "jailbreak",
    "disregard",
    "pretend",
    "roleplay",
    "child",
    "children",
    "kid",
    "kids",
    "boy",
    "girl",
    "teen",
    "teens",
    "teenager",
    "minor",
    "underage",
    "baby",
    "toddler",
    "infant",
    "schoolgirl",
    "schoolboy",
    "loli",
    "shota",
    "youngster",
    "nike",
    "adidas",
    "disney",
    "marvel",
    "pokemon",
    "nintendo",
    "sony",
    "apple",
    "google",
    "microsoft",
    "amazon",
    "coca",
    "cola",
    "pepsi",
    "mcdonalds",
    "starbucks",
    "lego",
    "ikea",
    "tesla",
    "facebook",
    "instagram",
    "tiktok",
    "youtube",
    "netflix",
    "samsung",
    "gucci",
    "prada",
    "rolex",
    "harley",
    "toyota",
    "bmw",
    "mercedes",
    "ferrari",
    "barbie",
    "hasbro",
    "mattel",
    "trump",
    "biden",
    "obama",
    "putin",
];

pub(super) fn denied(word: &str) -> bool {
    DENIED_WORDS.contains(&word)
}

/// A short text from the adventure record, such as a place name. Plain words only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct VettedFact(String);

impl VettedFact {
    /// `None` unless the text is 2 to 80 plain characters (letters, digits, spaces and
    /// `' - , .`), with whitespace collapsed and none of the [`DENIED_WORDS`].
    pub(in crate::gameplay) fn new(text: &str) -> Option<Self> {
        let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let length = collapsed.chars().count();
        if !(2..=MAXIMUM_FACT_CHARACTERS).contains(&length)
            || !collapsed.chars().all(|character| {
                character.is_ascii_alphanumeric()
                    || matches!(character, ' ' | '\'' | '-' | ',' | '.')
            })
        {
            return None;
        }
        let lowered = collapsed.to_ascii_lowercase();
        let has_denied_word = lowered
            .split(|character: char| !character.is_ascii_alphabetic())
            .any(denied);
        (!has_denied_word).then_some(Self(collapsed))
    }

    pub(in crate::gameplay) fn as_str(&self) -> &str {
        &self.0
    }
}

/// The place a scene happens in, from the adventure's setting record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct PlaceFacts {
    pub(in crate::gameplay) name: VettedFact,
    pub(in crate::gameplay) short: Option<VettedFact>,
}

impl PlaceFacts {
    /// The place's facts, or `None` when its name does not pass vetting. A short description
    /// that does not pass is left out.
    pub(in crate::gameplay) fn new(name: &str, short: &str) -> Option<Self> {
        Some(Self {
            name: VettedFact::new(name)?,
            short: VettedFact::new(short),
        })
    }

    /// Words the scene itself uses; an artifact subject may use them too.
    pub(super) fn vocabulary(&self) -> Vec<String> {
        let mut words: Vec<String> = Vec::new();
        for fact in std::iter::once(&self.name).chain(self.short.iter()) {
            for word in fact
                .as_str()
                .to_ascii_lowercase()
                .split(|character: char| !character.is_ascii_alphabetic())
                .filter(|word| word.len() >= 3)
            {
                words.push(word.to_owned());
            }
        }
        words
    }
}

/// The rating-independent character facts a prompt uses. No name is ever used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct CharacterFacts {
    pub(in crate::gameplay) role: CharacterRole,
    pub(in crate::gameplay) life_stage: LifeStage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum CharacterRole {
    Patron,
    Antagonist,
    Contact,
    Bystander,
    /// One of the party's heroes.
    Hero,
}

impl CharacterRole {
    #[cfg(test)]
    pub(in crate::gameplay) const ALL: [Self; 5] = [
        Self::Patron,
        Self::Antagonist,
        Self::Contact,
        Self::Bystander,
        Self::Hero,
    ];

    fn phrase(self) -> &'static str {
        match self {
            Self::Patron => "someone who has hired the heroes, a patron.",
            Self::Antagonist => "an antagonist, an enemy of the heroes.",
            Self::Contact => "a helpful contact the heroes have met.",
            Self::Bystander => "a bystander at the scene.",
            Self::Hero => "one of the heroes of the party.",
        }
    }
}

/// How old a character looks. There is no variant younger than an adult.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum LifeStage {
    YoungAdult,
    Adult,
    Elder,
}

impl LifeStage {
    #[cfg(test)]
    pub(in crate::gameplay) const ALL: [Self; 3] = [Self::YoungAdult, Self::Adult, Self::Elder];

    /// An archetype of unknown age is an adult; "young" is a young adult.
    pub(in crate::gameplay) fn from_age(age: Option<pack::Age>) -> Self {
        match age {
            Some(pack::Age::Young) => Self::YoungAdult,
            Some(pack::Age::Elder) => Self::Elder,
            Some(pack::Age::Adult) | None => Self::Adult,
        }
    }

    fn phrase(self) -> &'static str {
        match self {
            Self::YoungAdult => "a young adult, clearly over eighteen.",
            Self::Adult => "a grown adult.",
            Self::Elder => "an elderly adult.",
        }
    }
}

fn mood_phrase(mood: Mood, rating: ContentRating) -> &'static str {
    let mature = rating == ContentRating::Mature;
    match mood {
        Mood::Calm => "a calm, quiet, settled mood",
        Mood::Warm => "a warm, welcoming, friendly mood",
        Mood::Tense => "a tense, wary mood, a held breath before something happens",
        Mood::Menacing => "a menacing mood with danger close at hand",
        Mood::Playful => "a playful, light-hearted mood",
        Mood::Flirtatious => "a charged, flirtatious mood",
        Mood::Seductive if mature => {
            "a sensual, seductive, adult mood: low intimate light and an air of closeness and desire"
        }
        Mood::Seductive => "a charged, flirtatious mood",
        Mood::Grim => "a grim, heavy, ominous mood",
        Mood::Brutal if mature => {
            "a brutal interrogation: the room dressed for cruelty with a lone chair, restraints, \
             tools laid out and dark stains, shown unflinchingly"
        }
        Mood::Brutal => {
            "a menacing, harsh interrogation mood with a lone chair and hard light, implied \
             rather than shown"
        }
        Mood::Triumphant => "a triumphant, victorious mood",
        Mood::Fearful => "a fearful, dread-filled mood with the shadows pressing in",
    }
}

fn shot_phrase(shot: Shot) -> &'static str {
    match shot {
        Shot::WideEstablishing => "a wide establishing view of the whole place",
        Shot::MediumConversation => {
            "a medium-distance view of the middle of the room, the open floor wide in the foreground"
        }
        Shot::CloseOnSpeaker => {
            "a closer, more intimate view, the camera nearer to the middle of the room"
        }
        Shot::OverShoulderTowardSpeaker => {
            "a view from low and slightly to one side, looking across the open floor toward the \
             far side of the room"
        }
        Shot::LowAngle => "a low camera angle looking up so the place looms over the viewer",
        Shot::HighAngle => "a high camera angle looking down on the place",
    }
}

fn lighting_phrase(lighting: Lighting) -> &'static str {
    match lighting {
        Lighting::Unchanged => "the same lighting as the reference",
        Lighting::Warmer => "warmer, amber light",
        Lighting::Colder => "colder, blue-grey light",
        Lighting::HarderShadows => "hard directional light with deep, sharp shadows",
        Lighting::Flicker => "flickering firelight and unsteady lantern glow",
    }
}

pub(super) fn expression_phrase(expression: Expression, rating: ContentRating) -> &'static str {
    let mature = rating == ContentRating::Mature;
    match expression {
        Expression::Neutral => "a calm, neutral expression",
        Expression::Smiling => "a warm, genuine smile",
        Expression::Amused => {
            "a broad, wry, amused grin: eyes crinkled, one brow raised, head tilted slightly"
        }
        Expression::Flirtatious => {
            "a flirtatious look: a playful half-smile, one raised brow, a slow lingering sideways glance and a slight head tilt"
        }
        Expression::Seductive if mature => {
            "a seductive, sultry look: heavy-lidded eyes, parted lips, a slight head tilt and an intent, inviting gaze"
        }
        Expression::Seductive => {
            "a flirtatious look: a playful half-smile, one raised brow, a slow lingering sideways glance and a slight head tilt"
        }
        Expression::Angry => {
            "furious anger: brows lowered and drawn together, eyes narrowed and hard, jaw \
             clenched, lips pulled tight over the teeth"
        }
        Expression::Contemptuous => {
            "cold contempt: a sneering curl of the lip and eyes looking down the nose"
        }
        Expression::Afraid => "fear: wide frightened eyes, raised brows and a tense, open mouth",
        Expression::Pained => "pain: a wince, eyes squeezed shut and teeth bared",
        Expression::Sorrowful => "deep sorrow: downcast, glistening eyes and a trembling mouth",
    }
}

/// What a hurt character's face shows. Closed vocabulary, the same under both ratings: pain and
/// fear are shown as they are.
pub(super) fn health_face_phrase(tier: HealthTier) -> Option<&'static str> {
    match tier {
        HealthTier::Healthy => None,
        HealthTier::Hurt => {
            Some("strain: a tight jaw, narrowed watchful eyes and a faint sheen of sweat, guarded")
        }
        HealthTier::Bloodied => Some("pain: a wince, clenched teeth and eyes narrowed against it"),
        HealthTier::Critical => {
            Some("desperation: ashen skin, wide unfocused eyes and a slack, trembling mouth")
        }
        HealthTier::Down => Some("unconscious: closed eyes, a slack face and the head slumped"),
    }
}

/// How a hurt character's body and gear look; this is where the two ratings part.
pub(super) fn health_look_phrase(tier: HealthTier, rating: ContentRating) -> Option<&'static str> {
    let mature = rating == ContentRating::Mature;
    match tier {
        HealthTier::Healthy => None,
        HealthTier::Hurt => Some("tense and guarded, the clothing a little rumpled and dirtied"),
        HealthTier::Bloodied if mature => {
            Some("visibly wounded: blood from cuts, bruising, and torn clothing and gear")
        }
        HealthTier::Bloodied => {
            Some("battered: smudges of dirt and sweat, bruised and rumpled clothing")
        }
        HealthTier::Critical if mature => {
            Some("barely standing and soaked in blood: deep wounds and ragged, torn gear")
        }
        HealthTier::Critical => {
            Some("barely standing, ashen and exhausted, in filthy, ragged clothing")
        }
        HealthTier::Down if mature => Some(
            "collapsed and bloodied, limp, the gear torn and the body slumped in the same frame",
        ),
        HealthTier::Down => {
            Some("collapsed and limp, battered and dirty, slumped in the same frame")
        }
    }
}

/// The rating's stance and limits, appended to every prompt.
pub(super) fn rating_parts(rating: ContentRating) -> Vec<Part> {
    let mut parts = vec![Part::Fixed(FILM_LOOK), Part::Fixed(NO_TEXT)];
    parts.push(Part::Fixed(NO_REAL_PEOPLE));
    parts.push(Part::Fixed(ADULTS_ONLY));
    parts.push(Part::Fixed(match rating {
        ContentRating::Mature => MATURE_STANCE,
        ContentRating::Restrained => RESTRAINED_LIMITS,
    }));
    parts
}

pub(super) fn place_parts(place: &PlaceFacts) -> Vec<Part> {
    let mut parts = vec![
        Part::Fixed(SCENE_SETTING),
        Part::Fact(format!("{}.", place.name.as_str().trim_end_matches('.'))),
    ];
    if let Some(short) = &place.short {
        parts.push(Part::Fact(format!(
            "{}.",
            short.as_str().trim_end_matches('.')
        )));
    }
    parts
}

/// The prompt that re-frames the current backdrop: same place, no people, the look's camera and
/// light.
pub(in crate::gameplay) fn scene_refresh(
    rating: ContentRating,
    place: &PlaceFacts,
    look: Look,
) -> AssembledPrompt {
    let mut parts = vec![Part::Fixed(RECAMERA)];
    parts.extend(place_parts(place));
    parts.extend([
        Part::Fixed(KEEP_PLACE),
        Part::Fixed(NO_PEOPLE),
        Part::Fixed(KEEP_FLOOR),
        Part::Fixed(KEEP_FRAME),
        Part::Fixed("The camera:"),
        Part::Fixed(shot_phrase(look.shot)),
        Part::Fixed("The light:"),
        Part::Fixed(lighting_phrase(look.lighting)),
        Part::Fixed("The mood of the room:"),
        Part::Fixed(mood_phrase(look.mood, rating)),
    ]);
    parts.extend(rating_parts(rating));
    AssembledPrompt::from_parts(parts)
}

/// The prompt that changes one character's face (and, when hurt, their tier) and nothing
/// else. `real_likeness` marks art that derives from a real person's likeness: such a character
/// never gets a seductive variant.
pub(in crate::gameplay) fn cast_expression(
    rating: ContentRating,
    character: CharacterFacts,
    expression: Expression,
    tier: HealthTier,
    real_likeness: bool,
) -> Result<AssembledPrompt, PromptRefusal> {
    if real_likeness && expression == Expression::Seductive {
        return Err(PromptRefusal::RealLikenessIntimate);
    }
    let mut parts = vec![
        Part::Fixed(KEEP_PERSON),
        Part::Fixed(KEEP_TRANSPARENT),
        Part::Fixed(CHARACTER_IS),
        Part::Fixed(character.life_stage.phrase()),
        Part::Fixed(character.role.phrase()),
        Part::Fixed(CHANGE_ONLY),
        Part::Fixed(SHOW_EXPRESSION),
    ];
    // Priority, not multiplication: a hurt character shows the health face and look, a healthy
    // one the conversation's mood face.
    match health_face_phrase(tier) {
        Some(face) => {
            parts.push(Part::Fixed(face));
            if let Some(look) = health_look_phrase(tier, rating) {
                parts.push(Part::Fixed(SHOW_APPEARANCE));
                parts.push(Part::Fixed(look));
            }
        }
        None => parts.push(Part::Fixed(expression_phrase(expression, rating))),
    }
    parts.extend(rating_parts(rating));
    Ok(AssembledPrompt::from_parts(parts))
}

/// Every fixed phrase a prompt of `rating` can contain, for the closed-vocabulary check.
#[cfg(test)]
pub(in crate::gameplay) fn closed_vocabulary(rating: ContentRating) -> Vec<&'static str> {
    let mut phrases = super::artifact::FIXED_PHRASES.to_vec();
    phrases.extend([
        FILM_LOOK,
        NO_TEXT,
        NO_REAL_PEOPLE,
        ADULTS_ONLY,
        RESTRAINED_LIMITS,
        MATURE_STANCE,
        KEEP_PLACE,
        NO_PEOPLE,
        KEEP_FLOOR,
        KEEP_FRAME,
        KEEP_PERSON,
        KEEP_TRANSPARENT,
        CHANGE_ONLY,
        RECAMERA,
        SCENE_SETTING,
        CHARACTER_IS,
        SHOW_EXPRESSION,
        SHOW_APPEARANCE,
        "The camera:",
        "The light:",
        "The mood of the room:",
    ]);
    phrases.extend(Mood::ALL.map(|mood| mood_phrase(mood, rating)));
    phrases.extend(Shot::ALL.map(shot_phrase));
    phrases.extend(Lighting::ALL.map(lighting_phrase));
    phrases.extend(Expression::ALL.map(|expression| expression_phrase(expression, rating)));
    for tier in HealthTier::ALL {
        phrases.extend(health_face_phrase(tier));
        phrases.extend(health_look_phrase(tier, rating));
    }
    phrases.extend(CharacterRole::ALL.map(CharacterRole::phrase));
    phrases.extend(LifeStage::ALL.map(LifeStage::phrase));
    phrases
}
