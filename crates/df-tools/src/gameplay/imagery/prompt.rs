//! Image prompts, assembled only from a closed vocabulary the server owns and vetted facts.
//!
//! Mood, shot, lighting, expression and tier are server-owned enums. Authored facts are
//! vetted; the creator also accepts bounded public custom appearance as quoted reference
//! data. Creator classification runs before an image call. Private story, player names and
//! camera images are excluded. No reference text grants instructions or game mechanics.
//! The fixed photographic direction also follows the server's content rating.
//!
//! Limits that hold under both ratings: every person is an adult (the types have no younger
//! variant and a young character is described as a "young adult"), no real person's likeness
//! gets a seductive variant, and no real public figure or brand can be named.

use super::super::{
    hero_identity::{IdentityError, ValidatedIdentity},
    pack,
    scene_mood::{Expression, HealthTier, Lighting, Look, Mood, Shot},
    startup_configuration::ContentRating,
};

const MAXIMUM_FACT_CHARACTERS: usize = 80;

// The owner's direction (2026-10-17): it must read like living in a D&D world, shot like a
// realistic live-action TV drama ("photographic cinema"). The look names only what the photograph
// is: naming another medium, even to forbid it, steers the model toward it (owner, 2026-10-17).
pub(in crate::gameplay) const FILM_LOOK: &str = "A still frame from a live-action fantasy TV drama set in a lived-in Dungeons & \
    Dragons world, shot on a real camera with a prime lens. Documentary color photograph: real \
    adult performers with real skin pores and natural skin texture, practical costumes in uneven \
    woven fabric with visible weave, worn real props on a real dressed location, natural \
    practical light, natural depth of field and fine film grain. One continuous camera frame of \
    one real place: no grid, no panels, no numbers, no text, no signage, no letters, \
    no logos, no glamour, no HDR, no heroic posing.";
/// The output is one frame, never a sheet: identity references are several photographs, and a
/// model that copies their layout produces the collage the owner rejected.
pub(super) const SINGLE_FRAME: &str = "The output is ONE single continuous photographic camera frame, one uninterrupted \
    view of one place at one moment: never a grid, collage, contact sheet, split screen, panels, \
    diptych or montage, and never subjects pasted onto a background.";
const NO_TEXT: &str = "No text, letters, numbers, signage, signs, logos or watermarks anywhere, \
    including on cloth, boards, banners and walls; any such surface is plain and blank.";
const NO_REAL_PEOPLE: &str = "Fictional identities; no recognizable real person or celebrity.";
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
const KEEP_PERSON: &str = "Keep exactly the same person: face, hair, facial hair, build, costume, \
    pose, framing, camera distance and lighting as in the reference image.";
const KEEP_TRANSPARENT: &str = "Keep the background fully transparent.";
const CHANGE_ONLY: &str = "Change only the following.";
const RECAMERA: &str = "Re-shoot the reference photograph from a changed camera and light.";
const SCENE_SETTING: &str = "The place is";
const CHARACTER_IS: &str = "The character is";
const SHOW_EXPRESSION: &str = "The face shows";
const SHOW_APPEARANCE: &str = "The character appears";
#[allow(
    dead_code,
    reason = "T02 stage rendering must consume the checked hero portrait prompt"
)]
const HERO_PORTRAIT_REFERENCE: &str = "Image 1 is a neutral pose and lighting reference only; rebuild \
    every physical identity detail from the selected identity below.";
#[allow(
    dead_code,
    reason = "T02 stage rendering must consume the checked hero portrait prompt"
)]
const HERO_PORTRAIT: &str = "Photograph exactly one adult fantasy hero (count: 1, alone, no other person, animal or \
    figure) full-body from head to boots, in a clear three-quarter view, with the face, hair, \
    clothing and held gear plainly visible. The performer was photographed against a plain \
    neutral grey photographic backdrop with real skin and fabric texture and natural \
    photographic edges; the background is delivered transparent only because downstream \
    layered compositing requires alpha, and the figure keeps the light, skin and fabric of that \
    photograph.";
#[allow(
    dead_code,
    reason = "T02 stage rendering must consume the checked hero cut-out prompt"
)]
const HERO_SHEET_REFERENCE: &str = "Image 1 is the party identity reference: separate full-body photographs side by \
    side, one hero per photograph. Isolate only its selected figure as a full-body transparent \
    cut-out for downstream compositing; use the reference for pose and scale only, never copy \
    its side-by-side layout, and rebuild identity from the selected details below.";
