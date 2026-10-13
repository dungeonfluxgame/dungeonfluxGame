//! The scene plate prompt: one composed photographic film still of a story scene (PLAY-V09).
//!
//! The prompt is built from a [`ShotSpec`] and fixed phrases only. The spec's own strings come
//! from the closed vocabulary of the shot director, the place name is a vetted fact of the
//! adventure record, and the only other free text is the percentages of the speaker's head
//! position. Nothing a player typed, and nothing a model wrote, reaches it. The photographic
//! recipe is `artifacts/tmp/play-v06b/look-v2.md` (physical capture terms rather than a
//! production style); the staging blocks are the ones the PLAY-V03 experiments proved.
//!
//! A moment change (a lasting expression of the speaker) is an edit of the existing plate, not a
//! fresh render: an edit keeps the room, the camera and the extras, a fresh render re-stages them.

use super::{
    super::{
        pack::Gender,
        scene_mood::{Expression as FaceExpression, HealthTier},
        shot::{
            DressingZone, Expression, ExtraZone, HeroShot, HeroSlot, Mood, ShotSpec, SpeakerShot,
            TimeOfDay, asked_head,
        },
        startup_configuration::ContentRating,
    },
    prompt::{
        AssembledPrompt, LifeStage, Part, PlaceFacts, expression_phrase, health_face_phrase,
        health_look_phrase,
    },
};

/// The adapter's prompt limit, in characters.
pub(in crate::gameplay) const MAXIMUM_PROMPT_CHARACTERS: usize = 4000;

/// Why a plate prompt could not be built. Nothing is sent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum PlatePromptRefusal {
    /// A light name the phrase table does not know (a vocabulary defect).
    UnknownLight,
    /// The assembled prompt is longer than the adapter accepts.
    TooLong,
    /// More references than the adapter takes, or a hero without a party sheet.
    References,
}

/// Which references the request carries, in this order: the party sheet, the speaker's art, the
/// place painting. A reference that is absent is simply left out and the next one moves up.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct PlateReferences {
    pub(in crate::gameplay) party_sheet: bool,
    pub(in crate::gameplay) speaker: bool,
    /// A close portrait of the speaker's face, which pins identity from scene to scene.
    pub(in crate::gameplay) speaker_face: bool,
    pub(in crate::gameplay) place: bool,
}

