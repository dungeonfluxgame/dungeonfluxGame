//! The roleplay card's live changes (`lobby::change` holds the rest): the title bar, one speech
//! bubble (by number) and the caption, with how a change is applied to a bubble and what its audit
//! line carries. `SceneOverlay::apply_screen` calls this; every percent is checked here, at the
//! edge.

use serde::{Deserialize, Serialize};

use super::{DEFAULT_BUBBLE, percent, plain, put, put_text};
use crate::audit_fields::present;
use crate::checks;
use crate::image_layout::{BubbleAnchor, CharacterCue, CharacterMarker, CharacterPoint, CueKind};
use crate::limits::{MAXIMUM_CAPTION_CHARACTERS, MAXIMUM_CHARACTER_MARKERS};
use crate::lobby::model::{Bubble, BubbleTone, RoleplayScene};
use crate::overlay::Glyph;
use crate::scene::SceneError;

/// A change to the roleplay card: its title bar, one bubble (number 1 when none is named) and the
/// caption.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RoleplayEdit {
    pub title: Option<String>,
    pub icon: Option<Glyph>,
    /// Takes the title bar away; `title` and `icon` given with it are ignored.
    pub title_remove: bool,
    /// Takes the bubble of this number (from 1) away, before any other bubble field applies.
    pub bubble_remove: Option<usize>,
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
    /// Anchors the bubble to the region of this label in the backdrop's layout; an empty label
    /// takes the anchor away.
    pub anchor: Option<String>,
    /// The character point an anchored bubble points at.
    pub anchor_point: Option<CharacterPoint>,
    /// How far an anchored bubble's box sits sideways from its point, in percent.
    pub anchor_shift: Option<i8>,
    /// The marker (numbered from 1) the `marker_*` fields change.
    pub marker: Option<usize>,
    pub marker_region: Option<String>,
    pub marker_text: Option<String>,
    pub marker_point: Option<CharacterPoint>,
    pub marker_glyph: Option<Glyph>,
    pub marker_dx: Option<i8>,
    pub marker_dy: Option<i8>,
    /// Takes the marker of this number away.
    pub marker_remove: Option<usize>,
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
        anchor: None,
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
            || self.anchor.is_some()
            || self.anchor_point.is_some()
            || self.anchor_shift.is_some()
    }

    pub(super) fn has_marker_fields(&self) -> bool {
        self.marker.is_some()
            || self.marker_region.is_some()
            || self.marker_text.is_some()
            || self.marker_point.is_some()
            || self.marker_glyph.is_some()
            || self.marker_dx.is_some()
            || self.marker_dy.is_some()
    }

    /// Applies the marker fields to `markers`: removes one, or changes (or adds) the numbered one.
    pub(super) fn apply_to_markers(
        &self,
        markers: &mut Vec<CharacterMarker>,
    ) -> Result<(), SceneError> {
        if let Some(number) = self.marker_remove {
            if !(1..=markers.len()).contains(&number) {
                return Err(SceneError::Screen(
                    super::ScreenError::InvalidMarkerNumber { number },
                ));
            }
            markers.remove(number - 1);
        }
        if !self.has_marker_fields() {
            return Ok(());
        }
        let number = self.marker.unwrap_or(markers.len() + 1);
        if !(1..=MAXIMUM_CHARACTER_MARKERS).contains(&number) || number > markers.len() + 1 {
            return Err(SceneError::Screen(
                super::ScreenError::InvalidMarkerNumber { number },
            ));
        }
        if number == markers.len() + 1 {
            let (Some(region), Some(text)) = (self.marker_region.clone(), self.marker_text.clone())
            else {
                return Err(SceneError::Screen(super::ScreenError::NewMarkerNeeds));
            };
            markers.push(CharacterMarker {
                region,
                point: CharacterPoint::default(),
                text,
                glyph: None,
                dx_percent: 0,
                dy_percent: 0,
            });
        }
        let marker = &mut markers[number - 1];
        put_text(
            &mut marker.region,
            self.marker_region.clone(),
            "a marker's character",
        )?;
        if let Some(text) = self.marker_text.clone() {
            checks::text("a marker's text", &text, MAXIMUM_CAPTION_CHARACTERS)?;
            marker.text = text;
        }
        if let Some(point) = self.marker_point {
            marker.point = point;
        }
        if let Some(glyph) = self.marker_glyph {
            marker.glyph = Some(glyph);
        }
        if let Some(dx) = self.marker_dx {
            marker.dx_percent = dx;
        }
        if let Some(dy) = self.marker_dy {
            marker.dy_percent = dy;
        }
        Ok(())
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
        match self.anchor.as_deref() {
            Some("") => bubble.anchor = None,
            Some(label) => {
                put_text(
                    &mut String::new(),
                    Some(label.to_owned()),
                    "a bubble's anchor",
                )?;
                let kept = bubble.anchor.take();
                bubble.anchor = Some(BubbleAnchor {
                    region: label.to_owned(),
                    shift_percent: kept.as_ref().map_or(0, |anchor| anchor.shift_percent),
                    point: kept.map_or(CharacterPoint::default(), |anchor| anchor.point),
                });
            }
            None => {}
        }
        if self.anchor_point.is_some() || self.anchor_shift.is_some() {
            let Some(anchor) = bubble.anchor.as_mut() else {
                return Err(SceneError::Screen(super::ScreenError::NoBubbleAnchor));
            };
            if let Some(point) = self.anchor_point {
                anchor.point = point;
            }
            if let Some(shift) = self.anchor_shift {
                anchor.shift_percent = shift;
            }
        }
        Ok(())
    }
}

