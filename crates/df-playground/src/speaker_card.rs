//! The card of a speaker who is not in the picture on screen.
//!
//! A line cued for a character the shared screen does not show (a close-up of a clue is up, or the
//! room's scan has no region of that name) is not drawn as a bubble pointing at nobody: it is a
//! card at the bottom right with the character's face, name and line. The face is a crop of the
//! room picture the backdrop's layout was scanned from, round the character's head. This module
//! only decides and measures; `kit::speaker` draws it.

use crate::fixture::TvScene;
use crate::image_layout::{ImageLayout, LayoutRegion, PictureBinding, anchored_place};
use crate::lobby::model::{Bubble, BubbleTone};
use crate::media::{MediaKind, media_kind};
use crate::timeline::line_id;

/// How much larger than the face box the portrait is cut, in percent: head and shoulders.
const PORTRAIT_PADDING_PERCENT: u32 = 170;
/// How far above the top of the head the portrait starts, in percent of its side.
const PORTRAIT_HEADROOM_PERCENT: u32 = 18;

/// A square cut of a picture, in thousandths of the picture's width and height. It is square in
/// pixels, so its two numbers differ in a picture that is not square.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortraitCrop {
    pub left_permille: u16,
    pub top_permille: u16,
    pub width_permille: u16,
    pub height_permille: u16,
}

impl PortraitCrop {
    /// How an image of the whole picture is sized and moved inside a square frame so that only
    /// this cut shows: its width and height as percents of the frame, and its left and top
    /// (negative) as percents of the frame.
    pub fn frame(self) -> (f32, f32, f32, f32) {
        let width = f32::from(self.width_permille);
        let height = f32::from(self.height_permille);
        (
            100_000.0 / width,
            100_000.0 / height,
            -f32::from(self.left_permille) * 100.0 / width,
            -f32::from(self.top_permille) * 100.0 / height,
        )
    }
}

/// The portrait of the character in `region` of a picture bound as `picture`: a square, in
/// pixels, centred on the face across and from just above the head down, held inside the picture.
pub fn portrait_crop(region: &LayoutRegion, picture: &PictureBinding) -> PortraitCrop {
    let (width_px, height_px) = (picture.width_px.max(1), picture.height_px.max(1));
    let face = df_scene_layout::face_box(region, width_px, height_px);
    let face_width_px = u32::from(face.right_permille - face.left_permille) * width_px / 1000;
    let face_height_px = u32::from(face.bottom_permille - face.top_permille) * height_px / 1000;
    let side_px = (face_width_px.max(face_height_px) * PORTRAIT_PADDING_PERCENT / 100)
        .clamp(1, width_px.min(height_px));
    let centre_x_px =
        (u32::from(face.left_permille) + u32::from(face.right_permille)) * width_px / 2000;
    let left_px = centre_x_px
        .saturating_sub(side_px / 2)
        .min(width_px - side_px);
    // Head and shoulders: room above the hair, the rest of the cut below the chin.
    let top_px = (u32::from(face.top_permille) * height_px / 1000)
        .saturating_sub(side_px * PORTRAIT_HEADROOM_PERCENT / 100)
        .min(height_px - side_px);
    let permille = |value: u32, whole: u32| u16::try_from(value * 1000 / whole).unwrap_or(1000);
    PortraitCrop {
        left_permille: permille(left_px, width_px),
        top_permille: permille(top_px, height_px),
        width_permille: permille(side_px, width_px).max(1),
        height_permille: permille(side_px, height_px).max(1),
    }
}

/// The picture a face is cut from, and the cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Portrait {
    pub media: String,
    pub crop: PortraitCrop,
}

/// What the card shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeakerCard {
    /// The character's name in the layout, which anchors the cue.
    pub character: String,
    /// The name shown: the layout's shown name, never the id.
    pub name: String,
    /// The line, or `None` for a thinking indicator.
    pub line: Option<String>,
    /// The face, when the layout still carries the character's region of an image.
    pub portrait: Option<Portrait>,
    /// A hidden copy of a visible speaker's line, shown only while a plate change is under way:
    /// the bubble over the speaker gives way to this lower third so the plates stay clean.
    pub twin: bool,
}