impl PlateReferences {
    fn number(self, which: Which) -> Option<u8> {
        let mut next = 1;
        for (present, candidate) in [
            (self.party_sheet, Which::PartySheet),
            (self.speaker, Which::Speaker),
            (self.speaker_face, Which::SpeakerFace),
            (self.place, Which::Place),
        ] {
            if candidate == which {
                return present.then_some(next);
            }
            if present {
                next += 1;
            }
        }
        None
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Which {
    PartySheet,
    Speaker,
    SpeakerFace,
    Place,
}

const PHOTO: &str = "A single frame from a live-action film set, shot on a real camera with a fast prime lens \
    at about f/2.8, focus on the speaker's face. Real people in practical costume: sewn, \
    mended, stained wool and linen, creased leather, dull dirty mail. Real skin, pores, \
    stubble, no retouching. Motivated, uneven light with deep shadows, no studio fill. Light \
    even grain, halation only around flames and lantern glass. Muted colour grade with deep \
    blacks.";
const CAMERA: &str = "A wide shot on a 28mm lens from far back among the party: the heroes are small figures \
    at the edges of the picture, each no wider than a quarter of the frame, and the room \
    itself is the subject, its beams, floor and far walls all in view.";
const PLACE_IS: &str = "The place is";
const INTERIOR: &str = "Indoors, under a ceiling.";
const EXTERIOR: &str = "Outdoors.";
const PLACE_REFERENCE: [&str; 5] = [
    "",
    "Image 1 is a painting of this place: keep its architecture, props and palette, but \
     repaint it as a lived-in working room from the new camera position.",
    "Image 2 is a painting of this place: keep its architecture, props and palette, but \
     repaint it as a lived-in working room from the new camera position.",
    "Image 3 is a painting of this place: keep its architecture, props and palette, but \
     repaint it as a lived-in working room from the new camera position.",
    "Image 4 is a painting of this place: keep its architecture, props and palette, but \
     repaint it as a lived-in working room from the new camera position.",
];
const HEROES_INTRO: [&str; 4] = [
    "",
    "The hero stands in the foreground, seen from behind or three-quarter back, turned toward \
     the speaker, not looking at the camera, busy with something that fits the place.",
    "The 2 heroes stand in the foreground and middle distance, seen from behind or \
     three-quarter back, all turned toward the speaker, none looking at the camera, not in a \
     row, each busy with something that fits the place.",
    "The 3 heroes stand in the foreground and middle distance, seen from behind or \
     three-quarter back, all turned toward the speaker, none looking at the camera, not in a \
     row, each busy with something that fits the place.",
];
const HEROES_IDENTITY: &str = "Image 1 is the party sheet (the heroes on plain grey): keep each hero's exact face, \
    hair, build, costume and weapon, in new poses and viewpoint.";
const HERO_NUMBER: [&str; 3] = ["Hero 1", "Hero 2", "Hero 3"];
const SHEET_POSITION: [&str; 3] = [
    "(first figure on the sheet,",
    "(second figure on the sheet,",
    "(third figure on the sheet,",
];
const HERO_IS: &str = "is";
const SPEAKER_IS: &str = "The speaker is";
const SPEAKER_REFERENCE: [&str; 5] = [
    "",
    "Image 1 shows this person: keep their exact face, hair, headwear and clothes, not that \
     image's background.",
    "Image 2 shows this person: keep their exact face, hair, headwear and clothes, not that \
     image's background.",
    "Image 3 shows this person: keep their exact face, hair, headwear and clothes, not that \
     image's background.",
    "",
];
const SPEAKER_FACE: [&str; 5] = [
    "",
    "",
    "Image 2 is a close portrait of their face: match it exactly.",
    "Image 3 is a close portrait of their face: match it exactly.",
    "Image 4 is a close portrait of their face: match it exactly.",
];
const SPEAKER_STAGING: &str = "They stand in the lit mid-ground, the best-lit person, in crisp focus, facing the party \
    and the camera, mid-task at their workplace:";
const SPEAKER_EXPRESSION: &str = "Their expression is";
const FOREGROUND: &str = "Soft out-of-focus foreground objects fill the bottom corners:";
const BEHIND: &str = "The speaker and the heroes are in front of, in dimmer and softer light:";
const OTHER: &str = "Also in the room, watchful and far back:";
const DEEP: &str = "then deep architecture with small warm lights.";
const LIGHT: &str = "Light:";
const MOOD: &str = "Mood:";
const PRACTICALS: &str = "Warm practical sources:";
const EDGES: &str = "the edges of the frame fall darker.";
const SAFE: &str = "Keep clear for an interface: no faces or important detail in the top-left band (top 18%, \
    left 60%), the top-right block (top 30%, right 34%) or the bottom-centre strip (bottom \
    12%, middle 30%). All heads below the top 30%.";
const HEAD_ABOUT: &str = "The speaker's head is about";
const HEAD_ACROSS: &str = "% across and";
const HEAD_DOWN: &str = "% down.";
const CALM_LEFT: &str = "Directly left of the head: a calm area of dark, soft, out-of-focus background about two \
    head-widths wide, no faces, no bright details, for a speech bubble.";
const CALM_RIGHT: &str = "Directly right of the head: a calm area of dark, soft, out-of-focus background about two \
    head-widths wide, no faces, no bright details, for a speech bubble.";
const NEGATIVE: &str = "Not a painting, illustration, concept art, 3D render, game art, CGI or HDR. No outlines \
    or cut-out look, no floating figures, no plastic or waxy skin, no glossy metal, no \
    speckle on mail or cloth. No text, letters, numbers, signs, logos, borders, frames, \
    interface or speech bubbles. No real people or celebrities. Every person is clearly an \
    adult.";
const MATURE_STANCE: &str = "Mature content rating: violence, horror and desire are shown as \
    the story demands, not softened.";
const RESTRAINED_LIMITS: &str = "No gore, no nudity, no sexual content.";
const MOMENT_FIRST: &str = "Image 1 is the previous film still of this exact scene. Keep the \
    same place, cast, faces, hair, clothing, weapons and photographic art style. Keep the \
    camera and framing roughly the same, but let poses and small actions move on naturally \
    from what was just said or done: a gesture, a turn of the head, a step, or hands busy with \
    the scene's activity. Change the expression and this small action to match what just happened.";
const MOMENT_SPEAKER_WITH_REFERENCE: &str =
    "The speaker (image 2 shows their face and clothes) now shows";
const MOMENT_SPEAKER: &str = "The speaker now shows";
const MOMENT_APPEARANCE: &str = "The speaker also appears";
const MOMENT_LAST: &str = "Keep the same people and place; the speaker's head stays roughly in \
    the same place in the frame.";

fn gender_phrase(gender: Option<Gender>) -> &'static str {
    match gender {
        Some(Gender::He) => "(a man),",
        Some(Gender::She) => "(a woman),",
        Some(Gender::They | Gender::It) | None => "(a person),",
    }
}

fn stage_phrase(stage: LifeStage) -> &'static str {
    match stage {
        LifeStage::YoungAdult => "a young adult, clearly over eighteen.",
        LifeStage::Adult => "a grown adult.",
        LifeStage::Elder => "an elderly adult.",
    }
}