pub(super) fn roleplay_pairs(edit: &RoleplayEdit) -> Vec<(&'static str, String)> {
    let mut pairs = Vec::new();
    present(&mut pairs, "title", edit.title.as_ref());
    present(&mut pairs, "icon", edit.icon.as_ref());
    if let Some(bubble) = edit.bubble {
        pairs.push(("bubble", bubble.to_string()));
    }
    if let Some(bubble) = edit.bubble_remove {
        pairs.push(("bubble_remove", bubble.to_string()));
    }
    present(&mut pairs, "speaker", edit.speaker.as_ref());
    present(&mut pairs, "line", edit.line.as_ref());
    present(&mut pairs, "emphasis", edit.emphasis.as_ref());
    present(&mut pairs, "caption", edit.caption.as_ref());
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
    if let Some(anchor) = &edit.anchor {
        pairs.push(("anchor", plain(anchor)));
    }
    if let Some(point) = edit.anchor_point {
        pairs.push(("anchor_point", point.key().to_owned()));
    }
    for (key, value) in [
        ("anchor_shift", edit.anchor_shift),
        ("marker_dx", edit.marker_dx),
        ("marker_dy", edit.marker_dy),
    ] {
        if let Some(value) = value {
            pairs.push((key, value.to_string()));
        }
    }
    for (key, value) in [
        ("marker", edit.marker),
        ("marker_remove", edit.marker_remove),
    ] {
        if let Some(value) = value {
            pairs.push((key, value.to_string()));
        }
    }
    if let Some(region) = &edit.marker_region {
        pairs.push(("marker_region", plain(region)));
    }
    present(&mut pairs, "marker_text", edit.marker_text.as_ref());
    if let Some(point) = edit.marker_point {
        pairs.push(("marker_point", point.key().to_owned()));
    }
    pairs
}

/// How wide a character's own bubble is, in percent of the screen: a wordless thinking indicator
/// and a spoken line.
const CUE_THOUGHT_WIDTH_PERCENT: u8 = 7;
const CUE_SPEECH_WIDTH_PERCENT: u8 = 22;

/// The bubble a cue draws for `character`, anchored over its head.
fn cue_bubble(character: &str, tone: BubbleTone, line: String, width: u8) -> Bubble {
    let mut bubble = new_bubble();
    bubble.speaker = character.to_owned();
    bubble.line = line;
    bubble.tone = tone;
    bubble.width_percent = width;
    bubble.anchor = Some(BubbleAnchor {
        region: character.to_owned(),
        shift_percent: 0,
        point: CharacterPoint::Head,
    });
    bubble
}

/// Applies a chat system's cue to the roleplay card. Each character has at most one bubble of its
/// own, the one anchored to its name: thinking replaces it with a wordless indicator, speaking with
/// a bubble of the line, quiet takes it away. A shift already set on the character's bubble is
/// kept, so a scene's hand placement survives a change of cue. Bubbles of other characters, and
/// bubbles placed by hand, are never touched.
pub(super) fn apply_cue(card: &mut RoleplayScene, cue: &CharacterCue) -> Result<(), SceneError> {
    if !df_scene_layout::is_character_name(&cue.character) {
        return Err(SceneError::Screen(super::ScreenError::InvalidCue));
    }
    let own = |bubble: &Bubble| {
        bubble
            .anchor
            .as_ref()
            .is_some_and(|anchor| anchor.region == cue.character)
    };
    let kept_shift = card
        .bubbles
        .iter()
        .find(|bubble| own(bubble))
        .and_then(|bubble| bubble.anchor.as_ref())
        .map(|anchor| anchor.shift_percent);
    let replacement = match &cue.kind {
        CueKind::Quiet => None,
        CueKind::Thinking => Some(cue_bubble(
            &cue.character,
            BubbleTone::Thought,
            String::new(),
            CUE_THOUGHT_WIDTH_PERCENT,
        )),
        CueKind::Speaking { line } => {
            checks::text("a spoken line", line, MAXIMUM_CAPTION_CHARACTERS)
                .map_err(|_| SceneError::Screen(super::ScreenError::InvalidCue))?;
            Some(cue_bubble(
                &cue.character,
                BubbleTone::Npc,
                line.clone(),
                CUE_SPEECH_WIDTH_PERCENT,
            ))
        }
    };
    let at = card.bubbles.iter().position(&own);
    card.bubbles.retain(|bubble| !own(bubble));
    if let Some(mut bubble) = replacement {
        if let (Some(shift), Some(anchor)) = (kept_shift, bubble.anchor.as_mut()) {
            anchor.shift_percent = shift;
        }
        let at = at.unwrap_or(card.bubbles.len()).min(card.bubbles.len());
        card.bubbles.insert(at, bubble);
    }
    Ok(())
}

/// The audit pairs of a cue: the character and the kind, never the line.
pub(super) fn cue_pairs(cue: &CharacterCue) -> Vec<(&'static str, String)> {
    let kind = match cue.kind {
        CueKind::Thinking => "thinking",
        CueKind::Speaking { .. } => "speaking",
        CueKind::Quiet => "quiet",
    };
    vec![
        ("character", plain(&cue.character)),
        ("cue", kind.to_owned()),
    ]
}
