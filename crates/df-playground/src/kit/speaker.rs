//! The off-picture speaker card (component 14): a glass panel at the bottom right with a round
//! face in a gold ring, the name and the line or the thinking dots, for a speaker who is not in
//! the picture. It follows the bubble look of renders 03, 12 and 18.
//!
//! Who gets a card, and its twin rule, stay with `speaker_card`; this is the card's look.

use serde::{Deserialize, Serialize};

use crate::kit::shown;
use crate::lobby::model::BubbleTone;
use crate::speaker_card::{PortraitCrop, SpeakerCard};

/// The card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeakerPanel {
    /// The shown name, never an id.
    pub name: String,
    /// The line, or none for the thinking dots.
    #[serde(default)]
    pub line: Option<String>,
    /// A face from the media library.
    #[serde(default)]
    pub portrait: Option<String>,
    /// The cut of `portrait` round the head, when the face is a crop of a room picture
    /// (`speaker_card::portrait_crop`). It is not read from a scene file until `PortraitCrop` has
    /// a serde form (`speaker_card` owns it).
    #[serde(skip)]
    pub crop: Option<PortraitCrop>,
    /// The rim's tone; the rose of a character the table talks to when left out.
    #[serde(default = "npc")]
    pub tone: BubbleTone,
    /// A copy of a visible speaker's line shown while the plate changes.
    #[serde(default)]
    pub twin: bool,
}

fn npc() -> BubbleTone {
    BubbleTone::Npc
}

impl From<&SpeakerCard> for SpeakerPanel {
    fn from(card: &SpeakerCard) -> Self {
        Self {
            name: card.name.clone(),
            line: card.line.clone(),
            portrait: card
                .portrait
                .as_ref()
                .map(|portrait| portrait.media.clone()),
            crop: card.portrait.as_ref().map(|portrait| portrait.crop),
            tone: BubbleTone::Npc,
            twin: card.twin,
        }
    }
}

impl SpeakerPanel {
    pub fn name(&self) -> &str {
        self.name.trim()
    }

    pub fn line(&self) -> Option<&str> {
        shown(self.line.as_deref())
    }

    /// The rim's tone: the thought tone has no rim of its own, so it takes the default.
    pub fn rim_tone(&self) -> BubbleTone {
        match self.tone {
            BubbleTone::Thought => BubbleTone::Npc,
            tone => tone,
        }
    }
}