fn rating_stance(rating: ContentRating) -> &'static str {
    match rating {
        ContentRating::Mature => MATURE_STANCE,
        ContentRating::Restrained => RESTRAINED_LIMITS,
    }
}

fn light_phrase(name: &str) -> Option<&'static str> {
    Some(match name {
        "hearth-fire" => "the hearth fire,",
        "lantern-warm" => "warm hanging lanterns,",
        "candle-cluster" => "clusters of candles,",
        "forge-glow" => "the forge glow,",
        "window-cool-daylight" => "cool daylight from a window,",
        "window-cool-moonlight" => "cool moonlight from a window,",
        "brazier-orange" => "an orange brazier,",
        "torch-smoke" => "smoking torches,",
        "pale-overcast-sky" => "pale overcast sky light,",
        "lamp-glow" => "oil lamps,",
        _ => return None,
    })
}

fn time_phrase(time: TimeOfDay) -> &'static str {
    match time {
        TimeOfDay::Dawn => "dawn, cold grey-blue first light against the warm interior.",
        TimeOfDay::Morning => "morning, soft pale daylight against the warm interior.",
        TimeOfDay::Midday => "midday, bright cool daylight from the openings.",
        TimeOfDay::Afternoon => "late afternoon, low amber daylight from the openings.",
        TimeOfDay::Dusk => "dusk, deep blue light outside against warm practical light inside.",
        TimeOfDay::Night => "night, cold moonlight against warm practical light.",
    }
}

fn mood_phrase(mood: Mood) -> &'static str {
    match mood {
        Mood::Tense => "tense and taut.",
        Mood::Warm => "warm and settled.",
        Mood::Grim => "grim and subdued.",
        Mood::Hopeful => "hopeful, a little light breaking through.",
        Mood::Wary => "wary and watchful.",
        Mood::Quiet => "quiet and still.",
        Mood::Uneasy => "uneasy and unsettled.",
        Mood::Urgent => "urgent, time pressing.",
    }
}

fn expression_phrase_of(expression: Expression) -> &'static str {
    match expression {
        Expression::Warm => "a warm, knowing half-smile.",
        Expression::Wary => "wary, eyes narrowed and watchful.",
        Expression::Weary => "weary, tired eyes and a slack mouth.",
        Expression::Stern => "stern, a set jaw and a level, appraising look.",
        Expression::Anxious => "anxious, brows drawn up and glancing aside.",
        Expression::Amused => "wry and amused, one brow raised.",
        Expression::Pleased => "quietly pleased, a small relieved smile.",
        Expression::Grim => "grim, heavy-lidded and unsmiling.",
        Expression::Neutral => "calm and neutral.",
    }
}

fn slot_phrase(slot: HeroSlot) -> &'static str {
    match slot {
        HeroSlot::LeftForeground => {
            "at the left side of the frame, small, from behind, head 55% down,"
        }
        HeroSlot::RightForeground => {
            "at the right side of the frame, small, from behind, head 55% down,"
        }
        HeroSlot::LeftMid => "in the left middle distance, three-quarter back,",
        HeroSlot::RightMid => "in the right middle distance, three-quarter back,",
        HeroSlot::BackLeft => "small at the far left, in profile,",
        HeroSlot::BackRight => "small at the far right, in profile,",
    }
}

