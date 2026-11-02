//! What the host sends the page for a frame, and how the page asks for the next one.
//!
//! The host holds a frame request until something changes; when nothing does within its wait it
//! answers with no body (or 204), which is "no change", not a mistake. The request names the last
//! frame and the host run it came from, so a restarted host answers at once. None of this touches
//! the page, so native tests cover it.

use serde::Deserialize;

use crate::controls::ControlsReply;
use crate::fixture::StageElement;

/// The canvas the host serves, as far as the stage needs it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LiveCanvas {
    pub revision: u64,
    pub elements: Vec<StageElement>,
}

/// One frame from the host: the canvas and what the playground is showing, at one moment.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LiveFrame {
    pub frame: u64,
    pub canvas: LiveCanvas,
    pub controls: ControlsReply,
}

/// What the host answered to a frame request.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameReply {
    /// Nothing changed while the host waited; ask again.
    NoChange,
    Frame(Box<LiveFrame>),
}

/// Reads a frame answer. An empty body is "no change"; anything else must be a frame.
pub fn parse_frame_reply(text: &str) -> Result<FrameReply, serde_json::Error> {
    if text.trim().is_empty() {
        return Ok(FrameReply::NoChange);
    }
    serde_json::from_str(text).map(|frame| FrameReply::Frame(Box::new(frame)))
}

/// The status the host may answer an idle wait with instead of a whole frame.
const IDLE_NO_CONTENT: u16 = 204;

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
        Some(boot) => format!("{base}?after={after}&boot={boot}&idle={IDLE_NO_CONTENT}"),
        None => format!("{base}?after={after}&idle={IDLE_NO_CONTENT}"),
    }
}
