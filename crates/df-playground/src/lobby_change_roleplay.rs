//! The roleplay card's live changes (`lobby_change` holds the rest): the title bar, one speech bubble
//! (by number) and the caption, with how a change is applied to a bubble and what its audit line
//! carries. `ScreenOverlay::apply` calls this; every percent is checked here, at the edge.

use serde::{Deserialize, Serialize};

use super::{DEFAULT_BUBBLE, percent, plain, present, put, put_text};
use crate::checks;
use crate::limits::MAXIMUM_CAPTION_CHARACTERS;
use crate::lobby_model::{Bubble, BubbleTone};
use crate::overlay::Glyph;
use crate::scene::SceneError;

/// A change to the roleplay card: its title bar, one bubble (number 1 when none is named) and the
/// caption.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RoleplayEdit {
    pub title: Option<String>,
    pub icon: Option<Glyph>,
    pub bubble: Option<usize>,
    pub speaker: Option<String>,
    pub line: Option<String>,
    pub emphasis: Option<String>,
    pub speaker_icon: Option<Glyph>,
    pub portrait: Option<String>,
    pub tone: Option<BubbleTone>,
    pub left_percent: Option<u8>,
    pub top_percent: Option<u8>,
    pub width_percent: Option<u8>,
    pub tail_x_percent: Option<u8>,
    pub tail_y_percent: Option<u8>,
    pub caption: Option<String>,
}

pub(super) fn new_bubble() -> Bubble {
    let (left, top, width, tail_x, tail_y) = DEFAULT_BUBBLE;
    Bubble {
        speaker: String::new(),
        line: String::new(),
        emphasis: None,
        glyph: None,
        portrait: None,
        tone: BubbleTone::default(),
        left_percent: left,
        top_percent: top,
        width_percent: width,
        tail_x_percent: tail_x,
        tail_y_percent: tail_y,
    }
}

impl RoleplayEdit {
    pub(super) fn has_bubble_fields(&self) -> bool {
        self.emphasis.is_some()
            || self.speaker_icon.is_some()
            || self.portrait.is_some()
            || self.tone.is_some()
            || self.left_percent.is_some()
            || self.top_percent.is_some()
            || self.width_percent.is_some()
            || self.tail_x_percent.is_some()
            || self.tail_y_percent.is_some()
    }

    pub(super) fn apply_to_bubble(&self, bubble: &mut Bubble) -> Result<(), SceneError> {
        put_text(&mut bubble.speaker, self.speaker.clone(), "a speaker")?;
        if let Some(line) = self.line.clone() {
            checks::text("a bubble's line", &line, MAXIMUM_CAPTION_CHARACTERS)?;
            bubble.line = line;
        }
        put(
            &mut bubble.emphasis,
            self.emphasis.clone(),
            "a bubble's emphasis",
        )?;
        put(&mut bubble.portrait, self.portrait.clone(), "a media name")?;
        if let Some(icon) = self.speaker_icon {
            bubble.glyph = Some(icon);
        }
        if let Some(tone) = self.tone {
            bubble.tone = tone;
        }
        for (what, value, slot) in [
            (
                "a bubble's left edge",
                self.left_percent,
                &mut bubble.left_percent,
            ),
            ("a bubble's top", self.top_percent, &mut bubble.top_percent),
            (
                "a bubble's width",
                self.width_percent,
                &mut bubble.width_percent,
            ),
            (
                "a bubble's tail x",
                self.tail_x_percent,
                &mut bubble.tail_x_percent,
            ),
            (
                "a bubble's tail y",
                self.tail_y_percent,
                &mut bubble.tail_y_percent,
            ),
        ] {
            if let Some(value) = value {
                *slot = percent(what, value)?;
            }
        }
        Ok(())
    }
}

pub(super) fn roleplay_pairs(edit: &RoleplayEdit) -> Vec<(&'static str, String)> {
    let mut pairs = Vec::new();
    present(&mut pairs, "title", &edit.title);
    present(&mut pairs, "icon", &edit.icon);
    if let Some(bubble) = edit.bubble {
        pairs.push(("bubble", bubble.to_string()));
    }
    present(&mut pairs, "speaker", &edit.speaker);
    present(&mut pairs, "line", &edit.line);
    present(&mut pairs, "emphasis", &edit.emphasis);
    present(&mut pairs, "caption", &edit.caption);
    if let Some(media) = &edit.portrait {
        pairs.push(("portrait", plain(media)));
    }
    for (key, value) in [
        ("left", edit.left_percent),
        ("top", edit.top_percent),
        ("width", edit.width_percent),
        ("tail_x", edit.tail_x_percent),
        ("tail_y", edit.tail_y_percent),
    ] {
        if let Some(value) = value {
            pairs.push((key, value.to_string()));
        }
    }
    pairs
}