/// What each hero's art shows, for the party sheet's figures.
fn hero_description(art: u8) -> &'static str {
    match art {
        1 => "red-bearded adult dwarf fighter in chainmail, greatsword)",
        2 => "red-haired adult dwarf woman fighter in a blue cloak, greatsword)",
        _ => "blonde-braided adult dwarf woman archer in a teal cloak, longbow)",
    }
}

/// What the head locator is told about a hero seen from behind in the plate: fixed phrases that
/// match the hero's art, so it can tell one hero's head from another's.
pub(in crate::gameplay) fn hero_locator_phrase(art: u8) -> &'static str {
    match art {
        1 => "the red-bearded dwarf fighter in chainmail, seen from behind or in profile",
        2 => "the red-haired braided dwarf woman fighter in a blue cloak, seen from behind",
        _ => "the blonde-braided dwarf woman archer in a teal cloak, seen from behind",
    }
}

fn extra_zone_phrase(zone: ExtraZone) -> &'static str {
    match zone {
        ExtraZone::FarLeft => "far back on the left;",
        ExtraZone::FarRight => "far back on the right;",
        ExtraZone::Deep => "deep in the middle distance;",
    }
}

fn dressing_zone_phrase(zone: DressingZone) -> &'static str {
    match zone {
        DressingZone::BottomLeft => "bottom left,",
        DressingZone::BottomRight => "bottom right.",
    }
}

/// The position of a hero on the party sheet: heroes stand on it in the order of their art.
pub(in crate::gameplay) fn sheet_position(heroes: &[HeroShot], art: u8) -> usize {
    heroes.iter().filter(|hero| hero.art < art).count()
}

/// The prompt for a fresh plate of `spec`. `place` is the scene's vetted place name; a name that
/// would take the prompt over the adapter's limit is left out (the vocabulary describes the place).
pub(in crate::gameplay) fn scene_plate(
    rating: ContentRating,
    spec: &ShotSpec,
    place: Option<&PlaceFacts>,
    references: PlateReferences,
) -> Result<AssembledPrompt, PlatePromptRefusal> {
    match scene_plate_with(rating, spec, place, references) {
        Err(PlatePromptRefusal::TooLong) if place.is_some() => {
            scene_plate_with(rating, spec, None, references)
        }
        other => other,
    }
}

