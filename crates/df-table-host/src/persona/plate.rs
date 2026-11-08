//! The plate of `persona converse plate=NAME`: a new tv backdrop made from what the characters
//! just said, put behind them while a spinner shows on the scene.
//!
//! A dev tool over routes that already exist, like the rest of `converse`: the image service
//! makes the picture (`media generate`), the imagery API finds the people in it (`media locate`),
//! the text service says which region is which character (`media locate identify`), and a scene
//! change puts the picture behind the figures. The prompt is built here from the table and the
//! latest lines, with no model call, so it is the same for the same talk. The title chip of the
//! scene is the spinner while the picture is made, and is put back whether the plate worked or not.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use df_playground::lobby::change::{RoleplayEdit, ScreenEdit};
use df_playground::lobby::model::SceneTitle;
use df_playground::overlay::Glyph;
use df_playground::scene::{ImageSlot, SceneChange};
use df_scene_layout::SceneLayout;

use super::converse::{Table, Turn};
use crate::dev::DevError;
use crate::dev_client::{get_json, post_json, reply_value};
use crate::imagegen::request::{GenerateReply, GenerateRequest, JobState};
use crate::imagegen::{Purpose, routes::path as image_path};
use crate::locate::cli::show as show_layout;
use crate::locate::request::{
    CharacterRequest, IdentifiedLayout, IdentifyRequest, LocateReply, LocateRequest, LocatedLayout,
};
use crate::locate::routes::path as locate_path;
use crate::movement::brief::BriefStyle;
use crate::playground::ChangeReply;
use crate::wire::{ControlsReply, route};

// TODO(DEV-12): tests for the prompt, the title put back after a failed step and the `plate=`
// option, and a live plate run once a dedicated signed-in Codex home exists (the image service
// refuses the user's own home: it has a config.toml).

/// The title of the scene while the picture is made.
const UPDATING_TITLE: &str = "Updating the scene...";
/// How many of the latest lines the picture is told.
const PICTURE_LINES: usize = 4;
/// The most characters of one line the picture is told.
const PICTURE_LINE_CHARS: usize = 300;

/// The step of the plate that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlateStep {
    Generate,
    Locate,
    Identify,
    Backdrop,
}

impl PlateStep {
    pub fn key(self) -> &'static str {
        match self {
            Self::Generate => "generate",
            Self::Locate => "locate",
            Self::Identify => "identify",
            Self::Backdrop => "backdrop",
        }
    }
}

/// The text the image service is asked to paint: the room, who sits in it, and what they have
/// just been talking about. It forbids every kind of writing, because the page draws the words.
pub(super) fn picture_prompt(table: &Table, history: &[Turn]) -> String {
    let people: Vec<String> = table
        .speakers()
        .iter()
        .map(|speaker| format!("{} ({})", speaker.display_name, speaker.job))
        .collect();
    let style = BriefStyle::default();
    let mut text = format!(
        concat!(
            "{} a lamplit tavern interior at night, warm candlelight and deep shadow, everyone ",
            "seated together at a long wooden table and facing the viewer. Seated, left to ",
            "right: {}. "
        ),
        style.direction,
        people.join("; ")
    );
    let skip = history.len().saturating_sub(PICTURE_LINES);
    if skip < history.len() {
        text.push_str("They have just been talking, and the scene shows what it was about:");
        for turn in &history[skip..] {
            let line: String = turn.line.chars().take(PICTURE_LINE_CHARS).collect();
            text.push_str(&format!(" \"{line}\""));
        }
        text.push(' ');
    }
    text.push_str("Show what they talk about as objects and mood in the room. ");
    text.push_str(&style.writing_ban);
    text
}

/// The step that failed and why.
pub(crate) type PlateFailure = (PlateStep, Box<DevError>);

fn at(step: PlateStep) -> impl FnOnce(DevError) -> PlateFailure {
    move |error| (step, Box::new(error))
}

/// A scene change sent to the host on `port`.
async fn change(port: u16, change: &SceneChange) -> Result<(), DevError> {
    let reply: ChangeReply = post_json(port, route::DEV_PLAYGROUND_SCENE, change).await?;
    reply_value(reply).map(|_| ())
}

fn title_change(title: &str, glyph: Glyph) -> SceneChange {
    SceneChange::Screen {
        edit: ScreenEdit::RoleplaySet {
            roleplay: RoleplayEdit {
                title: Some(title.to_owned()),
                icon: Some(glyph),
                ..RoleplayEdit::default()
            },
        },
    }
}

