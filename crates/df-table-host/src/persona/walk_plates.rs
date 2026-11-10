//! The plates of a move's walk (`movement::walk`): each sight on the way and the place itself is
//! made and scanned apart from being shown, so they can be made together and put behind the scene
//! one after the other, as soon as each is ready and the one before has been up long enough. The
//! making, the scan and the swap are the plate's (`plate`); this module only splits them and
//! reports each moment of a walk's plate to the host as a `PERSONA frame_timing .. reason=walk`
//! line.

use std::time::Duration;

use df_playground::lobby::model::SceneTitle;
use df_types::saturating_millis;

use super::frame::{
    DroppedWhy, FrameMoment, FrameReason, FrameTimingReply, FrameTimingRequest,
    PERSONA_FRAME_TIMING,
};
use super::plate::{
    Painting, PlateFailure, RoomTurn, ScannedPlate, SharedScreen, drop_stale_bubbles,
    put_title_back, scan_plate_with, show_scanned, start_spinner,
};
use crate::dev::DevError;
use crate::dev_client::{post_json, reply_value};
use crate::imagegen::{Priority, SheetPick};
use crate::locate::request::CharacterRequest;

/// A plate of a walk, made and scanned and not yet behind the scene.
pub(crate) struct MadePlate {
    scanned: ScannedPlate,
    making: Option<Duration>,
}

impl MadePlate {
    /// Its media library name.
    pub(crate) fn picture(&self) -> &str {
        &self.scanned.picture
    }

    /// How long it took to make, scan and name; none for a plate kept from before.
    pub(crate) fn making(&self) -> Option<Duration> {
        self.making
    }

    /// What the scan found, for a transcript line: regions found and characters named of those
    /// asked for; a kept plate says so.
    pub(crate) fn found(&self) -> String {
        match &self.scanned.made {
            Some(made) => format!(
                "{} regions found, {} of {} characters named; {}",
                made.regions, made.named, made.asked, made.sheets_note
            ),
            None => "kept from before, not rendered again".to_owned(),
        }
    }
}

/// Makes the plate `name` from `painting` with `characters` located and named in it, and shows it
/// nowhere. Every plate of a walk is made at normal priority, so the sights never starve the place,
/// and shows its people by the full-length view of their sheets: a small walking figure in a wide
/// view.
pub(crate) async fn make_walk_plate(
    port: u16,
    name: &str,
    painting: &Painting<'_>,
    characters: Vec<CharacterRequest>,
) -> Result<MadePlate, PlateFailure> {
    scan_plate_with(
        port,
        name,
        painting,
        characters,
        None,
        (Priority::Normal, Some(SheetPick::FullLength)),
    )
    .await
    .map(|scanned| MadePlate {
        making: scanned.made.as_ref().map(|made| made.since.elapsed()),
        scanned,
    })
}

/// Puts `plate` behind the scene at once, with the cast of `cast_from` (the picture before it) when
/// its own scan names fewer of the characters than were asked for, and takes away the bubbles of
/// characters who are not in it. Answers the line for the transcript.
pub(crate) async fn show_walk_plate(
    port: u16,
    plate: MadePlate,
    name: &str,
    cast_from: Option<&str>,
    present: &[String],
) -> Result<String, PlateFailure> {
    let turn = RoomTurn {
        number: 1,
        screen: SharedScreen::default(),
        cast_from: cast_from.map(str::to_owned),
        cued: false,
        emergency: None,
    };
    let shown = show_scanned(port, name, plate.scanned, &turn).await?;
    drop_stale_bubbles(port, present)
        .await
        .map_err(|error| (super::plate::PlateStep::Backdrop, Box::new(error)))?;
    Ok(shown.summary)
}

/// The scene's title as it was before the spinner of the walk.
pub(crate) struct Spinner(Option<SceneTitle>);

/// Shows the spinner title on the scene while the walk's plates are made.
pub(crate) async fn spinner_on(port: u16) -> Result<Spinner, DevError> {
    start_spinner(port).await.map(Spinner)
}

/// Puts the title that the spinner replaced back.
pub(crate) async fn spinner_off(port: u16, spinner: Spinner) -> Result<(), DevError> {
    put_title_back(port, spinner.0).await
}

/// How a walk's plate left the screen's queue, as the host's `frame_timing` line says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WalkMoment {
    /// Its making began.
    Started,
    /// It went behind the scene, after `took` of making.
    Shown(Duration),
    /// It was up for `took`.
    Ended(Duration),
    /// Making it failed.
    Failed,
    /// It was not shown: the place was ready first.
    Skipped,
}

/// Records a moment of the walk's plate `frame` on the host (`POST /dev/persona/frame/timing`) and
/// answers when the host recorded it, on the playground's clock.
pub(crate) async fn mark_walk(port: u16, frame: &str, moment: WalkMoment) -> Result<u64, DevError> {
    let (moment, took, dropped) = match moment {
        WalkMoment::Started => (FrameMoment::Started, Duration::ZERO, None),
        WalkMoment::Shown(took) => (FrameMoment::Shown, took, None),
        WalkMoment::Ended(took) => (FrameMoment::Ended, took, None),
        WalkMoment::Failed => (FrameMoment::Failed, Duration::ZERO, None),
        WalkMoment::Skipped => (
            FrameMoment::Dropped,
            Duration::ZERO,
            Some(DroppedWhy::Overtaken),
        ),
    };
    let request = FrameTimingRequest {
        frame: frame.to_owned(),
        moment,
        reason: FrameReason::Walk,
        took_ms: saturating_millis(took),
        declared: None,
        revealed: None,
        dropped,
    };
    let reply: FrameTimingReply = post_json(port, PERSONA_FRAME_TIMING, &request).await?;
    reply_value(reply).map(|timed| timed.at_ms)
}