#[allow(
    dead_code,
    reason = "T02 stage rendering must consume the checked hero image prompts"
)]
const HERO_SELECTED_IDENTITY: &str =
    "The hero's exact selected identity and canonical held gear are:";
const HERO_APPEARANCE_ONLY: &str =
    "The selected attire is appearance only; it grants no equipment or rules.";
#[allow(
    dead_code,
    reason = "T02 stage rendering must consume the checked hero expression prompt"
)]
const HERO_EXPRESSION_IDENTITY: &str = "Keep the same selected hero identity, canonical gear and \
    facial-hair choice exactly as described.";
#[allow(
    dead_code,
    reason = "T02 stage rendering must consume the checked hero cut-out prompt"
)]
const HERO_NUMBERED_FIGURE: &str = "The selected party-sheet figure is number";
#[allow(
    dead_code,
    reason = "T02 stage rendering must consume the checked hero cut-out prompt"
)]
const HERO_NUMBERED_FIGURE_END: &str = "from the left.";
#[allow(
    dead_code,
    reason = "T02 stage rendering must consume the checked hero cut-out prompt"
)]
const HERO_SHEET_COUNT: &str = "The party sheet contains";
#[allow(
    dead_code,
    reason = "T02 stage rendering must consume the checked hero cut-out prompt"
)]
const HERO_SHEET_FIGURES: &str = "figures.";

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

    /// The still of one committed killing blow: the kill-shot plan's photographic prompt, built
    /// from the committed facts only (see `home_video::kill_shot::still_plan`).
    pub(in crate::gameplay) fn kill_shot(
        request: &crate::gameplay::home_video::kill_shot::KillShotRequest,
        rating: ContentRating,
    ) -> Self {
        let plan = crate::gameplay::home_video::kill_shot::still_plan(request, rating);
        Self::from_parts(vec![Part::Fact(plan.prompt)])
    }

    pub(in crate::gameplay) fn with_cast_sheet(
        mut self,
        image_number: usize,
        legend: &str,
    ) -> Self {
        self.text.push_str(&format!(
            "\nImage {image_number} is the canonical photographic identity contact sheet. {legend}. It shows separate identity photographs, one person per photograph in slot order, each photographed against a neutral backdrop. Match each named person's exact face, age, hair, hood, clothes and equipment on every rebuild and edit. The photographs identify performers only: include only canonical scene participants. The output is ONE single continuous photographic camera frame: never copy the sheet's side-by-side layout, grid, panels, numbers, labels, backdrop or cut-out look into the photograph, and never paste figures onto a background. Preserve the medieval live-action documentary photograph: real skin and woven fabric texture."
        ));
        self
    }

    pub(in crate::gameplay) fn with_canonical_room_reference(
        mut self,
        image_number: usize,
    ) -> Self {
        self.text.push_str(&format!(
            "\nImage {image_number} is the canonical photograph of this exact authored room. Use it to preserve the room's architecture, geography, materials and lighting while keeping the current photograph's composition and camera."
        ));
        self
    }

    pub(in crate::gameplay) fn with_accepted_appearance_dossier(
        mut self,
        dossier: &'static str,
    ) -> Self {
        self.text.push_str(
            "\nImage 1 is the approved documentary photograph of this exact accepted hero. \
             Match its face, hair, build and costume; do not copy its background or pose. \
             The accepted quickstart portrait defines this hero's full public physical identity. \
             Preserve these exact details across every depiction: ",
        );
        self.text.push_str(dossier);
        self.text.push_str(
            " The approved photographic figure is an identity reference; use the accepted \
             character package for canonical equipment.",
        );
        self
    }

    pub(in crate::gameplay) fn with_canonical_speaker_reference(
        mut self,
        image_number: usize,
        actor_id: &str,
        actor_name: &str,
    ) -> Self {
        self.text.push_str(&format!(
            "\nImage {image_number} is the canonical photographic identity reference for actor id {actor_id}, named {actor_name}. Match this exact person's face, age, hair and clothing while preserving their established position and the current photograph's composition. The reference identifies the person only; do not copy its background or pose."
        ));
        self
    }

    /// Bind the camera's conversational focus to the canonical actor selected by the story
    /// record. The complete cast remains in the scene; this only identifies who must be readable.
    pub(in crate::gameplay) fn with_focus_actor(
        mut self,
        actor_id: &str,
        actor_name: &str,
    ) -> Self {
        self.text.push_str(&format!(
            "\nConversational camera focus: canonical actor id {actor_id}, named {actor_name}. Keep this exact cast member readable in the lit mid-ground with their face and upper body fully inside the frame; all other canonical cast members remain present in their established positions."
        ));
        self
    }

    /// Character count after all request fragments have been appended.
    pub(in crate::gameplay) fn character_count(&self) -> usize {
        self.text.chars().count()
    }

    /// Only the authenticated, gated development owner may supply this bounded direction.
    /// Append it after the scene description; retain the original identity and rating clauses.
    pub(in crate::gameplay) fn with_development_direction(mut self, direction: &str) -> Self {
        self.text.push_str("\nDevelopment picture direction (change the action only; preserve the scene, cast identities, faces, art style and content rating):\n");
        self.text.push_str(direction);
        self
    }

    pub(in crate::gameplay) fn with_canonical_binding(mut self, binding: &str) -> Self {
        self.text.push_str(&format!(
            "\nCanonical identity and authored layout binding: {binding}."
        ));
        self
    }

    pub(in crate::gameplay) fn with_photographic_style_reference(mut self) -> Self {
        self.text.push_str(
            "\nThe empty landscape reference supplies only documentary camera finish and materials. Written scene facts alone define location, layout, cast, objects and light.",
        );
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

/// Server-owned facts used to draw a player hero. Gear phrases must come from the canonical
/// admitted equipment list; identity appearance phrases come only from the validated menu ids.
#[derive(Clone, Copy, Debug)]
pub(in crate::gameplay) struct HeroVisualIdentity<'a> {
    pub(in crate::gameplay) identity: &'a ValidatedIdentity,
    pub(in crate::gameplay) species: &'a str,
    pub(in crate::gameplay) held_gear: &'a [&'static str],
}