fn scene_plate_with(
    rating: ContentRating,
    spec: &ShotSpec,
    place: Option<&PlaceFacts>,
    references: PlateReferences,
) -> Result<AssembledPrompt, PlatePromptRefusal> {
    let reference_count = [references.party_sheet, references.speaker, references.place]
        .into_iter()
        .filter(|present| *present)
        .count();
    if reference_count > 4 || references.party_sheet != !spec.heroes.is_empty() {
        return Err(PlatePromptRefusal::References);
    }
    let mut parts = vec![
        Part::Fixed(PHOTO),
        Part::Fixed(CAMERA),
        Part::Fixed(PLACE_IS),
    ];
    if let Some(place) = place {
        parts.push(Part::Fact(format!(
            "{}:",
            place.name.as_str().trim_end_matches('.')
        )));
    }
    parts.push(Part::Fixed(spec.place_description));
    parts.push(Part::Fixed("."));
    parts.push(Part::Fixed(if spec.exterior { EXTERIOR } else { INTERIOR }));
    if let Some(number) = references.number(Which::Place) {
        parts.push(Part::Fixed(PLACE_REFERENCE[usize::from(number)]));
    }
    if !spec.heroes.is_empty() {
        parts.push(Part::Fixed(HEROES_INTRO[spec.heroes.len().min(3)]));
        parts.push(Part::Fixed(HEROES_IDENTITY));
        for (ordinal, hero) in spec.heroes.iter().enumerate() {
            let sheet = sheet_position(&spec.heroes, hero.art).min(2);
            parts.extend([
                Part::Fixed(HERO_NUMBER[ordinal.min(2)]),
                Part::Fixed(SHEET_POSITION[sheet]),
                Part::Fixed(hero_description(hero.art)),
                Part::Fixed(slot_phrase(hero.slot)),
                Part::Fixed(HERO_IS),
                Part::Fixed(hero.activity),
            ]);
            // The activity is a phrase; end each hero's sentence.
            parts.push(Part::Fixed("."));
        }
    }
    if let Some(speaker) = &spec.speaker {
        parts.extend(speaker_parts(speaker, references));
    }
    parts.push(Part::Fixed(FOREGROUND));
    for dressing in &spec.dressing {
        parts.push(Part::Fixed(dressing.item));
        parts.push(Part::Fixed(dressing_zone_phrase(dressing.zone)));
    }
    if !spec.extras.is_empty() {
        parts.push(Part::Fixed(BEHIND));
        for extra in &spec.extras {
            parts.push(Part::Fixed(extra.name));
            parts.push(Part::Fixed(extra_zone_phrase(extra.zone)));
        }
        parts.push(Part::Fixed(DEEP));
    }
    if let Some(other) = spec.others.first() {
        parts.extend([Part::Fixed(OTHER), Part::Fixed(other), Part::Fixed(".")]);
    }
    parts.extend([
        Part::Fixed(LIGHT),
        Part::Fixed(time_phrase(spec.time_of_day)),
        Part::Fixed(MOOD),
        Part::Fixed(mood_phrase(spec.mood)),
        Part::Fixed(PRACTICALS),
    ]);
    for light in &spec.lights {
        parts.push(Part::Fixed(
            light_phrase(light).ok_or(PlatePromptRefusal::UnknownLight)?,
        ));
    }
    parts.push(Part::Fixed(EDGES));
    parts.push(Part::Fixed(SAFE));
    if let Some(speaker) = &spec.speaker {
        let (x, y) = asked_head(&speaker.layout);
        parts.extend([
            Part::Fixed(HEAD_ABOUT),
            Part::Fact((x / 10).to_string()),
            Part::Fixed(HEAD_ACROSS),
            Part::Fact((y / 10).to_string()),
            Part::Fixed(HEAD_DOWN),
            Part::Fixed(match speaker.layout.bubble_side {
                super::super::shot::Side::Left => CALM_LEFT,
                super::super::shot::Side::Right => CALM_RIGHT,
            }),
        ]);
    }
    parts.push(Part::Fixed(NEGATIVE));
    parts.push(Part::Fixed(rating_stance(rating)));
    finish(parts)
}

fn speaker_parts(speaker: &SpeakerShot, references: PlateReferences) -> Vec<Part> {
    let mut parts = vec![
        Part::Fixed(SPEAKER_IS),
        Part::Fixed(speaker.look),
        Part::Fixed(gender_phrase(speaker.gender)),
        Part::Fixed(stage_phrase(speaker.life_stage)),
    ];
    if let Some(number) = references.number(Which::Speaker) {
        parts.push(Part::Fixed(SPEAKER_REFERENCE[usize::from(number)]));
    }
    if let Some(number) = references.number(Which::SpeakerFace) {
        parts.push(Part::Fixed(SPEAKER_FACE[usize::from(number)]));
    }
    parts.extend([
        Part::Fixed(SPEAKER_STAGING),
        Part::Fixed(speaker.task),
        Part::Fixed("."),
        Part::Fixed(SPEAKER_EXPRESSION),
        Part::Fixed(expression_phrase_of(speaker.expression)),
    ]);
    parts
}

fn finish(parts: Vec<Part>) -> Result<AssembledPrompt, PlatePromptRefusal> {
    let prompt = AssembledPrompt::from_parts(parts);
    if prompt.as_str().chars().count() > MAXIMUM_PROMPT_CHARACTERS {
        return Err(PlatePromptRefusal::TooLong);
    }
    Ok(prompt)
}

