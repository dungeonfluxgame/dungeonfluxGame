//! What the scene builder is told about a scene besides its cast: the place, the stored
//! description, the set pieces and what to look for on the plate (D.8 to D.11, D12 to D15).
//!
//! It owns `SceneBrief` (the scene record's place, optional stored description, place music and
//! set), `SetPiece` (a named thing with a stated position, the noun phrases Locate may try for it
//! and its own picture) and `LocatePhrase`, a validated noun phrase. The cast is not here: the
//! plate job reads it from the table snapshot, so the people in the picture and the people the
//! table knows cannot disagree (RES6). It never decides what a phrase means: a phrase is words
//! for the segmentation model, written by the dress step and never taken from a player.

use std::fmt;

use crate::element::ElementId;
use crate::identity::SceneId;
use crate::limits::MAXIMUM_NAME_LENGTH;
use crate::private_data::{LengthClass, Measured, Narration};
use crate::table_state::{DisplayName, MediaName, Placement};

/// How many noun phrases Locate tries for one item before it falls back to the item's own
/// picture: the S2 result (73 percent found on the first phrase, 87 percent after one retry with
/// a plain head noun) and a third try for the last stragglers (O.16, S2 result).
pub(crate) const MAXIMUM_PHRASES_PER_ITEM: usize = 3;

/// Why a phrase was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhraseError {
    Blank,
    TooLong,
    ControlCharacter,
}

impl fmt::Display for PhraseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blank => formatter.write_str("a phrase cannot be blank"),
            Self::TooLong => write!(
                formatter,
                "a phrase is at most {MAXIMUM_NAME_LENGTH} characters"
            ),
            Self::ControlCharacter => formatter.write_str("a phrase has no control characters"),
        }
    }
}

impl std::error::Error for PhraseError {}

/// A noun phrase for the segmentation model, such as `paper lantern`. Its `Debug` says only a
/// length class, like every other text a model writes.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocatePhrase(String);

impl LocatePhrase {
    pub fn new(text: &str) -> Result<Self, PhraseError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(PhraseError::Blank);
        }
        if text.chars().any(char::is_control) {
            return Err(PhraseError::ControlCharacter);
        }
        if text.chars().count() > MAXIMUM_NAME_LENGTH {
            return Err(PhraseError::TooLong);
        }
        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for LocatePhrase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let length = LengthClass::of_length(self.0.length_in_chars());
        write!(formatter, "LocatePhrase(length={})", length.key())
    }
}

/// Why a set piece could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetPieceError {
    TooManyPhrases { given: usize },
}

impl fmt::Display for SetPieceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyPhrases { given } => write!(
                formatter,
                "an item has at most {MAXIMUM_PHRASES_PER_ITEM} phrases, not {given}"
            ),
        }
    }
}

impl std::error::Error for SetPieceError {}

/// One thing of the set (D.11): a prop, a piece of furniture, a ware. A piece with phrases is an
/// item Locate looks for on the plate; a piece without any is dressing the plate shows and the
/// engine never points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetPiece {
    id: ElementId,
    name: DisplayName,
    position: Option<Placement>,
    phrases: Vec<LocatePhrase>,
    picture: Option<MediaName>,
}

impl SetPiece {
    /// A piece with no stated position, no phrases and no picture of its own.
    pub fn new(id: ElementId, name: DisplayName) -> Self {
        Self {
            id,
            name,
            position: None,
            phrases: Vec::new(),
            picture: None,
        }
    }

    pub fn at(mut self, position: Placement) -> Self {
        self.position = Some(position);
        self
    }

    /// The phrases Locate tries, best first: the item's own name, then a plain head noun.
    pub fn locating_by(mut self, phrases: Vec<LocatePhrase>) -> Result<Self, SetPieceError> {
        if phrases.len() > MAXIMUM_PHRASES_PER_ITEM {
            return Err(SetPieceError::TooManyPhrases {
                given: phrases.len(),
            });
        }
        self.phrases = phrases;
        Ok(self)
    }

    /// The item's own picture: the fallback when no phrase finds it (O.16, S2 result).
    pub fn with_picture(mut self, picture: MediaName) -> Self {
        self.picture = Some(picture);
        self
    }

    pub fn id(&self) -> &ElementId {
        &self.id
    }

    pub fn name(&self) -> &DisplayName {
        &self.name
    }

    pub fn position(&self) -> Option<Placement> {
        self.position
    }

    pub fn phrases(&self) -> &[LocatePhrase] {
        &self.phrases
    }

    pub fn picture(&self) -> Option<&MediaName> {
        self.picture.as_ref()
    }

    /// Whether Locate looks for this piece.
    pub fn is_item(&self) -> bool {
        !self.phrases.is_empty()
    }
}

/// The scene record as the scene builder reads it: everything but the cast (D.11, K.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneBrief {
    scene: SceneId,
    place: DisplayName,
    description: Option<Narration>,
    place_music: Option<MediaName>,
    set: Vec<SetPiece>,
}

impl SceneBrief {
    /// A scene whose description is not stored yet, so the Describe stage writes it.
    pub fn new(scene: SceneId, place: DisplayName) -> Self {
        Self {
            scene,
            place,
            description: None,
            place_music: None,
            set: Vec::new(),
        }
    }

    /// An authored description is already stored, so Describe has nothing to ask for.
    pub fn described(mut self, description: Narration) -> Self {
        self.description = Some(description);
        self
    }

    pub fn with_music(mut self, music: MediaName) -> Self {
        self.place_music = Some(music);
        self
    }

    pub fn with_set(mut self, set: Vec<SetPiece>) -> Self {
        self.set = set;
        self
    }

    pub fn scene(&self) -> SceneId {
        self.scene
    }

    pub fn place(&self) -> &DisplayName {
        &self.place
    }

    pub fn description(&self) -> Option<&Narration> {
        self.description.as_ref()
    }

    pub fn place_music(&self) -> Option<&MediaName> {
        self.place_music.as_ref()
    }

    pub fn set(&self) -> &[SetPiece] {
        &self.set
    }
}
