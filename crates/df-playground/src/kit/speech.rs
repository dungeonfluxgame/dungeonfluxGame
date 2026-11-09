//! The speech bubble (component 7) and the thinking cue (component 22): an octagon of glass with a
//! tail, a rim and halo in the speaker's tone, the speaker's glyph or face and name in a head row
//! or a tab, and the line (renders 03, 04, 12, 18, 26 to 35).
//!
//! Placement stays with `view::bubble_layout` and the anchor rules; this is the bubble's look. A
//! free bubble is drawn at the percents it was given, its width at least the narrowest bubble and
//! its tail off the chamfered ends; it is never moved here. Keeping it clear of the chrome is
//! `view::bubble_layout`'s, which measures the laid-out boxes against its keep-out list: the
//! kit's `.kit-bubble`, `.kit-top`, `.kit-bottom`, `.kit-column` and `.kit-place` are the classes
//! to add to that list when the kit reaches the live screen. A bubble with no words is the
//! thinking cue: three pulsing dots, the speaker named for a screen reader only.

use serde::{Deserialize, Serialize};

use crate::home::glyph::HomeIcon;
use crate::kit::{Fill, shown};
use crate::lobby::model::{Bubble, BubbleTone};

/// The narrowest a bubble is drawn, in percent of the screen's width, so a line always has room.
pub const BUBBLE_WIDTH_PERCENT_AT_LEAST: u8 = 8;
/// How near the bubble's ends its tail may sit, in percent of its width.
pub const TAIL_PERCENT_FROM_END: u8 = 10;

/// Where the speaker's name is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameStyle {
    /// A head row with the glyph or face and the name.
    #[default]
    Head,
    /// A small chamfered tab on the top edge (renders 17 and 20). A name longer than the bubble
    /// wraps in the tab, which grows upward; how it wraps is the stylesheet's.
    Tab,
}

impl NameStyle {
    /// The word that ends the CSS class, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Tab => "tab",
        }
    }
}

/// Which edge the tail leaves from. A tail on the left or right edge is for a bubble beside the
/// speaker's face, which `view::bubble_layout` places and keeps in step with the shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TailSide {
    #[default]
    Below,
    Above,
    Left,
    Right,
}

/// Which way the tail's tip leans from the middle of its base: toward the start (left, or up on a
/// side tail) or the end of the edge, toward the speaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TailLean {
    Left,
    Right,
}

impl TailSide {
    /// The word that ends the CSS class, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Below => "below",
            Self::Above => "above",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// Where a free bubble sits, in percent of the screen, and where along its edge the tail leaves.
/// The bubble hangs from its tail's tip: a tail below holds the box's bottom edge at
/// `tail_y_percent`, so a longer line grows the box upward, away from the speaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BubblePlace {
    pub left_percent: u8,
    /// The height of the tail's tip on the screen.
    pub tail_y_percent: u8,
    pub width_percent: u8,
    #[serde(default = "default_tail_percent")]
    pub tail_percent: u8,
}

fn default_tail_percent() -> u8 {
    30
}

impl BubblePlace {
    /// The tail's distance from the bubble's left edge in percent of the screen's width: the
    /// place's width is the most the bubble is drawn, and a bubble shrinks to its words, so the
    /// tail is held where its place put it, not at a share of the drawn width.
    pub fn tail_offset_percent(&self) -> f32 {
        f32::from(self.width_percent) * f32::from(self.tail_percent) / 100.0
    }
}

/// One bubble.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeechBubble {
    pub speaker: String,
    /// The words; none (or the thought tone) draws the thinking cue.
    #[serde(default)]
    pub line: Option<String>,
    #[serde(default)]
    pub emphasis: Option<String>,
    #[serde(default)]
    pub glyph: Option<HomeIcon>,
    /// A face from the media library, drawn instead of the glyph.
    #[serde(default)]
    pub portrait: Option<String>,
    #[serde(default)]
    pub tone: BubbleTone,
    /// A `#rrggbb` rim colour that replaces the tone's (render 12's verdigris "Seris").
    #[serde(default)]
    pub accent: Option<String>,
    #[serde(default)]
    pub name_style: NameStyle,
    #[serde(default)]
    pub tail: TailSide,
    /// The tip's lean; when left out, the side of the box the tail's place is on decides it.
    #[serde(default)]
    pub lean: Option<TailLean>,
    /// Where a free bubble sits; an anchored one is placed by `view::bubble_layout`.
    #[serde(default)]
    pub place: Option<BubblePlace>,
}

impl From<&Bubble> for SpeechBubble {
    fn from(bubble: &Bubble) -> Self {
        let width = bubble.width_percent.max(1);
        let along = i32::from(bubble.tail_x_percent) - i32::from(bubble.left_percent);
        let tail_percent = (along * 100 / i32::from(width)).clamp(0, 100) as u8;
        Self {
            speaker: bubble.speaker.clone(),
            line: Some(bubble.line.clone()),
            emphasis: bubble.emphasis.clone(),
            glyph: bubble.glyph.map(HomeIcon::Overlay),
            portrait: bubble.portrait.clone(),
            tone: bubble.tone,
            accent: None,
            name_style: NameStyle::Head,
            tail: if bubble.tail_y_percent < bubble.top_percent {
                TailSide::Above
            } else {
                TailSide::Below
            },
            lean: None,
            place: Some(BubblePlace {
                left_percent: bubble.left_percent,
                tail_y_percent: bubble.tail_y_percent,
                width_percent: bubble.width_percent,
                tail_percent,
            }),
        }
    }
}

impl SpeechBubble {
    /// A bubble is a small pane of fine glass, its tail and all.
    pub fn fill(&self) -> Fill {
        Fill::Fine
    }

    pub fn is_thinking(&self) -> bool {
        self.tone == BubbleTone::Thought || shown(self.line.as_deref()).is_none()
    }

    /// The words, or `None` for the thinking cue.
    pub fn line(&self) -> Option<&str> {
        if self.tone == BubbleTone::Thought {
            return None;
        }
        shown(self.line.as_deref())
    }

    pub fn emphasis(&self) -> Option<&str> {
        self.line().and(shown(self.emphasis.as_deref()))
    }

    /// The speaker's name; the cue names them even when it shows no words.
    pub fn speaker(&self) -> &str {
        self.speaker.trim()
    }

    fn held_width_percent(&self) -> Option<u8> {
        self.place.map(|place| {
            place
                .width_percent
                .clamp(BUBBLE_WIDTH_PERCENT_AT_LEAST, 100)
        })
    }

    /// Whether the tail leans to the left: a tail in the left half of the box leans out to the
    /// left, one in the right half to the right, toward the speaker beside the bubble.
    pub fn tail_leans_left(&self) -> bool {
        match self.lean {
            Some(lean) => lean == TailLean::Left,
            None => self
                .drawn_place()
                .is_none_or(|place| place.tail_percent < 50),
        }
    }

    /// The free place as drawn: where it was given, the width (the most the bubble is drawn) at
    /// least the narrowest bubble and the tail away from the chamfered ends. The bubble is not
    /// moved; keeping it clear of the chrome is `view::bubble_layout`'s.
    pub fn drawn_place(&self) -> Option<BubblePlace> {
        self.place
            .zip(self.held_width_percent())
            .map(|(place, width)| BubblePlace {
                width_percent: width,
                tail_percent: place
                    .tail_percent
                    .clamp(TAIL_PERCENT_FROM_END, 100 - TAIL_PERCENT_FROM_END),
                ..place
            })
    }
}