impl SpeakerCard {
    /// A name for this card's showing, plain enough for the timeline: the character, and which
    /// line (or thinking). The page fades a card out and another in when this changes.
    pub fn card_identity(&self) -> String {
        match &self.line {
            Some(line) => format!("card-{}-{}", self.character, line_id(line)),
            None => format!("card-{}-thinking", self.character),
        }
    }
}

/// The name `bubble` shows: the shown name its character has in `cast`, else the written speaker.
pub fn bubble_name<'a>(bubble: &'a Bubble, cast: Option<&'a ImageLayout>) -> &'a str {
    bubble
        .anchor
        .as_ref()
        .and_then(|anchor| cast?.region(&anchor.region)?.display_name())
        .unwrap_or(&bubble.speaker)
}

/// The card to show for `bubbles`, if a character who speaks is not in the picture on screen:
/// `visible` is the layout scanned from that picture, `cast` the layout whose names and faces are
/// known (the room's, kept on a close-up). One card at a time: the latest speaker, a thinking one
/// only when nobody speaks.
pub fn speaker_card(
    bubbles: &[Bubble],
    visible: Option<&ImageLayout>,
    cast: Option<&ImageLayout>,
) -> Option<SpeakerCard> {
    let off_screen =
        |bubble: &&Bubble| bubble.anchor.is_some() && anchored_place(bubble, visible).is_none();
    let chosen = bubbles
        .iter()
        .rev()
        .filter(off_screen)
        .find(|bubble| bubble.tone != BubbleTone::Thought)
        .or_else(|| bubbles.iter().rev().find(off_screen))?;
    card_of(chosen, cast, false)
}

/// The lower third that stands in for the bubble of the latest visible speaker during a plate
/// change: a hidden twin of that line, none when nobody visible speaks.
pub fn lower_third_twin(
    bubbles: &[Bubble],
    visible: Option<&ImageLayout>,
    cast: Option<&ImageLayout>,
) -> Option<SpeakerCard> {
    let chosen = bubbles.iter().rev().find(|bubble| {
        bubble.tone != BubbleTone::Thought
            && bubble.anchor.is_some()
            && anchored_place(bubble, visible).is_some()
    })?;
    card_of(chosen, cast, true)
}

fn card_of(chosen: &Bubble, cast: Option<&ImageLayout>, twin: bool) -> Option<SpeakerCard> {
    let anchor = chosen.anchor.as_ref()?;
    let portrait = cast.and_then(|layout| {
        let region = layout.region(&anchor.region)?;
        let picture = layout.picture();
        (media_kind(&picture.name) == Some(MediaKind::Image)).then(|| Portrait {
            media: picture.name.clone(),
            crop: portrait_crop(region, picture),
        })
    });
    Some(SpeakerCard {
        character: anchor.region.clone(),
        name: bubble_name(chosen, cast).to_owned(),
        line: (chosen.tone != BubbleTone::Thought).then(|| chosen.line.clone()),
        portrait,
        twin,
    })
}

/// The card `scene` shows, if its roleplay card has an off-screen speaker.
pub fn scene_card(scene: &TvScene) -> Option<SpeakerCard> {
    let roleplay = scene.roleplay.as_ref()?;
    speaker_card(
        &roleplay.bubbles,
        scene.backdrop_layout(),
        scene.cast_layout(),
    )
}

/// The hidden lower-third twin `scene` carries for its visible speaker, when no off-screen
/// speaker has the card (one presentation of a line at a time).
pub fn scene_twin(scene: &TvScene) -> Option<SpeakerCard> {
    let roleplay = scene.roleplay.as_ref()?;
    if scene_card(scene).is_some() {
        return None;
    }
    lower_third_twin(
        &roleplay.bubbles,
        scene.backdrop_layout(),
        scene.cast_layout(),
    )
}
