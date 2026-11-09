//! The Dungeon Master's narration box (component 4) and its danger tone, the narrator's twist card
//! (component 5): an octagon of warm glass with an ember-to-amethyst rim, an optional die
//! medallion, the voice wave and speaker between rules, the line with server-marked highlights,
//! an italic last line and a continue hint (renders 02, 04, 13, 35).
//!
//! It keeps the caption's contract of `overlay::NarrationBox`: it never moves with the camera,
//! nothing is drawn over it, and the hint is a hint only, since the shared screen takes no input.
//!
//! The header has two styles. The centre box wears render 02's: the name and the voice wave after
//! it inside a chamfered hairline tab whose foot runs on as the header's rule. The right, compact
//! and twist boxes wear the plain one of renders 13, 04 and 35: the wave before the name. The die
//! medallion is the d20 drawing alone, without a numeral on its face.

use serde::{Deserialize, Serialize};

use crate::home::glyph::HomeIcon;
use crate::kit::{PanelTone, shown};
use crate::overlay::rules::{DEFAULT_NARRATOR_LABEL, progress_text};
use crate::overlay::{NarrationBox, NarrationPlacement, Progress};

/// Where the box sits across the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NarrationPlace {
    #[default]
    Center,
    /// Against the right edge, leaving the left for a figure (render 13).
    Right,
    /// A compact box at the lower left with an italic header (render 04).
    Left,
}

impl NarrationPlace {
    /// The word that ends the CSS class, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Right => "right",
            Self::Left => "left",
        }
    }
}

/// How the header of the box is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NarrationHeader {
    /// The name and the wave after it in a chamfered hairline tab, the rule running on from its
    /// foot (render 02).
    Tab,
    /// The wave before the name and the rule after it (renders 13, 04 and 35).
    Plain,
}

impl NarrationHeader {
    /// The word that ends the CSS class, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Tab => "tab",
            Self::Plain => "plain",
        }
    }
}

/// How the continue hint is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinueStyle {
    /// A small gold chevron inside the box's lower right (render 02).
    #[default]
    Chevron,
    /// A gold ring with the label under it (render 13).
    Ring,
}

impl ContinueStyle {
    /// The word that ends the CSS class, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Chevron => "chevron",
            Self::Ring => "ring",
        }
    }
}

/// A piece of the line, highlighted when the server marked it (a place name, say). The page never
/// looks for words to highlight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineSegment {
    pub text: String,
    #[serde(default)]
    pub highlight: bool,
}

/// The narration box.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Narration {
    /// "Dungeon Master" when left out.
    #[serde(default)]
    pub speaker: Option<String>,
    /// The whole line: what is spoken, timed and read by a screen reader.
    pub text: String,
    /// The line cut into highlighted and plain pieces. Drawn only when the pieces make up `text`
    /// exactly, so a highlight can never change what is said.
    #[serde(default)]
    pub segments: Vec<LineSegment>,
    /// A last line in italics, such as the twist's warning.
    #[serde(default)]
    pub emphasis: Option<String>,
    /// The round medallion at the left (a die, for the Dungeon Master).
    #[serde(default)]
    pub badge: Option<HomeIcon>,
    /// Whether the line is being spoken, which draws the voice wave.
    #[serde(default = "spoken")]
    pub voice: bool,
    /// The word of the continue hint; `None` draws no hint.
    #[serde(default)]
    pub continue_label: Option<String>,
    #[serde(default)]
    pub continue_style: ContinueStyle,
    #[serde(default)]
    pub progress: Option<Progress>,
    #[serde(default)]
    pub placement: NarrationPlace,
    #[serde(default)]
    pub tone: PanelTone,
    /// The header's style; the centre box's gold tone has the tab, every other box the plain one.
    #[serde(default)]
    pub header: Option<NarrationHeader>,
}

fn spoken() -> bool {
    true
}

/// The continue hint of a box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContinueHint<'a> {
    pub style: ContinueStyle,
    pub label: &'a str,
}

impl From<&NarrationBox> for Narration {
    fn from(narration: &NarrationBox) -> Self {
        Self {
            speaker: narration.speaker.clone(),
            text: narration.text.clone(),
            segments: Vec::new(),
            emphasis: None,
            badge: narration.badge.map(HomeIcon::Overlay),
            voice: narration.voice,
            continue_label: narration.continue_label.clone(),
            continue_style: match narration.placement {
                NarrationPlacement::Center => ContinueStyle::Chevron,
                NarrationPlacement::Right => ContinueStyle::Ring,
            },
            progress: narration.progress.clone(),
            placement: match narration.placement {
                NarrationPlacement::Center => NarrationPlace::Center,
                NarrationPlacement::Right => NarrationPlace::Right,
            },
            tone: PanelTone::Gold,
            header: None,
        }
    }
}

impl Narration {
    /// The name in the header.
    pub fn speaker(&self) -> &str {
        shown(self.speaker.as_deref()).unwrap_or(DEFAULT_NARRATOR_LABEL)
    }

    /// The pieces the line is drawn as: the segments when they spell the line exactly, otherwise
    /// the whole line as one plain piece.
    pub fn segments(&self) -> Vec<LineSegment> {
        let joined: String = self
            .segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect();
        if !self.segments.is_empty() && joined == self.text {
            return self
                .segments
                .iter()
                .filter(|segment| !segment.text.is_empty())
                .cloned()
                .collect();
        }
        vec![LineSegment {
            text: self.text.clone(),
            highlight: false,
        }]
    }

    pub fn emphasis(&self) -> Option<&str> {
        shown(self.emphasis.as_deref())
    }

    /// The header's style: the one given, else the tab on the centre box and the plain header on
    /// the others and on the twist card.
    pub fn header_style(&self) -> NarrationHeader {
        self.header.unwrap_or(
            if self.placement == NarrationPlace::Center && self.tone == PanelTone::Gold {
                NarrationHeader::Tab
            } else {
                NarrationHeader::Plain
            },
        )
    }

    pub fn continue_hint(&self) -> Option<ContinueHint<'_>> {
        shown(self.continue_label.as_deref()).map(|label| ContinueHint {
            style: self.continue_style,
            label,
        })
    }

    /// "Scene 2 of 5: Introduction" in the header, when the box carries progress.
    pub fn progress_text(&self) -> Option<String> {
        self.progress.as_ref().map(progress_text)
    }

    /// The compact box has an italic header.
    pub fn is_compact(&self) -> bool {
        self.placement == NarrationPlace::Left
    }

    /// The twist card sits under the top bar, as a warning over the scene (render 35); every
    /// other box sits in the bottom stack, above the choices and the party.
    pub fn sits_on_top(&self) -> bool {
        self.tone == PanelTone::Danger
    }
}