/// Takes the title bar away, for a scene that had none before the spinner.
fn title_removed() -> SceneChange {
    SceneChange::Screen {
        edit: ScreenEdit::RoleplaySet {
            roleplay: RoleplayEdit {
                title_remove: true,
                ..RoleplayEdit::default()
            },
        },
    }
}

/// The title bar the scene shows now through a live change, if one is.
async fn current_title(port: u16) -> Result<Option<SceneTitle>, DevError> {
    let controls: ControlsReply = get_json(port, route::PLAYGROUND_CONTROLS).await?;
    Ok(controls
        .scene
        .roleplay
        .set_value()
        .and_then(|card| card.title.clone()))
}

/// Makes the plate `name` from the talk and puts it behind the scene: the prompt is the talk's
/// (see `picture_prompt`) and the characters are the table's.
pub(super) async fn make_plate(
    port: u16,
    name: &str,
    table: &Table,
    history: &[Turn],
) -> Result<String, PlateFailure> {
    let characters = table
        .speakers()
        .iter()
        .map(|speaker| CharacterRequest {
            name: speaker.id.clone(),
            display_name: Some(speaker.display_name.clone()),
            description: Some(speaker.job.clone()),
            portrait: None,
        })
        .collect();
    deliver_plate(
        port,
        name,
        &picture_prompt(table, history),
        characters,
        None,
    )
    .await
}

/// Makes the plate `name` from `prompt` (an edit of `base`, a picture of the media library, when
/// given) and puts it behind the scene, with `characters` located and named in it. The spinner
/// title is shown from the start and the title that was there is put back at the end, also when a
/// step failed; the failure is that of the step, and a title that could not be put back is not
/// hidden by it.
pub(crate) async fn deliver_plate(
    port: u16,
    name: &str,
    prompt: &str,
    characters: Vec<CharacterRequest>,
    base: Option<&str>,
) -> Result<String, PlateFailure> {
    let before = start_spinner(port).await.map_err(at(PlateStep::Generate))?;
    let alone = RoomTurn {
        previous: None,
        screen: SharedScreen::default(),
        cast_from: None,
    };
    let made = make_frame(port, name, prompt, characters, base, alone)
        .await
        .map(|shown| shown.summary);
    let restored = put_title_back(port, before).await;
    match (made, restored) {
        (Ok(summary), Ok(())) => Ok(summary),
        (Err(failure), _) => Err(failure),
        (Ok(_), Err(error)) => Err(at(PlateStep::Backdrop)(error)),
    }
}

/// Shows the spinner title on the scene and answers the title it replaced.
pub(super) async fn start_spinner(port: u16) -> Result<Option<SceneTitle>, DevError> {
    let before = current_title(port).await?;
    change(port, &title_change(UPDATING_TITLE, Glyph::Spinner)).await?;
    Ok(before)
}

/// Puts `before` back as the scene's title, or takes the title away when there was none.
pub(super) async fn put_title_back(port: u16, before: Option<SceneTitle>) -> Result<(), DevError> {
    let put_back = before.map_or_else(title_removed, |title| {
        title_change(&title.text, title.glyph)
    });
    change(port, &put_back).await
}

/// When a frame of the room may go on screen: once the room frame started before it has been
/// shown (`previous`; a sender that went away, its frame failed, lets it go on), and once the
/// screen allows it (`screen`); and the picture whose cast it keeps when its own scan finds fewer
/// of the characters than were asked for (`cast_from`).
pub(super) struct RoomTurn {
    pub previous: Option<tokio::sync::oneshot::Receiver<()>>,
    pub screen: SharedScreen,
    pub cast_from: Option<String>,
}

/// A frame put on screen: what was done, and whether it shows with its own scan (every character
/// asked for named in it) rather than with the cast of the picture it replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Shown {
    pub summary: String,
    pub own_cast: bool,
}

/// Makes the plate `name` and puts it behind the scene, as `deliver_plate` does but without the
/// spinner (its caller shows one), and only at its `turn`: a frame made alongside an earlier one
/// waits for that one to be shown, so an older picture never replaces a newer one, and waits
/// while an insert holds the screen. Bubbles of characters not in the new picture are taken away.
pub(super) async fn make_frame(
    port: u16,
    name: &str,
    prompt: &str,
    characters: Vec<CharacterRequest>,
    base: Option<&str>,
    turn: RoomTurn,
) -> Result<Shown, PlateFailure> {
    let present: Vec<String> = characters
        .iter()
        .map(|character| character.name.clone())
        .collect();
    let shown = build(port, name, prompt, characters, base, turn).await?;
    drop_stale_bubbles(port, &present)
        .await
        .map_err(at(PlateStep::Backdrop))?;
    Ok(shown)
}

