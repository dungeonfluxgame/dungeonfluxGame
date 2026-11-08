//! The plate brief: a `SceneDesign` and the characters' look sheets become the image prompt and one
//! reference card per placed character, by a fixed template. Pure and deterministic. A prompt over
//! the image service's limit is refused, never cut short.

use std::collections::BTreeMap;
use std::fmt;

use super::types::{Depth, PlateBrief, Placement, ReferenceCard, SceneDesign, Side};
use crate::imagegen::limits::MAXIMUM_SENT_TEXT_CHARS;

/// A character who may be placed in the picture: the persona id, the name the prompt uses, the job
/// that stands in when there is no look sheet, and a library portrait when one exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefCharacter {
    pub id: String,
    pub display_name: String,
    pub job: String,
    pub portrait: Option<String>,
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
                write!(formatter, "'{id}' is placed in the scene but is not in the cast")
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

/// The fixed template: the style's direction, the place, mood and elements, one sentence per
/// placed character, and the style's ban on writing.
pub fn build_brief_with(
    style: &BriefStyle,
    design: &SceneDesign,
    characters: &[BriefCharacter],
    looks: &LookSheets,
) -> Result<PlateBrief, BriefError> {
    let mut placed = Vec::with_capacity(design.placements.len());
    for placement in &design.placements {
        let character = characters
            .iter()
            .find(|character| character.id == placement.character)
            .ok_or_else(|| BriefError::UnknownCharacter {
                id: placement.character.clone(),
            })?;
        placed.push(PlacedCharacter {
            placement,
            character,
            look: look_or_job(looks, character),
        });
    }
    let prompt = prompt_text(style, design, &placed);
    let length = prompt.chars().count();
    if length > MAXIMUM_SENT_TEXT_CHARS {
        return Err(BriefError::PromptTooLong {
            length,
            maximum: MAXIMUM_SENT_TEXT_CHARS,
        });
    }
    let references = placed
        .iter()
        .map(|entry| ReferenceCard {
            character: entry.character.id.clone(),
            look: entry.look.to_owned(),
            portrait: entry.character.portrait.clone(),
        })
        .collect();
    Ok(PlateBrief { prompt, references })
}

fn look_or_job<'a>(looks: &'a LookSheets, character: &'a BriefCharacter) -> &'a str {
    looks.look_of(&character.id).unwrap_or(&character.job)
}

/// The words that frame every picture: how it is painted and the ban on writing in it. A campaign
/// or scene can set its own without a code change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefStyle {
    pub direction: String,
    pub writing_ban: String,
}

impl Default for BriefStyle {
    fn default() -> Self {
        Self {
            direction: concat!(
                "A still from a glossy TV period soap opera set in a medieval Dungeons and ",
                "Dragons world, photorealistic live action shot on 35mm: an intimate medium shot ",
                "framed on the main characters' faces and reactions, soft flattering candlelit ",
                "key light with a warm glow on skin, creamy shallow depth of field, a Hollywood ",
                "teal and orange color grade, real actors with natural skin in believable, ",
                "lived-in medieval clothes of linen, wool and leather, in a real medieval set of ",
                "timber, stone and candlelight; nothing modern (no electric light, glass mugs, ",
                "drum kits or modern clothes) and no high-fantasy art (no glowing magic, ornate ",
                "armor or painterly look), of"
            )
            .to_owned(),
            writing_ban: "No text, letters, captions, speech bubbles, logos or interface of any kind."
                .to_owned(),
        }
    }
}

fn prompt_text(
    style: &BriefStyle,
    design: &SceneDesign,
    placed: &[PlacedCharacter<'_>],
) -> String {
    let mut text = format!("{} {}. Mood: {}. ", style.direction, design.place, design.mood);
    if !design.elements.is_empty() {
        text.push_str(&format!(
            "Elements in the scene: {}. ",
            design.elements.join("; ")
        ));
    }
    for entry in placed {
        text.push_str(&placement_sentence(entry));
        text.push(' ');
    }
    text.push_str(&style.writing_ban);
    text
}

fn placement_sentence(entry: &PlacedCharacter<'_>) -> String {
    let placement = entry.placement;
    format!(
        "{} ({}) stands in the {} {}, {}.",
        entry.character.display_name,
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
