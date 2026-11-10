//! The plate brief: a `SceneDesign` and the characters' look sheets become the image prompt and one
//! reference card per placed character, by a fixed template. Pure and deterministic. A prompt over
//! the image service's limit is refused, never cut short.

use std::collections::BTreeMap;
use std::fmt;

use super::types::{
    Depth, Placement, PlateBrief, Pose, ReferenceCard, RouteSight, SceneDesign, Side,
};
use crate::imagegen::limits::MAXIMUM_SENT_TEXT_CHARS;

/// A character who may be placed in the picture: the persona id, the display name (for the
/// scan that names them in the picture, never for the prompt: a name painted by the image model
/// ends up on a signboard), the job that stands in when there is no look sheet, a library portrait
/// when one exists, and the names of what the design shows them with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefCharacter {
    pub id: String,
    pub display_name: String,
    pub job: String,
    pub portrait: Option<String>,
    pub carrying: Vec<String>,
}

/// The look of each character in words, by persona id. A content file the host loads; a character
/// without an entry is described by the job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookSheets {
    looks: BTreeMap<String, String>,
}

impl LookSheets {
    pub fn empty() -> Self {
        Self {
            looks: BTreeMap::new(),
        }
    }

    pub fn from_json_str(text: &str) -> Result<Self, BriefError> {
        let looks: BTreeMap<String, String> =
            serde_json::from_str(text).map_err(|error| BriefError::BadLooks {
                reason: error.to_string(),
            })?;
        if let Some((id, _)) = looks.iter().find(|(_, look)| look.trim().is_empty()) {
            return Err(BriefError::BadLooks {
                reason: format!("the look of '{id}' is empty"),
            });
        }
        Ok(Self { looks })
    }

    pub fn look_of(&self, id: &str) -> Option<&str> {
        self.looks.get(id).map(String::as_str)
    }
}

/// Why a brief cannot be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BriefError {
    /// A placement names a character who is not in the cast.
    UnknownCharacter { id: String },
    /// The look sheet content is not a JSON object of id to non-empty look.
    BadLooks { reason: String },
    /// The prompt is longer than the image service accepts.
    PromptTooLong { length: usize, maximum: usize },
}

impl BriefError {
    pub fn key(&self) -> &'static str {
        match self {
            Self::UnknownCharacter { .. } => "unknown_character",
            Self::BadLooks { .. } => "bad_looks",
            Self::PromptTooLong { .. } => "prompt_too_long",
        }
    }
}

impl fmt::Display for BriefError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCharacter { id } => {
                write!(
                    formatter,
                    "'{id}' is placed in the scene but is not in the cast"
                )
            }
            Self::BadLooks { reason } => write!(formatter, "the look sheets are bad: {reason}"),
            Self::PromptTooLong { length, maximum } => write!(
                formatter,
                "the picture prompt is {length} characters; the limit is {maximum}"
            ),
        }
    }
}

impl std::error::Error for BriefError {}

/// A placement with its character and the look the picture uses for them.
struct PlacedCharacter<'a> {
    placement: &'a Placement,
    character: &'a BriefCharacter,
    look: &'a str,
}

/// The default-style brief. Characters who stayed behind are not in the picture.
pub fn build_brief(
    design: &SceneDesign,
    characters: &[BriefCharacter],
    looks: &LookSheets,
) -> Result<PlateBrief, BriefError> {
    build_brief_with(&BriefStyle::default(), design, characters, looks)
}

/// The fixed template: the style's direction, the scene ([`scene_words`]) and how writing shows;
/// the brief carries what the picture must not show.
pub fn build_brief_with(
    style: &BriefStyle,
    design: &SceneDesign,
    characters: &[BriefCharacter],
    looks: &LookSheets,
) -> Result<PlateBrief, BriefError> {
    let placed = placed_characters(design, characters, looks)?;
    let prompt = format!(
        "{} {}{}",
        style.direction,
        scene_text(design, &placed),
        style.writing
    );
    check_length(&prompt)?;
    let references = placed
        .iter()
        .map(|entry| reference_card(entry.character, entry.look))
        .collect();
    Ok(PlateBrief {
        prompt,
        avoid: style.avoid.clone(),
        references,
    })
}

