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
use crate::locate::request::{
    CharacterRequest, IdentifiedLayout, IdentifyRequest, IdentityRow, LabelRequest, LocateReply,
    StoredLayout,
};
use crate::locate::routes::path as locate_path;
use df_scene_layout::SceneLayout;

/// How sure the identity judge must be, in thousandths, to name a region for someone a walk's
/// plate was made with whom its scan left unnamed. The plate was painted with them in it, so the
/// judge's pick of a region nobody holds and nobody else is picked for names them at a lower bar
/// than a picture of strangers needs (host 40: an apple seller on an arrival plate was picked at
/// 500, under the scan's 600, never became a character and could not be talked to).
const PLACED_MIN_CONFIDENCE_PERMILLE: u16 = 350;

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
/// view. `base` is the picture it continues (the step before, on an approach), when it has one.
pub(crate) async fn make_walk_plate(
    port: u16,
    name: &str,
    painting: &Painting<'_>,
    characters: Vec<CharacterRequest>,
    base: Option<&str>,
) -> Result<MadePlate, PlateFailure> {
    let mut scanned = scan_plate_with(
        port,
        name,
        painting,
        characters.clone(),
        base,
        (Priority::Normal, Some(SheetPick::FullLength)),
    )
    .await?;
    // Someone the plate was painted with whom the scan left unnamed is named where the judge
    // picked them, when that region is free: then they are a character of the scene before it is
    // shown, and a persona can be made for them. A failure here leaves the plate as scanned.
    if let Some(made) = scanned.made.as_mut()
        && made.named < made.asked
        && let Ok(named) = name_the_placed(port, &scanned.picture, &characters).await
    {
        made.named += named;
    }
    Ok(MadePlate {
        making: scanned.made.as_ref().map(|made| made.since.elapsed()),
        scanned,
    })
}

/// Names on `picture` each of `characters` its scan left unnamed whose region the identity judge
/// picked with at least [`PLACED_MIN_CONFIDENCE_PERMILLE`], when no one holds that region and no
/// one else was picked for it; answers how many were named. The judge is asked the scan's own
/// question (`min=1000`: it names nothing below certainty itself), so the answer is its earlier
/// one.
async fn name_the_placed(
    port: u16,
    picture: &str,
    characters: &[CharacterRequest],
) -> Result<usize, DevError> {
    let identified: LocateReply<IdentifiedLayout> = post_json(
        port,
        locate_path::LOCATE_IDENTIFY,
        &IdentifyRequest {
            picture: picture.to_owned(),
            characters: characters.to_vec(),
            min_confidence_permille: Some(1000),
        },
    )
    .await?;
    let identified = reply_value(identified)?;
    let to_name = placed_to_name(&identified.rows, &identified.layout);
    for (region, name) in &to_name {
        let display_name = characters
            .iter()
            .find(|character| &character.name == name)
            .and_then(|character| character.display_name.clone());
        let labelled: LocateReply<StoredLayout> = post_json(
            port,
            locate_path::LOCATE_LABEL,
            &LabelRequest {
                picture: picture.to_owned(),
                region: region.to_string(),
                label: Some(name.clone()),
                display_name,
            },
        )
        .await?;
        reply_value(labelled)?;
    }
    Ok(to_name.len())
}

/// The regions (numbered from 0) to name, and whom: each character `layout` does not name yet
/// whose picked region nobody holds, nobody else was picked for, and was picked with at least
/// [`PLACED_MIN_CONFIDENCE_PERMILLE`].
pub(super) fn placed_to_name(rows: &[IdentityRow], layout: &SceneLayout) -> Vec<(usize, String)> {
    rows.iter()
        .filter_map(|row| {
            let region = row.region?;
            let free = layout
                .regions()
                .get(region)
                .is_some_and(|shown| shown.label().is_none());
            let picked_once = rows
                .iter()
                .filter(|other| other.region == Some(region))
                .count()
                == 1;
            (layout.region(&row.name).is_none()
                && row.confidence_permille >= PLACED_MIN_CONFIDENCE_PERMILLE
                && free
                && picked_once)
                .then(|| (region, row.name.clone()))
        })
        .collect()
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

#[cfg(test)]
#[path = "walk_plates_tests.rs"]
mod tests;