/// The prompt that edits an existing plate: the previous plate is image 1, the speaker's art (when
/// there is one) image 2, and only the speaker's face and look change.
pub(in crate::gameplay) fn plate_moment(
    rating: ContentRating,
    expression: FaceExpression,
    tier: HealthTier,
    speaker_reference: bool,
) -> Result<AssembledPrompt, PlatePromptRefusal> {
    let mut parts = vec![
        Part::Fixed(MOMENT_FIRST),
        Part::Fixed(if speaker_reference {
            MOMENT_SPEAKER_WITH_REFERENCE
        } else {
            MOMENT_SPEAKER
        }),
    ];
    // Priority, not multiplication (as for the cut-out): a hurt speaker shows the health face and
    // look, a healthy one the conversation's mood face.
    match health_face_phrase(tier) {
        Some(face) => {
            parts.push(Part::Fixed(face));
            if let Some(look) = health_look_phrase(tier, rating) {
                parts.push(Part::Fixed(MOMENT_APPEARANCE));
                parts.push(Part::Fixed(look));
            }
        }
        None => parts.push(Part::Fixed(expression_phrase(expression, rating))),
    }
    parts.extend([
        Part::Fixed(MOMENT_LAST),
        Part::Fixed(NEGATIVE),
        Part::Fixed(rating_stance(rating)),
    ]);
    finish(parts)
}

/// Every fixed phrase a plate prompt can contain besides the vocabulary of the shot director,
/// for the closed-vocabulary check.
#[cfg(test)]
pub(in crate::gameplay) fn closed_vocabulary(rating: ContentRating) -> Vec<&'static str> {
    use super::super::shot::{TimeOfDay as Time, vocabulary_lights};
    let mut phrases = vec![
        PHOTO,
        CAMERA,
        PLACE_IS,
        INTERIOR,
        EXTERIOR,
        HEROES_IDENTITY,
        HERO_IS,
        SPEAKER_IS,
        SPEAKER_STAGING,
        SPEAKER_EXPRESSION,
        FOREGROUND,
        BEHIND,
        DEEP,
        OTHER,
        LIGHT,
        MOOD,
        PRACTICALS,
        EDGES,
        SAFE,
        HEAD_ABOUT,
        HEAD_ACROSS,
        HEAD_DOWN,
        CALM_LEFT,
        CALM_RIGHT,
        NEGATIVE,
        rating_stance(rating),
        MOMENT_FIRST,
        MOMENT_SPEAKER_WITH_REFERENCE,
        MOMENT_SPEAKER,
        MOMENT_APPEARANCE,
        MOMENT_LAST,
        ".",
    ];
    phrases.extend(PLACE_REFERENCE.iter().skip(1));
    phrases.extend(HEROES_INTRO.iter().skip(1));
    phrases.extend(HERO_NUMBER);
    phrases.extend(SHEET_POSITION);
    phrases.extend(SPEAKER_REFERENCE.iter().filter(|phrase| !phrase.is_empty()));
    phrases.extend(SPEAKER_FACE.iter().filter(|phrase| !phrase.is_empty()));
    phrases.extend(vocabulary_lights().into_iter().filter_map(light_phrase));
    phrases.extend(Time::ALL.map(time_phrase));
    phrases.extend(
        [
            Mood::Tense,
            Mood::Warm,
            Mood::Grim,
            Mood::Hopeful,
            Mood::Wary,
            Mood::Quiet,
            Mood::Uneasy,
            Mood::Urgent,
        ]
        .map(mood_phrase),
    );
    phrases.extend(
        [
            Expression::Warm,
            Expression::Wary,
            Expression::Weary,
            Expression::Stern,
            Expression::Anxious,
            Expression::Amused,
            Expression::Pleased,
            Expression::Grim,
            Expression::Neutral,
        ]
        .map(expression_phrase_of),
    );
    phrases.extend([1, 2, 3].map(hero_description));
    phrases.extend([1, 2, 3].map(hero_locator_phrase));
    phrases.extend([Some(Gender::He), Some(Gender::She), None].map(gender_phrase));
    phrases.extend([LifeStage::YoungAdult, LifeStage::Adult, LifeStage::Elder].map(stage_phrase));
    phrases.extend(
        [
            HeroSlot::LeftForeground,
            HeroSlot::RightForeground,
            HeroSlot::LeftMid,
            HeroSlot::RightMid,
            HeroSlot::BackLeft,
            HeroSlot::BackRight,
        ]
        .map(slot_phrase),
    );
    phrases
        .extend([ExtraZone::FarLeft, ExtraZone::FarRight, ExtraZone::Deep].map(extra_zone_phrase));
    phrases.extend([DressingZone::BottomLeft, DressingZone::BottomRight].map(dressing_zone_phrase));
    phrases
}

#[cfg(test)]
#[path = "plate_prompt_tests.rs"]
mod tests;