/// Makes the insert `name` (a cut to an item filling the screen) and puts it behind the scene as
/// soon as it is made and the last picture has been up the screen's least time: it waits for no
/// room frame, and holds the screen for `hold`, so a room frame that finishes meanwhile waits.
/// Nobody's face is in it, so it is not scanned: it is shown with the layout of `room`, the
/// picture it cuts away from, so the scene keeps its cast (who may speak, and where their bubbles
/// sit) while the camera is on the item.
pub(super) async fn make_insert(
    port: u16,
    name: &str,
    prompt: &str,
    base: Option<&str>,
    room: &str,
    (screen, hold): (&SharedScreen, Duration),
) -> Result<String, PlateFailure> {
    let picture = generate(port, name, prompt, base).await?;
    let layout = layout_of(port, room).await?;
    claim_screen(screen, false, Some(hold)).await;
    put_backdrop(port, &picture, Some(layout)).await?;
    Ok(format!(
        "insert {name}: {picture} is behind the scene, with the cast of {room}"
    ))
}

/// The screen, as the frames of one conversation share it: when the last picture went up, until
/// when an insert holds it, and the least time a picture stays up before another replaces it
/// (one that flashed by would be a flicker nobody at the table could follow; `frames.json`
/// `show.minimum_shown_seconds`, set by the watch from each judgement).
#[derive(Debug, Default)]
pub(super) struct Screen {
    last_shown: Option<Instant>,
    held_until: Option<Instant>,
    pub minimum_shown: Duration,
}

pub(super) type SharedScreen = Arc<Mutex<Screen>>;

/// Waits until a new picture may go up and claims the screen for it: the screen's least time after
/// the last picture and, when `respect_hold`, not while an insert holds it. The claim marks the
/// picture as up now and holds the screen for `hold` (an insert) or releases it, under one lock,
/// so two frames never pass together.
pub(super) async fn claim_screen(
    screen: &SharedScreen,
    respect_hold: bool,
    hold: Option<Duration>,
) {
    loop {
        let wait = {
            let mut state = screen.lock().unwrap_or_else(PoisonError::into_inner);
            let now = Instant::now();
            let shown_by = state.last_shown.map(|shown| shown + state.minimum_shown);
            let free_at = if respect_hold {
                shown_by.max(state.held_until)
            } else {
                shown_by
            };
            match free_at.filter(|free| *free > now) {
                Some(free) => free - now,
                None => {
                    state.last_shown = Some(now);
                    state.held_until = hold.map(|held| now + held);
                    return;
                }
            }
        };
        tokio::time::sleep(wait).await;
    }
}

/// Waits for the room frame before this one (when there is one), then claims the screen.
async fn wait_turn(turn: RoomTurn) {
    if let Some(previous) = turn.previous {
        let _ = previous.await;
    }
    claim_screen(&turn.screen, true, None).await;
}

/// Takes away the bubbles anchored to a character who is not in the new picture: their region is
/// gone from the layout, so they would fall back to a fixed spot over someone else.
async fn drop_stale_bubbles(port: u16, present: &[String]) -> Result<(), DevError> {
    let controls: ControlsReply = get_json(port, route::PLAYGROUND_CONTROLS).await?;
    let Some(card) = controls.scene.roleplay.set_value() else {
        return Ok(());
    };
    let stale: Vec<usize> = card
        .bubbles
        .iter()
        .enumerate()
        .filter(|(_, bubble)| {
            bubble
                .anchor
                .as_ref()
                .is_some_and(|anchor| !present.contains(&anchor.region))
        })
        .map(|(index, _)| index + 1)
        .collect();
    for number in stale.into_iter().rev() {
        let removal = SceneChange::Screen {
            edit: ScreenEdit::RoleplaySet {
                roleplay: RoleplayEdit {
                    bubble_remove: Some(number),
                    ..RoleplayEdit::default()
                },
            },
        };
        change(port, &removal).await?;
    }
    Ok(())
}

/// The stored scan of `picture`: who is in it and where.
async fn layout_of(port: u16, picture: &str) -> Result<SceneLayout, PlateFailure> {
    show_layout(port, picture)
        .await
        .map(|stored| stored.layout)
        .map_err(at(PlateStep::Locate))
}