impl HeroVisualIdentity<'_> {
    pub(in crate::gameplay) fn description(self) -> Result<String, IdentityError> {
        let appearance = self.identity.appearance_description(self.species)?;
        let gear = if self.held_gear.is_empty() {
            "none".to_owned()
        } else {
            self.held_gear.join(", ")
        };
        Ok(format!(
            "{}; canonical held gear: {gear}; {}",
            appearance.trim_end_matches('.'),
            HERO_APPEARANCE_ONLY
        ))
    }
}

/// A new hero image uses validated public appearance and canonical held gear. Player
/// names and private backstory never enter this prompt. Public custom appearance is quoted data.
#[allow(
    dead_code,
    reason = "T02 stage rendering must consume the checked hero portrait prompt"
)]
pub(in crate::gameplay) fn hero_portrait(
    rating: ContentRating,
    hero: HeroVisualIdentity<'_>,
) -> Result<AssembledPrompt, IdentityError> {
    // No pose carrier is sent any more (editing a blank carrier made the result read as
    // painted), so the portrait opens with its direction rather than a reference description.
    let mut parts = vec![
        Part::Fixed(FILM_LOOK),
        Part::Fixed(HERO_PORTRAIT),
        Part::Fixed(HERO_SELECTED_IDENTITY),
        Part::Fact(hero.description()?),
    ];
    parts.extend(rating_tail(rating));
    Ok(AssembledPrompt::from_parts(parts))
}

/// The creator stores an opaque JPEG, so its photographic backdrop must remain visible.
pub(in crate::gameplay) fn creator_portrait(
    rating: ContentRating,
    hero: HeroVisualIdentity<'_>,
    binding: &str,
    design: &str,
) -> Result<AssembledPrompt, IdentityError> {
    let mut parts = vec![
        Part::Fixed(FILM_LOOK),
        Part::Fixed(
            "Photograph exactly one fictional fantasy character alone, full body from head to boots, face and appearance clearly visible, in a natural three-quarter stance in the selected public portrait setting. Keep that continuous real setting visible in the opaque photograph, with the character clearly readable against it. Natural skin and fabric texture, soft cinematic lighting appropriate to that setting. No transparency, checkerboard, grid, panels or extra figures.",
        ),
        Part::Fixed(HERO_SELECTED_IDENTITY),
        Part::Fact(hero.description()?),
    ];
    parts.push(Part::Fixed("Realize the selected ancestry's actual anatomy in this live-action photograph. Nonhuman faces and bodies use convincing practical creature heads, articulated prosthetic jaws, sculpted ears, tusks, horns, layered scales and fitted creature costumes as appropriate, photographed with the same physical light and material detail. Natural surface texture follows the selected anatomy: scales, horns and tusks retain their physical structure. Keep the user's custom appearance and selected colors, build, adult age, gender presentation and clothing."));
    parts.push(Part::Fact(design.to_owned()));
    if rating == ContentRating::Restrained {
        parts.push(Part::Fixed("A clothed, nonsexual documentary character reference. Preserve the exact described body proportions naturally under the selected clothing. Body-size and anatomy details describe physical identity, without erotic posing or sexual framing."));
    }
    parts.extend(rating_tail(rating));
    Ok(AssembledPrompt::from_parts(parts).with_canonical_binding(binding))
}

