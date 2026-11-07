//! Designs the scene a move arrives in: the place, the mood, the set elements and where each
//! character stands. [`SceneDesigner`] is the seam the narrative engine implements;
//! [`TableDesigner`] is the deterministic default driven by a [`DesignTable`] of content. Pure: no
//! IO, no provider.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;

use super::types::{Depth, MoveIntent, MoveKind, PartyMember, Placement, SceneDesign, Side};

/// What the designer may know beyond the move itself. Build it with struct-update syntax from
/// `Default` so a new input is a new defaulted field. `tone`, when given, replaces the mood of the
/// destination's kit; `time_of_day` and `weather` are appended to the mood when present.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesignContext {
    pub previous_place: Option<String>,
    pub tone: Option<String>,
    pub time_of_day: Option<String>,
    pub weather: Option<String>,
}

/// Why a scene could not be designed, or why a design table was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesignError {
    /// The move names no character who moves.
    NoMovers,
    /// A character who moves is not in the party.
    UnknownMover { id: String },
    /// The destination is blank.
    EmptyDestination,
    /// The design table is not valid JSON of the table's shape, or breaks a table rule.
    BadTable { reason: String },
}

impl DesignError {
    pub fn key(&self) -> &'static str {
        match self {
            Self::NoMovers => "no_movers",
            Self::UnknownMover { .. } => "unknown_mover",
            Self::EmptyDestination => "empty_destination",
            Self::BadTable { .. } => "bad_table",
        }
    }
}

impl fmt::Display for DesignError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoMovers => formatter.write_str("the move names no one who moves"),
            Self::UnknownMover { id } => write!(formatter, "{id} is not in the party"),
            Self::EmptyDestination => formatter.write_str("the move has no destination"),
            Self::BadTable { reason } => {
                write!(formatter, "the design table is not valid: {reason}")
            }
        }
    }
}

impl std::error::Error for DesignError {}

/// Designs the scene a move arrives in. The narrative engine implements this later; the default is
/// [`TableDesigner`].
pub trait SceneDesigner {
    fn design(
        &self,
        intent: &MoveIntent,
        party: &[PartyMember],
        context: &DesignContext,
    ) -> Result<SceneDesign, DesignError>;
}

/// The set and mood of one kind of place, as words for the picture.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneKit {
    mood: String,
    elements: Vec<String>,
}

/// The set kits keyed by a destination word, and the kit used when no word matches. Content, not
/// code: a new place is a new entry in the design file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignTable {
    kits: BTreeMap<String, SceneKit>,
    fallback: SceneKit,
    /// How many movers, from the front of the party's order, stand in the foreground; the rest
    /// stand in the middle.
    #[serde(default = "default_foreground_movers")]
    foreground_movers: usize,
}

fn default_foreground_movers() -> usize {
    2
}

impl DesignTable {
    /// The authored default, so the designer works with no file.
    pub fn builtin() -> Self {
        Self {
            kits: BTreeMap::from([
                kit_entry(
                    "balcony",
                    "cool night air",
                    &["stone balustrade", "hanging lanterns", "the city beyond"],
                ),
                kit_entry(
                    "door",
                    "close and expectant",
                    &[
                        "a studded wooden door",
                        "iron hinges",
                        "a worn stone threshold",
                    ],
                ),
                kit_entry(
                    "hall",
                    "hushed and echoing",
                    &["tall stone pillars", "a long runner rug", "wall sconces"],
                ),
                kit_entry(
                    "hallway",
                    "hushed and echoing",
                    &["tall stone pillars", "a long runner rug", "wall sconces"],
                ),
                kit_entry(
                    "square",
                    "open and busy",
                    &["a market well", "cobbled paving", "shuttered shopfronts"],
                ),
                kit_entry(
                    "tavern",
                    "warm and smoky",
                    &["a long oak bar", "low tables", "a crackling hearth"],
                ),
                kit_entry(
                    "cellar",
                    "damp and dim",
                    &["barrels on a rack", "low vaulted brick", "a single lantern"],
                ),
            ]),
            fallback: kit("even and unremarkable", &["bare walls", "a plain floor"]),
            foreground_movers: default_foreground_movers(),
        }
    }