/// Refuses a prompt longer than the image service accepts.
fn check_length(prompt: &str) -> Result<(), BriefError> {
    let length = prompt.chars().count();
    if length > MAXIMUM_SENT_TEXT_CHARS {
        return Err(BriefError::PromptTooLong {
            length,
            maximum: MAXIMUM_SENT_TEXT_CHARS,
        });
    }
    Ok(())
}

/// The brief of one sight on the way to the place `design` describes: a wide view of the sight
/// with the movers going through it, painted in the town's look and after the movers' sheets
/// alone (no things, no items, no residents), so it is cheap to make and the same town stands in
/// every sight and in the place.
pub fn build_stage_brief(
    design: &SceneDesign,
    stage: &RouteSight,
    movers: &[BriefCharacter],
    looks: &LookSheets,
) -> Result<PlateBrief, BriefError> {
    let style = BriefStyle::default();
    let people: Vec<(&BriefCharacter, &str)> = movers
        .iter()
        .map(|character| (character, look_or_job(looks, character)))
        .collect();
    let mut prompt = format!("{} ", style.direction);
    if let Some(look) = &design.town_look {
        prompt.push_str(&format!(
            "The town, the same in every picture of it: {look}. "
        ));
    }
    prompt.push_str(&format!(
        "On the way through the town, a wide view of {}. ",
        stage.sight.trim().trim_end_matches('.')
    ));
    for (_, look) in &people {
        prompt.push_str(&format!("A person ({look}) {}. ", pose_words(stage.pose)));
    }
    prompt.push_str(&cast_words(people.len()));
    prompt.push_str(&style.writing);
    check_length(&prompt)?;
    Ok(PlateBrief {
        prompt,
        avoid: style.avoid,
        references: people
            .iter()
            .map(|(character, look)| reference_card(character, look))
            .collect(),
    })
}

fn reference_card(character: &BriefCharacter, look: &str) -> ReferenceCard {
    ReferenceCard {
        character: character.id.clone(),
        look: look.to_owned(),
        portrait: character.portrait.clone(),
    }
}

/// Who is in a view on the way: only the movers described, each once.
fn cast_words(count: usize) -> String {
    match count {
        1 => "The cast in view is one person, the one described, who appears once and only once; \
              nobody else is in the picture. "
            .to_owned(),
        _ => format!(
            "The cast in view is {count} people, the ones described, each once; nobody else is \
             in the picture. "
        ),
    }
}

fn pose_words(pose: Pose) -> &'static str {
    match pose {
        Pose::Walking => "is walking on through it, mid-stride, small in the wide view",
        Pose::Pausing => "has stopped there to take it in, small in the wide view",
    }
}

/// What the picture of `design` shows, in the words of its prompt without the style around them:
/// the place, mood, elements and things in sight, and one sentence per placed character. A frame
/// judge reads it as the picture now on screen.
pub fn scene_words(
    design: &SceneDesign,
    characters: &[BriefCharacter],
    looks: &LookSheets,
) -> Result<String, BriefError> {
    let placed = placed_characters(design, characters, looks)?;
    Ok(scene_text(design, &placed).trim_end().to_owned())
}

/// Each placement of `design` with its character and look.
fn placed_characters<'a>(
    design: &'a SceneDesign,
    characters: &'a [BriefCharacter],
    looks: &'a LookSheets,
) -> Result<Vec<PlacedCharacter<'a>>, BriefError> {
    design
        .placements
        .iter()
        .map(|placement| {
            let character = characters
                .iter()
                .find(|character| character.id == placement.character)
                .ok_or_else(|| BriefError::UnknownCharacter {
                    id: placement.character.clone(),
                })?;
            Ok(PlacedCharacter {
                placement,
                character,
                look: look_or_job(looks, character),
            })
        })
        .collect()
}

fn look_or_job<'a>(looks: &'a LookSheets, character: &'a BriefCharacter) -> &'a str {
    looks.look_of(&character.id).unwrap_or(&character.job)
}