/// A party-sheet figure is redrawn as its selected hero identity, rather than treated as the
/// source of face, body, hair or costume.
#[allow(
    dead_code,
    reason = "T02 stage rendering must consume the checked hero cut-out prompt"
)]
pub(in crate::gameplay) fn hero_cutout(
    rating: ContentRating,
    hero: HeroVisualIdentity<'_>,
    figure_index: usize,
    figure_count: usize,
) -> Result<AssembledPrompt, IdentityError> {
    let mut parts = vec![
        Part::Fixed(FILM_LOOK),
        Part::Fixed(HERO_SHEET_REFERENCE),
        Part::Fixed(HERO_NUMBERED_FIGURE),
        Part::Fact(figure_index.saturating_add(1).to_string()),
        Part::Fixed(HERO_NUMBERED_FIGURE_END),
        Part::Fixed(HERO_SHEET_COUNT),
        Part::Fact(figure_count.to_string()),
        Part::Fixed(HERO_SHEET_FIGURES),
        Part::Fixed(HERO_SELECTED_IDENTITY),
        Part::Fact(hero.description()?),
    ];
    parts.extend(rating_tail(rating));
    Ok(AssembledPrompt::from_parts(parts))
}

/// An expression or health edit keeps the hero's appearance and canonically held equipment.
#[allow(
    dead_code,
    reason = "T02 stage rendering must consume the checked hero expression prompt"
)]
pub(in crate::gameplay) fn hero_expression(
    rating: ContentRating,
    hero: HeroVisualIdentity<'_>,
    expression: Expression,
    tier: HealthTier,
) -> Result<AssembledPrompt, IdentityError> {
    let mut parts = vec![
        Part::Fixed(FILM_LOOK),
        Part::Fixed(KEEP_PERSON),
        Part::Fixed(KEEP_TRANSPARENT),
        Part::Fixed(HERO_EXPRESSION_IDENTITY),
        Part::Fixed(HERO_SELECTED_IDENTITY),
        Part::Fact(hero.description()?),
        Part::Fixed(CHANGE_ONLY),
        Part::Fixed(SHOW_EXPRESSION),
    ];
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
    parts.extend(rating_tail(rating));
    Ok(AssembledPrompt::from_parts(parts))
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

/// The same tail without FILM_LOOK, for builders in this module that put FILM_LOOK first.
fn rating_tail(rating: ContentRating) -> Vec<Part> {
    let mut parts = vec![Part::Fixed(NO_TEXT)];
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
    let mut parts = vec![
        Part::Fixed(FILM_LOOK),
        Part::Fixed(SINGLE_FRAME),
        Part::Fixed(RECAMERA),
    ];
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
    parts.extend(rating_tail(rating));
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
        Part::Fixed(FILM_LOOK),
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
    parts.extend(rating_tail(rating));
    Ok(AssembledPrompt::from_parts(parts))
}

/// Every fixed phrase a prompt of `rating` can contain, for the closed-vocabulary check.
#[cfg(test)]
pub(in crate::gameplay) fn closed_vocabulary(rating: ContentRating) -> Vec<&'static str> {
    let mut phrases = super::artifact::FIXED_PHRASES.to_vec();
    phrases.extend([
        FILM_LOOK,
        SINGLE_FRAME,
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
        HERO_PORTRAIT_REFERENCE,
        HERO_PORTRAIT,
        HERO_SHEET_REFERENCE,
        HERO_SELECTED_IDENTITY,
        HERO_APPEARANCE_ONLY,
        HERO_EXPRESSION_IDENTITY,
        HERO_NUMBERED_FIGURE,
        HERO_NUMBERED_FIGURE_END,
        HERO_SHEET_COUNT,
        HERO_SHEET_FIGURES,
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