    /// Reads a design table from JSON. A malformed file, an unknown field, a key that is not a
    /// lowercase word, or a blank mood is refused.
    pub fn from_json_str(text: &str) -> Result<Self, DesignError> {
        let table: Self = serde_json::from_str(text).map_err(|error| DesignError::BadTable {
            reason: error.to_string(),
        })?;
        table.validate()?;
        Ok(table)
    }

    fn validate(&self) -> Result<(), DesignError> {
        for (word, kit) in &self.kits {
            let is_word = !word.is_empty()
                && word.chars().all(char::is_alphabetic)
                && word.to_lowercase() == *word;
            if !is_word {
                return Err(DesignError::BadTable {
                    reason: format!("kit key `{word}` is not a lowercase word"),
                });
            }
            validate_mood(&kit.mood)?;
        }
        validate_mood(&self.fallback.mood)
    }

    /// The kit of the first destination word, in the order the words are said, that has one.
    fn kit_for(&self, destination: &str) -> &SceneKit {
        destination
            .split(|character: char| !character.is_alphabetic())
            .filter(|word| !word.is_empty())
            .map(str::to_lowercase)
            .find_map(|word| self.kits.get(word.as_str()))
            .unwrap_or(&self.fallback)
    }
}

fn kit(mood: &str, elements: &[&str]) -> SceneKit {
    SceneKit {
        mood: mood.to_owned(),
        elements: elements
            .iter()
            .map(|element| (*element).to_owned())
            .collect(),
    }
}

fn kit_entry(word: &str, mood: &str, elements: &[&str]) -> (String, SceneKit) {
    (word.to_owned(), kit(mood, elements))
}

fn validate_mood(mood: &str) -> Result<(), DesignError> {
    if mood.trim().is_empty() {
        return Err(DesignError::BadTable {
            reason: "a kit has a blank mood".to_owned(),
        });
    }
    Ok(())
}

/// The deterministic default designer: the same move and party always give the same design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableDesigner {
    table: DesignTable,
}

impl TableDesigner {
    pub fn new(table: DesignTable) -> Self {
        Self { table }
    }
}

impl SceneDesigner for TableDesigner {
    fn design(
        &self,
        intent: &MoveIntent,
        party: &[PartyMember],
        context: &DesignContext,
    ) -> Result<SceneDesign, DesignError> {
        if intent.who.is_empty() {
            return Err(DesignError::NoMovers);
        }
        if intent.to.trim().is_empty() {
            return Err(DesignError::EmptyDestination);
        }
        if let Some(id) = intent
            .who
            .iter()
            .find(|id| !party.iter().any(|member| member.id == **id))
        {
            return Err(DesignError::UnknownMover { id: id.clone() });
        }

        let scene_kit = self.table.kit_for(&intent.to);
        let placements = intent
            .who
            .iter()
            .enumerate()
            .map(|(index, id)| Placement {
                character: id.clone(),
                depth: depth_for(index, self.table.foreground_movers),
                side: side_for(index),
                doing: doing_for(intent.kind).to_owned(),
            })
            .collect();
        let left_behind = party
            .iter()
            .filter(|member| !intent.who.contains(&member.id))
            .map(|member| member.id.clone())
            .collect();

        Ok(SceneDesign {
            place: intent.to.clone(),
            mood: mood_for(scene_kit, context),
            elements: scene_kit.elements.clone(),
            placements,
            left_behind,
        })
    }
}

fn mood_for(scene_kit: &SceneKit, context: &DesignContext) -> String {
    std::iter::once(context.tone.as_deref().unwrap_or(&scene_kit.mood))
        .chain(context.time_of_day.as_deref())
        .chain(context.weather.as_deref())
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

fn depth_for(index: usize, foreground_movers: usize) -> Depth {
    if index < foreground_movers {
        Depth::Foreground
    } else {
        Depth::Middle
    }
}

fn side_for(index: usize) -> Side {
    match index % 3 {
        0 => Side::Left,
        1 => Side::Right,
        _ => Side::Centre,
    }
}

fn doing_for(kind: MoveKind) -> &'static str {
    match kind {
        MoveKind::Walk => "steps out",
        MoveKind::Enter => "enters",
        MoveKind::Run => "hurries in",
        MoveKind::Climb => "climbs up",
        MoveKind::Descend => "climbs down",
        MoveKind::Sneak => "creeps in",
    }
}