/// Puts `picture` behind the scene with the cast of `cast` (its own scan when it is `cast`).
pub(super) async fn show_with_cast(
    port: u16,
    picture: &str,
    cast: &str,
) -> Result<(), PlateFailure> {
    let layout = if picture == cast {
        None
    } else {
        Some(layout_of(port, cast).await?)
    };
    put_backdrop(port, picture, layout).await
}

/// Puts `picture` behind the scene, with `layout` when given (otherwise the host attaches the
/// picture's own scan, when it has one).
async fn put_backdrop(
    port: u16,
    picture: &str,
    layout: Option<SceneLayout>,
) -> Result<(), PlateFailure> {
    change(
        port,
        &SceneChange::Image {
            slot: ImageSlot::TvBackdrop,
            media: picture.to_owned(),
            caption: None,
            rotation_degrees: 0,
            slide: None,
            layout,
        },
    )
    .await
    .map_err(at(PlateStep::Backdrop))
}

async fn build(
    port: u16,
    name: &str,
    prompt: &str,
    characters: Vec<CharacterRequest>,
    base: Option<&str>,
    turn: RoomTurn,
) -> Result<Shown, PlateFailure> {
    // A plate already in the library with its scan is kept, not rendered again: the name is its
    // identity (a generated plate is stored as `<name>.png`). Use a new name to render a new one.
    let kept = format!("{name}.png");
    if show_layout(port, &kept).await.is_ok() {
        wait_turn(turn).await;
        put_backdrop(port, &kept, None).await?;
        return Ok(Shown {
            summary: format!(
                concat!(
                    "plate {}: kept {} and its scan, not rendered again; the characters in it ",
                    "are as named before"
                ),
                name, kept
            ),
            own_cast: true,
        });
    }
    let picture = generate(port, name, prompt, base).await?;
    let located: LocateReply<LocatedLayout> = post_json(
        port,
        locate_path::LOCATE,
        &LocateRequest {
            picture: picture.clone(),
            prompt: None,
            masks: None,
        },
    )
    .await
    .map_err(at(PlateStep::Locate))?;
    let located = reply_value(located).map_err(at(PlateStep::Locate))?;
    let identified: LocateReply<IdentifiedLayout> = post_json(
        port,
        locate_path::LOCATE_IDENTIFY,
        &IdentifyRequest {
            picture: picture.clone(),
            characters,
            min_confidence_permille: None,
        },
    )
    .await
    .map_err(at(PlateStep::Identify))?;
    let identified = reply_value(identified).map_err(at(PlateStep::Identify))?;
    let named = identified
        .rows
        .iter()
        .filter(|row| row.region.is_some() && row.outcome.key() == "named")
        .count();
    // A tighter shot that leaves someone out does not take them out of the scene: the frame is
    // shown with the cast of the picture it replaces, so they can go on speaking (run 61: an
    // extreme close-up named only the keeper, and three lines were refused).
    let own_cast = named == identified.rows.len();
    let kept_cast = match (&turn.cast_from, own_cast) {
        (Some(room), false) => Some((room.clone(), layout_of(port, room).await?)),
        _ => None,
    };
    wait_turn(turn).await;
    let cast_note = kept_cast.as_ref().map_or_else(String::new, |(room, _)| {
        format!(", with the cast of {room}")
    });
    put_backdrop(port, &picture, kept_cast.map(|(_, layout)| layout)).await?;
    Ok(Shown {
        summary: format!(
            "plate {name}: {picture} is behind the scene ({} regions found, {named} of {} \
             characters named){cast_note}",
            located.layout.regions().len(),
            identified.rows.len()
        ),
        own_cast,
    })
}

/// Generates the plate `name` from `prompt` (an edit of `base` when given) and gives the stored
/// picture's file name.
async fn generate(
    port: u16,
    name: &str,
    prompt: &str,
    base: Option<&str>,
) -> Result<String, PlateFailure> {
    let request = GenerateRequest {
        prompt: prompt.to_owned(),
        name: name.to_owned(),
        purpose: Some(Purpose::Plate),
        aspect: None,
        wait: true,
        base: base.map(str::to_owned),
    };
    let generated: GenerateReply<crate::imagegen::request::JobView> =
        post_json(port, image_path::GENERATE, &request)
            .await
            .map_err(at(PlateStep::Generate))?;
    let job = reply_value(generated).map_err(at(PlateStep::Generate))?;
    let JobState::Stored { provenance, .. } = job.state else {
        return Err(at(PlateStep::Generate)(DevError::PlateNotStored {
            name: job.name,
        }));
    };
    Ok(provenance.media_file)
}
