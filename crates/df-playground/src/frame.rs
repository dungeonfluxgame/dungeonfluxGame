//! The frame: everything a page draws, taken at one moment, as the host writes it and the page
//! reads it (one type on both sides), and how the page asks for the next one.
//!
//! The host holds a frame request until something changes; when nothing does within its wait it
//! answers with no body (or 204), which is "no change", not a mistake. The request names the last
//! frame and the host run it came from, so a restarted host answers at once. A frame is the page's
//! only view stream: it carries the canvas, the controls and the revision of the playground's
//! files, so the page asks for nothing else to follow the host. None of this touches the page, so
//! native tests cover it.

use serde::{Deserialize, Serialize};

use crate::controls::{Controls, ControlsReply};
use crate::fixture::ElementView;
use crate::path::{
    FRAME_AFTER_PARAMETER, FRAME_BOOT_PARAMETER, FRAME_IDLE_NO_CONTENT, FRAME_IDLE_PARAMETER,
};
use crate::scene::SceneOverlay;

/// The canvas as a frame carries it: its revision and its elements, without the kinds (the page
/// never reads them, and they would be sent again with every frame).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameCanvas {
    pub revision: u64,
    pub elements: Vec<ElementView>,
}

/// Everything the playground page draws, taken at one moment. `frame` counts the host's changes,
/// so a page that has frame N asks for the next one with `after=N` and is answered when there is
/// one. `asset_revision` changes when any playground file does, as `<assets>.<package>`: the page
/// reads its theme and fixtures again when the first part changes and reloads itself when the
/// second does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub frame: u64,
    pub canvas: FrameCanvas,
    pub controls: ControlsReply,
    pub asset_revision: String,
}

impl Frame {
    /// The frame as a phone on the player listener is sent it (`client-architecture.md`: the
    /// server sends each client only what it draws). The stage's elements are left out (the
    /// canvas keeps its revision), and so is every developer switch and every piece of the shared
    /// screen a phone does not mirror (`player_controls`). A seat's private things (its story, its
    /// ask, its pages) never ride the frame: they come from the seat routes, by the seat's key.
    pub fn for_player(self) -> Self {
        Self {
            canvas: FrameCanvas {
                revision: self.canvas.revision,
                elements: Vec::new(),
            },
            controls: player_controls(self.controls),
            ..self
        }
    }
}

/// The controls as a phone needs them: its reading pace, its sound, its switches for speech, the
/// scene's music and sound cues with their counters, which every phone at the table hears, and the
/// pieces of the shared screen a phone mirrors, which the whole room sees on the TV anyway: the
/// party strip and its seats, the handout, the clue board and the clue shown full screen, the
/// chapter title and the roll card (with its counter). Every other slot of the shared screen and
/// the developer's one phone (its suggested choices, its groups of cards, its unavailable choices
/// and its picture, which no seat owns) is left to the fixture (`Keep`), as is any slot added
/// later until it is named here; each seat's own cards, story and pages come from its seat routes,
/// by its key. The developer switches (the grid, the scanned layout, the camera's zoom bound and
/// the plate timer) are at their defaults, so the frame is the same for every phone and says
/// nothing of any seat.
pub fn player_controls(reply: ControlsReply) -> ControlsReply {
    let defaults = Controls::default();
    let scene = reply.scene;
    ControlsReply {
        controls: Controls {
            show_grid: defaults.show_grid,
            show_sam: defaults.show_sam,
            zoom_limit: defaults.zoom_limit,
            plate: defaults.plate,
            ..reply.controls
        },
        scene: SceneOverlay {
            party: scene.party,
            seats: scene.seats,
            card: scene.card,
            clues: scene.clues,
            shown_clue: scene.shown_clue,
            title_card: scene.title_card,
            roll: scene.roll,
            music: scene.music,
            cues: scene.cues,
            cue_count: scene.cue_count,
            slide_count: scene.slide_count,
            camera_count: scene.camera_count,
            roll_count: scene.roll_count,
            ..SceneOverlay::default()
        },
        ..reply
    }
}

/// What the host answered to a frame request.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameReply {
    /// Nothing changed while the host waited; ask again.
    NoChange,
    Frame(Box<Frame>),
}

/// Reads a frame answer. An empty body is "no change"; anything else must be a frame.
pub fn parse_frame_reply(text: &str) -> Result<FrameReply, serde_json::Error> {
    if text.trim().is_empty() {
        return Ok(FrameReply::NoChange);
    }
    serde_json::from_str(text).map(|frame| FrameReply::Frame(Box::new(frame)))
}

/// Whether a boot id is plain enough to put in an address as it is.
fn plain_boot_id(boot_id: &str) -> bool {
    !boot_id.is_empty()
        && boot_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

/// The address of the next frame request: the first asks for whatever is there, later ones name
/// the last frame seen (`after`) and the run of the host it came from (`boot`), so a host that
/// restarted answers at once even if its count has reached the page's, and say that a wait that
/// runs out may be answered with nothing (`idle=204`), which the page reads as no change.
pub fn frame_address(base: &str, after: Option<u64>, boot_id: Option<&str>) -> String {
    let Some(after) = after else {
        return base.to_owned();
    };
    match boot_id.filter(|boot| plain_boot_id(boot)) {
        Some(boot) => format!(
            "{base}?{FRAME_AFTER_PARAMETER}={after}&{FRAME_BOOT_PARAMETER}={boot}&{FRAME_IDLE_PARAMETER}={FRAME_IDLE_NO_CONTENT}"
        ),
        None => format!(
            "{base}?{FRAME_AFTER_PARAMETER}={after}&{FRAME_IDLE_PARAMETER}={FRAME_IDLE_NO_CONTENT}"
        ),
    }
}