/// The words that frame every picture, split as image backends take them: how it looks, said as
/// what it is (`direction`, and `writing`: writing a thing carries shows as unreadable ink, never
/// as nothing), and what no picture shows (`avoid`, plain terms), which a backend with a negative
/// prompt takes there and any other reads as a line of the text (`ImageRequest::avoiding`). A
/// campaign or scene can set its own without a code change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefStyle {
    pub direction: String,
    pub writing: String,
    pub avoid: Vec<String>,
}

impl Default for BriefStyle {
    fn default() -> Self {
        Self {
            // Every rule in few words: the Forge studio takes 4000 characters of text in all, and
            // a room plate also carries its scene, its things and the references' lines.
            direction: concat!(
                "Unretouched photo, live-action medieval TV drama set, 35mm f/4, ISO 800; candid ",
                "mid-scene, on the main faces and reactions, people turned to each other or their ",
                "task. Real skin: pores, fine hairs, stubble, blemishes, wrinkles, uneven tone. ",
                "Worn wool, linen, fur, mail, leather, weave and dirt showing; timber, plaster, ",
                "stone; only the place's daylight, candle or fire. Natural lens sharpness and ",
                "contrast; fine grain; TV depth of field; neutral grade. Metal and wet stone ",
                "dull; skin, cloth, wood, leather, vellum, paper matte and dry; only water and ",
                "what it wets look wet. Plain, worn, everyday things of the time. A still of"
            )
            .to_owned(),
            writing: concat!(
                "A thing's own writing (page, seal, carved sign) is faded, smudged ink, ",
                "unreadable but there."
            )
            .to_owned(),
            avoid: [
                "painting",
                "brushwork",
                "illustration",
                "digital art",
                "3D render",
                "airbrushed, waxy, plastic or glowing skin",
                "HDR",
                "clarity or smoothing filter",
                "amber, golden or teal-orange grade",
                "sparkles",
                "star glints",
                "glossy sheen",
                "strong highlights",
                "fantasy art",
                "glowing magic",
                "runes",
                "sigils",
                "gilding",
                "finery",
                "ornate armor",
                "god rays",
                "cathedral grandeur",
                "electric light",
                "glass mugs",
                "drum kits",
                "modern clothes",
                "people posed in a row",
                "people looking at the camera",
                "extra people beyond those described (crowds, bystanders)",
                "captions",
                "speech bubbles",
                "logos",
                "interface",
                "readable text",
            ]
            .map(str::to_owned)
            .to_vec(),
        }
    }
}

fn scene_text(design: &SceneDesign, placed: &[PlacedCharacter<'_>]) -> String {
    let mut text = format!("{}. Mood: {}. ", design.place, design.mood);
    if let Some(look) = &design.town_look {
        text.push_str(&format!(
            "The town, the same in every picture of it: {look}. "
        ));
    }
    if !design.elements.is_empty() {
        text.push_str(&format!(
            "Elements in the scene: {}. ",
            design.elements.join("; ")
        ));
    }
    if !design.things.is_empty() {
        let things: Vec<String> = design
            .things
            .iter()
            .map(|thing| format!("{} ({})", thing.name, thing.look))
            .collect();
        text.push_str(&format!("In plain sight: {}. ", things.join("; ")));
    }
    for entry in placed {
        text.push_str(&placement_sentence(entry));
        text.push(' ');
    }
    text
}

/// One sentence for a placed character, by their look and never by their name or id: the picture
/// is painted from it, and a name in it comes out as writing on a sign.
fn placement_sentence(entry: &PlacedCharacter<'_>) -> String {
    let placement = entry.placement;
    let carrying = match entry.character.carrying.as_slice() {
        [] => String::new(),
        names => format!(", with their {} in view", names.join(" and ")),
    };
    format!(
        "A person ({}) stands in the {} {}, {}{carrying}.",
        entry.look,
        depth_words(placement.depth),
        side_words(placement.side),
        placement.doing.trim().trim_end_matches('.'),
    )
}

fn depth_words(depth: Depth) -> &'static str {
    match depth {
        Depth::Foreground => "foreground",
        Depth::Middle => "middle",
        Depth::Background => "background",
    }
}

fn side_words(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Centre => "centre",
        Side::Right => "right",
    }
}
