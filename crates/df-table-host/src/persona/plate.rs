//! The plate of `persona converse plate=NAME`: a new tv backdrop made from what the characters
//! just said, put behind them while a spinner shows on the scene.
//!
//! A dev tool over routes that already exist, like the rest of `converse`: the image service
//! makes the picture (`media generate`), the imagery API finds the people in it (`media locate`),
//! the text service says which region is which character (`media locate identify`), and a scene
//! change puts the picture behind the figures. The prompt is built here from the table and the
//! latest lines, with no model call, so it is the same for the same talk. The title chip of the
//! scene is the spinner while the picture is made, and is put back whether the plate worked or not.

use std::ops::ControlFlow;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use df_playground::fixture::PlateKind;
use df_playground::lobby::change::{RoleplayEdit, ScreenEdit};
use df_playground::lobby::model::SceneTitle;
use df_playground::overlay::Glyph;
use df_playground::scene::{ImageSlot, SceneChange};
use df_scene_layout::SceneLayout;

use super::converse::{Table, Turn};
use super::frame::DroppedWhy;
use super::people_check::{Verified, verified};
use crate::dev::DevError;
use crate::dev_client::{get_json, post_json, reply_value};
use crate::imagegen::model::PictureSize;
use crate::imagegen::request::{BackendView, GenerateReply, GenerateRequest, JobState};
use crate::imagegen::{
    Aspect, BackendLimits, Priority, Purpose, SheetPick, ThingReference, routes::path as image_path,
};
use crate::locate::cli::show as show_layout;
use crate::locate::request::{
    CharacterRequest, IdentifiedLayout, IdentifyRequest, LocateReply, LocateRequest, LocatedLayout,
};
use crate::locate::routes::path as locate_path;
use crate::movement::brief::BriefStyle;
use crate::playground::ChangeReply;
use crate::services::request::{ServicesReply, ServicesReport};
use crate::services::routes::path as services_path;
use crate::wire::{ControlsReply, route};
use df_content::world_bible::Thing;

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
    /// Asking the media library which characters have a reference sheet.
    References,
    Generate,
    Locate,
    Identify,
    Backdrop,
    /// Reading or setting the scene's caption bar.
    Caption,
    /// Reading the scene's title, showing the spinner over it or putting it back.
    Title,
}

impl PlateStep {
    pub fn key(self) -> &'static str {
        match self {
            Self::References => "references",
            Self::Generate => "generate",
            Self::Locate => "locate",
            Self::Identify => "identify",
            Self::Backdrop => "backdrop",
            Self::Caption => "caption",
            Self::Title => "title",
        }
    }
}

/// The text the image service is asked to paint: the room, who sits in it (each by what they are
/// and look like, never by name: the image model paints any word it is given, and a name came out
/// on a signboard), and what they have just been talking about. It forbids readable writing,
/// because the page draws the words.
pub(super) fn picture_prompt(table: &Table, history: &[Turn]) -> String {
    let people: Vec<String> = table
        .speakers()
        .iter()
        .map(|speaker| format!("a person ({})", speaker.job))
        .collect();
    let style = BriefStyle::default();
    let mut text = format!(
        concat!(
            "{} the place where they are, everyone together and facing the viewer. Left to ",
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
    text.push_str(&style.writing);
    text
}

/// What a plate is painted from: its prompt, what it must not show (`BriefStyle::avoid`) and the
/// reference sheets of the things it shows (the story's things in sight, what the characters
/// carry, in the order they matter), each attached as its picture when it has one and room is left
/// after the people's sheets, otherwise described in the prompt.
pub(crate) struct Painting<'a> {
    pub prompt: &'a str,
    pub avoid: &'a [String],
    pub sheets: &'a [Thing],
}

/// `prompt` with how each of `sheets` looks, so a frame painted anew draws them as before.
pub(super) fn with_sheets(prompt: String, sheets: &[Thing]) -> String {
    if sheets.is_empty() {
        return prompt;
    }
    let looks: Vec<String> = sheets
        .iter()
        .map(|sheet| format!("{}: {}", sheet.name, sheet.look))
        .collect();
    format!(
        "{prompt} The things in sight, the same in every picture: {}",
        looks.join(" ")
    )
}

/// The pictures a plate is drawn after, within its room, and the sheets it describes in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FittedPictures {
    /// The people's reference sheets, in the cast's order.
    pub people: Vec<String>,
    /// The things' close-ups, each with what it shows.
    pub things: Vec<ThingReference>,
    /// The sheets described in words: those with no close-up yet, and those whose close-up found
    /// no room.
    pub described: Vec<Thing>,
    /// The pictures left out for want of room.
    pub dropped: Vec<String>,
}

/// Fills `room` pictures with the people's reference sheets first, then the close-ups of the
/// `sheets` that have one, in order; what does not fit is dropped (a thing whose close-up is
/// dropped is described in words instead).
pub(super) fn fit_pictures(
    room: usize,
    mut people: Vec<String>,
    sheets: &[Thing],
) -> FittedPictures {
    let mut dropped = people.split_off(room.min(people.len()));
    let mut things = Vec::new();
    let mut described = Vec::new();
    for sheet in sheets {
        match &sheet.picture {
            Some(picture) if people.len() + things.len() < room => things.push(ThingReference {
                picture: picture.clone(),
                shows: sheet.name.clone(),
            }),
            Some(picture) => {
                dropped.push(picture.clone());
                described.push(sheet.clone());
            }
            None => described.push(sheet.clone()),
        }
    }
    FittedPictures {
        people,
        things,
        described,
        dropped,
    }
}

/// The request the plate `name` is generated with: `painting`, an edit of `base` when given, drawn
/// after the `fitted` pictures, with the sheets they leave in words described in its prompt.
pub(super) fn plate_request(
    name: &str,
    painting: &Painting<'_>,
    base: Option<&str>,
    fitted: FittedPictures,
) -> GenerateRequest {
    GenerateRequest {
        prompt: with_sheets(painting.prompt.to_owned(), &fitted.described),
        name: name.to_owned(),
        purpose: Some(Purpose::Plate),
        aspect: None,
        wait: true,
        base: base.map(str::to_owned),
        references: fitted.people,
        things: fitted.things,
        avoid: painting.avoid.to_vec(),
        left_out: fitted.dropped,
        priority: Priority::Normal,
        sheet: None,
    }
}

/// The request the insert `name` is generated with: painted anew from `painting` (never an edit of
/// the room, whose people it would keep, and no reference sheet: nobody's face is in an insert), in
/// the shape of the picture the backend makes when it was switched to a `size`, so the shape asked
/// for is the shape that comes back and nothing is padded to fit.
pub(super) fn insert_request(
    name: &str,
    painting: &Painting<'_>,
    size: Option<PictureSize>,
) -> GenerateRequest {
    let mut request = plate_request(name, painting, None, fit_pictures(0, Vec::new(), &[]));
    request.aspect = size.and_then(Aspect::of_size);
    request
}

/// The size the host's image backend makes every picture at, when it was switched to one.
async fn backend_size(port: u16) -> Result<Option<PictureSize>, DevError> {
    let view: GenerateReply<BackendView> = get_json(port, image_path::BACKEND).await?;
    Ok(reply_value(view)?.size)
}

/// What the host's image backend takes, from the services report.
pub(crate) async fn image_limits(port: u16) -> Result<BackendLimits, DevError> {
    let reply: ServicesReply<ServicesReport> = get_json(port, services_path::SERVICES).await?;
    Ok(reply_value(reply)?.imagegen.limits)
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

/// Sets the scene's caption bar to `text` titled `title` (a close-up's item name), or takes it
/// away when `text` is empty. Any caption change replaces the title: none given clears it.
fn caption_change(text: &str, title: Option<&str>) -> SceneChange {
    SceneChange::Screen {
        edit: ScreenEdit::RoleplaySet {
            roleplay: RoleplayEdit {
                caption: Some(text.to_owned()),
                caption_title: title.map(str::to_owned),
                ..RoleplayEdit::default()
            },
        },
    }
}

/// The caption bar the scene shows now through a live change, if one is.
async fn current_caption(port: u16) -> Result<Option<String>, DevError> {
    let controls: ControlsReply = get_json(port, route::PLAYGROUND_CONTROLS).await?;
    Ok(controls
        .scene
        .roleplay
        .set_value()
        .and_then(|card| card.caption.clone()))
}

/// Puts `before` back as the scene's caption bar, or takes the bar away when there was none.
pub(super) async fn put_caption_back(port: u16, before: Option<String>) -> Result<(), DevError> {
    change(
        port,
        &caption_change(before.as_deref().unwrap_or_default(), None),
    )
    .await
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
    let painting = Painting {
        prompt: &picture_prompt(table, history),
        avoid: &BriefStyle::default().avoid,
        sheets: &[],
    };
    deliver_plate(port, name, &painting, characters, None).await
}

/// Makes the plate `name` from `painting` (an edit of `base`, a picture of the media library, when
/// given) and puts it behind the scene, with `characters` located and named in it. The spinner
/// title is shown from the start and the title that was there is put back at the end, also when a
/// step failed; the failure is that of the step, and a title that could not be put back is not
/// hidden by it.
pub(crate) async fn deliver_plate(
    port: u16,
    name: &str,
    painting: &Painting<'_>,
    characters: Vec<CharacterRequest>,
    base: Option<&str>,
) -> Result<String, PlateFailure> {
    let before = start_spinner(port).await.map_err(at(PlateStep::Title))?;
    let alone = RoomTurn {
        number: 1,
        screen: SharedScreen::default(),
        cast_from: None,
        cued: false,
        emergency: None,
    };
    let made = make_frame(port, name, painting, characters, base, alone)
        .await
        .map(|shown| shown.summary);
    let restored = put_title_back(port, before).await;
    match (made, restored) {
        (Ok(summary), Ok(())) => Ok(summary),
        (Err(failure), _) => Err(failure),
        (Ok(_), Err(error)) => Err(at(PlateStep::Title)(error)),
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
    /// The frame's number in its watch: a room frame shows only when no newer one is up or ready.
    pub number: u32,
    pub screen: SharedScreen,
    pub cast_from: Option<String>,
    /// Whether a line cued it (a change in the talk), not the schedule: once it is up, no older
    /// room frame follows it.
    pub cued: bool,
    /// The hold of an emergency plate (a declared action): it is made first in the backend's own
    /// line, goes up as soon as it is ready, without a slot, and holds the screen for this long.
    pub emergency: Option<Duration>,
}

impl RoomTurn {
    /// How urgent the plate's picture is for the image backend.
    fn priority(&self) -> Priority {
        if self.emergency.is_some() {
            Priority::Emergency
        } else {
            Priority::Normal
        }
    }

    /// The view of the people's sheets its picture is shown: a declared action painted anew shows
    /// its person whole and small, so the full-length view; any other leaves it to the request.
    fn sheet(&self, base: Option<&str>) -> Option<SheetPick> {
        (self.emergency.is_some() && base.is_none()).then_some(SheetPick::FullLength)
    }
}

/// A frame put on screen: what was done, whether it shows with its own scan (every character
/// asked for named in it) rather than with the cast of the picture it replaced, and, for an insert
/// shown with a caption, the caption it replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Shown {
    pub summary: String,
    pub own_cast: bool,
    pub captioned: Option<Captioned>,
    /// Whether it went up: a room frame a newer one overtook is made and never shown.
    pub on_screen: bool,
    /// Why a frame that was made did not go up, when it is made and neither up nor failed.
    pub dropped: Option<DroppedWhy>,
    /// How long a room frame painted now took to make (generated, scanned and named), before it
    /// waited for its slot; none for an insert or a kept plate.
    pub making: Option<Duration>,
}

/// An insert's caption on the scene's caption bar: the caption that was there before it (none
/// when there was no bar), put back when the room comes back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Captioned {
    pub before: Option<String>,
}

/// Makes the plate `name` and puts it behind the scene, as `deliver_plate` does but without the
/// spinner (its caller shows one), and only at its `turn`: a frame made alongside an earlier one
/// waits for that one to be shown, so an older picture never replaces a newer one, and waits
/// while an insert holds the screen. Bubbles of characters not in the new picture are taken away.
pub(super) async fn make_frame(
    port: u16,
    name: &str,
    painting: &Painting<'_>,
    characters: Vec<CharacterRequest>,
    base: Option<&str>,
    turn: RoomTurn,
) -> Result<Shown, PlateFailure> {
    let present: Vec<String> = characters
        .iter()
        .map(|character| character.name.clone())
        .collect();
    let shown = build(port, name, painting, characters, base, &turn).await?;
    drop_stale_bubbles(port, &present)
        .await
        .map_err(at(PlateStep::Backdrop))?;
    Ok(shown)
}

/// What an insert is made from and shown with (see [`make_insert`]).
pub(super) struct InsertRequest<'a> {
    /// The insert's name in the media library.
    pub name: &'a str,
    pub prompt: &'a str,
    /// What the picture must not show.
    pub avoid: &'a [String],
    /// The close-up the item's reference sheet keeps: shown again, none made.
    pub drawn: Option<&'a str>,
    /// The picture it cuts away from, whose cast the scene keeps.
    pub room: &'a str,
    /// A found clue's words, for the caption bar.
    pub caption: Option<&'a str>,
    /// The item's name that titles the caption (its reference sheet's).
    pub caption_title: Option<&'a str>,
    /// How long the insert holds the screen.
    pub hold: Duration,
}

/// Makes the insert `request.name` (a cut to an item filling the screen) and puts it behind the
/// scene as soon as it is made and the last picture has been up the screen's least time: it waits
/// for no room frame, and holds the screen for `hold`, so a room frame that finishes meanwhile
/// waits. It is painted anew from the item's look, never an edit of the room (`insert_request`).
/// Nobody's face is in it, so it is not scanned: it is shown with the layout of `room`, the
/// picture it cuts away from, so the scene keeps its cast (who may speak, and where their bubbles
/// sit) while the camera is on the item. With a `caption` (a found clue's words, titled by
/// `caption_title`), the scene's caption bar shows it from the moment the insert is up; the
/// caption it replaced is answered, to be put back with the room. With `drawn` (the close-up an
/// item's reference sheet keeps), that picture is shown again and none is made, so the item looks
/// the same each time. The picture shown is answered too, for the sheet to keep.
pub(super) async fn make_insert(
    port: u16,
    screen: &SharedScreen,
    request: InsertRequest<'_>,
) -> Result<(String, String, Option<Captioned>), PlateFailure> {
    let InsertRequest {
        name,
        prompt,
        avoid,
        drawn,
        room,
        caption,
        caption_title,
        hold,
    } = request;
    let picture = match drawn {
        Some(drawn) => drawn.to_owned(),
        None => {
            let painting = Painting {
                prompt,
                avoid,
                sheets: &[],
            };
            let size = backend_size(port).await.map_err(at(PlateStep::Generate))?;
            generate(port, insert_request(name, &painting, size)).await?
        }
    };
    let layout = layout_of(port, room).await?;
    // A clue's close-up waits out the one before it, so each is up its whole hold and its words
    // can be read (dungeon run 5: two went up 8 s apart).
    claim_screen(screen, true, Some(hold)).await;
    // The caption to put back is read once the screen is this insert's: read before the wait, it
    // was the last clue's own words, which then came back over the room (dungeon run 6).
    let before = match caption {
        Some(_) => Some(
            current_caption(port)
                .await
                .map_err(at(PlateStep::Caption))?,
        ),
        None => None,
    };
    put_backdrop(port, &picture, Some(layout), PlateKind::CloseUp).await?;
    let captioned = match (caption, before) {
        (Some(text), Some(before)) => {
            change(port, &caption_change(text, caption_title))
                .await
                .map_err(at(PlateStep::Caption))?;
            Some(Captioned { before })
        }
        _ => None,
    };
    Ok((
        format!(
            "insert {name}: {picture} is behind the scene, with the cast of {room}{}{}",
            if drawn.is_some() {
                ", its sheet's picture"
            } else {
                ""
            },
            if captioned.is_some() {
                ", captioned"
            } else {
                ""
            }
        ),
        picture,
        captioned,
    ))
}

/// Cuts to `picture`, a picture the library already holds (a finished painting), as it is: nothing
/// is made and no model is asked. It is shown full frame with the layout of `room`, the picture it
/// cuts away from, so the scene keeps its cast, and the scene's caption bar shows `caption` (the
/// caption it replaced is answered, to be put back with the room). It is shown at once
/// (`claim_reveal`) and holds the screen for `hold`.
pub(super) async fn make_reveal(
    port: u16,
    screen: &SharedScreen,
    reveal: Reveal<'_>,
    mut ticket: RevealTicket,
) -> Result<(String, Option<Captioned>), PlateFailure> {
    let Reveal {
        name,
        picture,
        room,
        caption,
        hold,
    } = reveal;
    let layout = layout_of(port, room).await?;
    claim_reveal(screen, &mut ticket, hold).await;
    let before = match caption {
        Some(_) => Some(
            current_caption(port)
                .await
                .map_err(at(PlateStep::Caption))?,
        ),
        None => None,
    };
    put_backdrop(port, picture, Some(layout), PlateKind::CloseUp).await?;
    let captioned = match (caption, before) {
        (Some(text), Some(before)) => {
            change(port, &caption_change(text, None))
                .await
                .map_err(at(PlateStep::Caption))?;
            Some(Captioned { before })
        }
        _ => None,
    };
    Ok((
        format!(
            "reveal {name}: {picture} is behind the scene, with the cast of {room}{}",
            if captioned.is_some() {
                ", captioned"
            } else {
                ""
            }
        ),
        captioned,
    ))
}

/// What a revealed picture is shown with (see [`make_reveal`]).
pub(super) struct Reveal<'a> {
    /// The reveal's name, for the transcript.
    pub name: &'a str,
    /// The picture of the media library to show.
    pub picture: &'a str,
    /// The picture it cuts away from, whose cast the scene keeps.
    pub room: &'a str,
    /// The words for the caption bar.
    pub caption: Option<&'a str>,
    /// How long it holds the screen.
    pub hold: Duration,
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
    /// How long a room plate stays up before the next one goes up (`every=`), so plates come at a
    /// steady pace however long each took to make; zero puts each up as soon as it is ready.
    pub room_every: Duration,
    /// The room plate up last: its frame number and when it went up.
    last_room: Option<(u32, Instant)>,
    /// The room frames made and waiting for their slot.
    room_ready: Vec<u32>,
    /// The newest room frame cued by a line that has gone up: an older one would step the screen
    /// back past the change it shows.
    cued_up: Option<u32>,
    /// The newest declaration (an emergency plate): its frame number and its name. Every room
    /// plate started before it, still waiting or being made, is superseded: the talk it shows is
    /// older than the action declared.
    declared: Option<(u32, String)>,
    /// The revealed pictures (`@show`) form a sequence: the next ticket to give, the one served
    /// next, the tickets given up without being served, when the picture up now (a reveal) has
    /// held its whole requested time, and when the last reveal went up.
    reveal_next: u64,
    reveal_serving: u64,
    reveal_abandoned: Vec<u64>,
    reveal_until: Option<Instant>,
    reveal_shown: Option<Instant>,
}

impl Screen {
    /// When the picture up last has been up long enough for another to replace it: the screen's
    /// least time for a generated picture, and for a revealed one the time it was asked to hold
    /// (an explicit short hold is the caller's own request and is not stretched).
    fn shown_by(&self) -> Option<Instant> {
        self.last_shown.map(|shown| {
            if self.reveal_shown == Some(shown) {
                shown
            } else {
                shown + self.minimum_shown
            }
        })
    }

    /// Whether a revealed picture is queued and not yet up: pictures that are not reveals wait
    /// for it, so a sequence of reveals is never broken into.
    fn reveals_pending(&mut self) -> bool {
        // Tickets given up without being served are skipped.
        while self.reveal_abandoned.contains(&self.reveal_serving) {
            self.reveal_serving += 1;
        }
        self.reveal_serving < self.reveal_next
    }
}

pub(super) type SharedScreen = Arc<Mutex<Screen>>;

/// Looks at the screen under its lock with `look` until it answers, sleeping between looks for
/// the wait it names, with the lock let go: the one loop every claim on the screen waits in.
async fn wait_on_screen<T>(
    screen: &SharedScreen,
    mut look: impl FnMut(&mut Screen, Instant) -> ControlFlow<T, Duration>,
) -> T {
    loop {
        let wait = {
            let mut state = screen.lock().unwrap_or_else(PoisonError::into_inner);
            match look(&mut state, Instant::now()) {
                ControlFlow::Break(answer) => return answer,
                ControlFlow::Continue(wait) => wait,
            }
        };
        tokio::time::sleep(wait).await;
    }
}

/// The wait until `free_at` while it is still ahead of `now`; none once the screen is free.
fn wait_until(free_at: Option<Instant>, now: Instant) -> Option<Duration> {
    free_at.filter(|free| *free > now).map(|free| free - now)
}

/// Waits until a new picture may go up and claims the screen for it: the screen's least time after
/// the last picture and, when `respect_hold`, not while an insert holds it. The claim marks the
/// picture as up now and holds the screen for `hold` (an insert) or releases it, under one lock,
/// so two frames never pass together.
pub(super) async fn claim_screen(
    screen: &SharedScreen,
    respect_hold: bool,
    hold: Option<Duration>,
) {
    wait_on_screen(screen, |state, now| {
        if state.reveals_pending() {
            return ControlFlow::Continue(ROOM_QUEUE_LOOK);
        }
        let shown_by = state.shown_by();
        let free_at = if respect_hold {
            shown_by.max(state.held_until)
        } else {
            shown_by
        };
        if let Some(wait) = wait_until(free_at, now) {
            return ControlFlow::Continue(wait);
        }
        state.last_shown = Some(now);
        state.held_until = hold.map(|held| now + held);
        ControlFlow::Break(())
    })
    .await;
}

/// Claims the screen for the room coming back after the insert that went up at `insert_shown`, as
/// `claim_screen` does without the hold, unless another picture went up after the insert (a room
/// frame that waited out the hold): then the screen stays that picture's, and `false` is answered,
/// so the room is never put back over a newer frame.
pub(super) async fn claim_after_insert(screen: &SharedScreen, insert_shown: Instant) -> bool {
    wait_on_screen(screen, |state, now| {
        if state.last_shown.is_some_and(|shown| shown > insert_shown) {
            return ControlFlow::Break(false);
        }
        let shown_by = state.shown_by();
        if let Some(wait) = wait_until(shown_by, now) {
            return ControlFlow::Continue(wait);
        }
        state.last_shown = Some(now);
        state.held_until = None;
        ControlFlow::Break(true)
    })
    .await
}

/// A room plate waiting for its slot: however its claim ends, even with its frame aborted
/// mid-wait, it leaves the waiting plates, so it never holds back a newer one.
struct Waiting<'a> {
    screen: &'a SharedScreen,
    number: u32,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.screen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .room_ready
            .retain(|ready| *ready != self.number);
    }
}

/// How many finished room plates may wait for slots at once: the oldest of more gives way, so the
/// screen never runs far behind the talk.
const ROOM_QUEUE: usize = 2;
/// How soon a plate waiting behind an older one looks again, once a slot is free.
const ROOM_QUEUE_LOOK: Duration = Duration::from_millis(100);

/// Waits for the slot of room frame `number` and claims the screen for it, or answers `false`
/// when it is overtaken. Plates finished together go up one slot apart, the oldest first (dungeon
/// run 12: when only the newest of three ready ones went up, the other two were thrown away and
/// the next slot had nothing, an 89 s gap), and one with `ROOM_QUEUE` newer ones waiting gives
/// way. A plate finished after a newer one went up still takes a slot: making times spread from
/// 90 to 220 s, and dropping every older one a fast plate passed left the screen unchanged for
/// 171 s (dungeon run 10). A slot comes `room_every` after the last room plate went up, never
/// while an insert holds the screen and never before the last picture has been up the screen's
/// least time. A plate older than a line-cued one already up never goes up: it shows the talk
/// before the change.
pub(super) async fn claim_room(screen: &SharedScreen, number: u32, cued: bool) -> bool {
    screen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .room_ready
        .push(number);
    let _waiting = Waiting { screen, number };
    wait_on_screen(screen, |state, now| {
        let newer_waiting = state
            .room_ready
            .iter()
            .filter(|ready| **ready > number)
            .count();
        let overtaken = newer_waiting >= ROOM_QUEUE
            || state.cued_up.is_some_and(|up| up > number)
            || state.declared.as_ref().is_some_and(|(at, _)| *at > number);
        if overtaken {
            state.room_ready.retain(|ready| *ready != number);
            return ControlFlow::Break(false);
        }
        if state.reveals_pending() {
            return ControlFlow::Continue(ROOM_QUEUE_LOOK);
        }
        let free_at = [
            state.shown_by(),
            state.held_until,
            state.last_room.map(|(_, up)| up + state.room_every),
        ]
        .into_iter()
        .flatten()
        .max();
        if let Some(wait) = wait_until(free_at, now) {
            return ControlFlow::Continue(wait);
        }
        // An older plate waiting takes this slot; this one a later one.
        if state.room_ready.iter().any(|ready| *ready < number) {
            return ControlFlow::Continue(ROOM_QUEUE_LOOK);
        }
        state.last_shown = Some(now);
        state.held_until = None;
        state.last_room = Some((number, now));
        if cued {
            state.cued_up = state.cued_up.max(Some(number));
        }
        state.room_ready.retain(|ready| *ready != number);
        ControlFlow::Break(true)
    })
    .await
}

/// Records that frame `number`, `name`, is a declaration (an emergency plate). Every room plate
/// started before it that waits for its slot gives up its place at once, so it holds back no newer
/// one; one still being made is dropped when it is made. Answers how many waiting plates gave up.
pub(super) fn begin_declaration(screen: &SharedScreen, number: u32, name: &str) -> usize {
    let mut state = screen.lock().unwrap_or_else(PoisonError::into_inner);
    state.declared = Some((number, name.to_owned()));
    let before = state.room_ready.len();
    state.room_ready.retain(|ready| *ready > number);
    before - state.room_ready.len()
}

/// The name of the declared plate that supersedes room frame `number`, when one does: a
/// declaration made after the frame started.
pub(super) fn superseded_by(screen: &SharedScreen, number: u32) -> Option<String> {
    screen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .declared
        .as_ref()
        .filter(|(at, _)| *at > number)
        .map(|(_, name)| name.clone())
}

/// Claims the screen for the emergency plate `number`, or answers `false` when a newer declaration
/// took its place. It waits for no slot and for no insert's hold, nor for the plates ahead of it:
/// only the least time the picture before it stays up (so nothing flickers faster than it can be
/// followed). Once up it holds the screen for `hold`, and the next room plate's slot counts from
/// it.
pub(super) async fn claim_declared(screen: &SharedScreen, number: u32, hold: Duration) -> bool {
    wait_on_screen(screen, |state, now| {
        if state.declared.as_ref().is_some_and(|(at, _)| *at > number) {
            return ControlFlow::Break(false);
        }
        // A revealed picture, and the ones queued behind it, are shown their whole requested time
        // first: the emergency plate takes its turn after them.
        if state.reveals_pending() {
            return ControlFlow::Continue(ROOM_QUEUE_LOOK);
        }
        let shown_by = state.shown_by().max(state.reveal_until);
        if let Some(wait) = wait_until(shown_by, now) {
            return ControlFlow::Continue(wait);
        }
        state.last_shown = Some(now);
        state.held_until = Some(now + hold);
        state.last_room = Some((number, now));
        ControlFlow::Break(true)
    })
    .await
}

/// A place in the sequence of revealed pictures. A ticket given up without being served (its
/// frame aborted) is skipped, so it never holds back the ones behind it.
pub(super) struct RevealTicket {
    screen: SharedScreen,
    number: u64,
    served: bool,
    /// How many revealed pictures were up or queued ahead of this one when it was given.
    pub behind: usize,
}

impl Drop for RevealTicket {
    fn drop(&mut self) {
        if !self.served {
            let mut state = self.screen.lock().unwrap_or_else(PoisonError::into_inner);
            state.reveal_abandoned.push(self.number);
        }
    }
}

/// Takes the next place in the sequence of revealed pictures. Every one asked for is taken, however
/// many are up or queued already: a place is a number and one waiting task, and the sequence shows
/// each in order for its whole hold, so the queue is bounded by what the runner sends.
pub(super) fn queue_reveal(screen: &SharedScreen) -> RevealTicket {
    let mut state = screen.lock().unwrap_or_else(PoisonError::into_inner);
    state.reveals_pending();
    let queued = usize::try_from(state.reveal_next - state.reveal_serving).unwrap_or(usize::MAX);
    let up = usize::from(
        state
            .reveal_until
            .is_some_and(|until| until > Instant::now()),
    );
    let behind = queued + up;
    let number = state.reveal_next;
    state.reveal_next += 1;
    RevealTicket {
        screen: Arc::clone(screen),
        number,
        served: false,
        behind,
    }
}

/// Claims the screen for a revealed picture in its turn, then shows it at once: it waits for the
/// revealed pictures before it to hold their whole requested time (a sequence, never one
/// displacing another), for no slot, no insert's hold and no plate ahead of it, and, when the
/// picture before it is a generated one, for the least time that stays up (nothing changes faster
/// than a person can follow); a revealed picture's own hold is its requested time, however short.
/// It holds the screen for `hold`. It is no new picture of the talk, so it touches neither the
/// room plates' slots nor the plates waiting: they wait out the sequence and are shown after it.
pub(super) async fn claim_reveal(screen: &SharedScreen, ticket: &mut RevealTicket, hold: Duration) {
    let number = ticket.number;
    wait_on_screen(screen, |state, now| {
        state.reveals_pending();
        if state.reveal_serving != number {
            return ControlFlow::Continue(ROOM_QUEUE_LOOK);
        }
        let free_at = state.shown_by().max(state.reveal_until);
        if let Some(wait) = wait_until(free_at, now) {
            return ControlFlow::Continue(wait);
        }
        state.last_shown = Some(now);
        state.reveal_shown = Some(now);
        state.reveal_until = Some(now + hold);
        state.held_until = Some(now + hold);
        state.reveal_serving += 1;
        ControlFlow::Break(())
    })
    .await;
    ticket.served = true;
}

/// Claims the screen for the room coming back after the revealed picture that went up at
/// `reveal_shown`, as `claim_after_insert` does, but without the least time a generated picture
/// stays up: the picture was up its whole requested hold, which is what was asked. A newer picture
/// that went up meanwhile (the next of the sequence) keeps the screen, and `false` is answered.
pub(super) async fn claim_after_reveal(screen: &SharedScreen, reveal_shown: Instant) -> bool {
    wait_on_screen(screen, |state, now| {
        if state.last_shown.is_some_and(|shown| shown > reveal_shown) || state.reveals_pending() {
            return ControlFlow::Break(false);
        }
        if let Some(wait) = wait_until(state.reveal_until, now) {
            return ControlFlow::Continue(wait);
        }
        state.last_shown = Some(now);
        state.held_until = None;
        ControlFlow::Break(true)
    })
    .await
}

/// Waits for the screen as `turn` asks: an emergency plate takes it at once, a room plate waits
/// for its slot. `false`: it was overtaken or superseded.
async fn claim(turn: &RoomTurn) -> bool {
    match turn.emergency {
        Some(hold) => claim_declared(&turn.screen, turn.number, hold).await,
        None => claim_room(&turn.screen, turn.number, turn.cued).await,
    }
}

/// The room frame `name` made but not shown: a declaration made after it started superseded it,
/// else a newer plate took its slot.
pub(super) fn not_shown(
    turn: &RoomTurn,
    name: &str,
    own_cast: bool,
    making: Option<Duration>,
) -> Shown {
    match superseded_by(&turn.screen, turn.number) {
        Some(by) => Shown {
            summary: format!("plate {name}: made, not shown, superseded by {by}"),
            own_cast,
            captioned: None,
            on_screen: false,
            dropped: Some(DroppedWhy::Superseded),
            making,
        },
        None => overtaken(name, own_cast, making),
    }
}

/// The frame `name` made but not shown, as a newer one took its slot.
fn overtaken(name: &str, own_cast: bool, making: Option<Duration>) -> Shown {
    Shown {
        summary: format!("plate {name}: made, not shown, as a newer plate took its slot"),
        own_cast,
        captioned: None,
        on_screen: false,
        dropped: Some(DroppedWhy::Overtaken),
        making,
    }
}

/// Takes away the bubbles anchored to a character who is not in the new picture: their region is
/// gone from the layout, so they would fall back to a fixed spot over someone else.
pub(super) async fn drop_stale_bubbles(port: u16, present: &[String]) -> Result<(), DevError> {
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

/// How many figures the stored scan of `picture` found, or none when it has no scan to read.
pub(super) async fn figures_in(port: u16, picture: &str) -> Option<usize> {
    show_layout(port, picture)
        .await
        .ok()
        .map(|stored| stored.layout.regions().len())
}

/// Puts the room `picture` behind the scene with the cast of `cast` (its own scan when it is
/// `cast`, otherwise as a room that kept the cast before it).
pub(super) async fn show_with_cast(
    port: u16,
    picture: &str,
    cast: &str,
) -> Result<(), PlateFailure> {
    if picture == cast {
        put_backdrop(port, picture, None, PlateKind::Room).await
    } else {
        let layout = layout_of(port, cast).await?;
        put_backdrop(port, picture, Some(layout), PlateKind::RoomKeptCast).await
    }
}

/// Puts `picture` behind the scene as a plate of kind `plate`, with `layout` when given
/// (otherwise the host attaches the picture's own scan, when it has one).
async fn put_backdrop(
    port: u16,
    picture: &str,
    layout: Option<SceneLayout>,
    plate: PlateKind,
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
            plate,
        },
    )
    .await
    .map_err(at(PlateStep::Backdrop))
}

/// A plate in the media library with its scan, not put on screen by its making: kept from before,
/// or painted, scanned and named now.
pub(super) struct ScannedPlate {
    /// Its media library name.
    pub picture: String,
    /// How it was made; none when it was kept, not rendered again.
    pub made: Option<PlateMade>,
}

/// What painting, scanning and naming a plate found.
pub(super) struct PlateMade {
    /// How many regions the scan found.
    pub regions: usize,
    /// How many of the characters asked for were named in it, of `asked`.
    pub named: usize,
    pub asked: usize,
    /// Which pictures it was drawn after, and which found no room.
    pub sheets_note: String,
    /// When its making began.
    pub since: Instant,
}

/// Makes the plate `name` from `painting` (an edit of `base` when given) with `characters`
/// located and named in it, and shows it nowhere. A plate already in the library with its scan is
/// kept, not rendered again: the name is its identity (a generated plate is stored as
/// `<name>.png`). Use a new name to render a new one.
pub(super) async fn scan_plate(
    port: u16,
    name: &str,
    painting: &Painting<'_>,
    characters: Vec<CharacterRequest>,
    base: Option<&str>,
) -> Result<ScannedPlate, PlateFailure> {
    scan_plate_with(port, name, painting, characters, base, (Priority::Normal, None)).await
}

/// [`scan_plate`] with the picture's urgency for the image backend and the view of the people's
/// reference sheets it is shown (none: the request's own, see `SheetPick`).
pub(super) async fn scan_plate_with(
    port: u16,
    name: &str,
    painting: &Painting<'_>,
    characters: Vec<CharacterRequest>,
    base: Option<&str>,
    (priority, sheet): (Priority, Option<SheetPick>),
) -> Result<ScannedPlate, PlateFailure> {
    let kept = format!("{name}.png");
    if show_layout(port, &kept).await.is_ok() {
        return Ok(ScannedPlate {
            picture: kept,
            made: None,
        });
    }
    let since = Instant::now();
    // The pictures a plate is drawn after are as many as the host's image backend takes: the
    // people's sheets first, then the things' close-ups (owner, 2026-11-09).
    let limits = image_limits(port)
        .await
        .map_err(at(PlateStep::References))?;
    let room = limits
        .maximum_pictures
        .saturating_sub(usize::from(base.is_some()));
    let people = reference_sheets(port, &characters).await?;
    let fitted = fit_pictures(room, people, painting.sheets);
    let mut attached: Vec<&str> = fitted.people.iter().map(String::as_str).collect();
    attached.extend(fitted.things.iter().map(|thing| thing.picture.as_str()));
    let mut sheets_note = if attached.is_empty() {
        "no reference sheets".to_owned()
    } else {
        format!("drawn after {}", attached.join(", "))
    };
    if !fitted.dropped.is_empty() {
        sheets_note.push_str(&format!(
            "; no room for {} (the backend takes {} pictures)",
            fitted.dropped.join(", "),
            limits.maximum_pictures
        ));
    }
    let mut request = plate_request(name, painting, base, fitted);
    request.priority = priority;
    request.sheet = sheet;
    let picture = generate(port, request).await?;
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
    Ok(ScannedPlate {
        picture,
        made: Some(PlateMade {
            regions: located.layout.regions().len(),
            named,
            asked: identified.rows.len(),
            sheets_note,
            since,
        }),
    })
}

async fn build(
    port: u16,
    name: &str,
    painting: &Painting<'_>,
    characters: Vec<CharacterRequest>,
    base: Option<&str>,
    turn: &RoomTurn,
) -> Result<Shown, PlateFailure> {
    let scanned = scan_plate_with(
        port,
        name,
        painting,
        characters,
        base,
        (turn.priority(), turn.sheet(base)),
    )
    .await?;
    show_scanned(port, name, scanned, turn).await
}

/// Puts the scanned plate `name` behind the scene at its `turn`, or says it was not shown.
pub(super) async fn show_scanned(
    port: u16,
    name: &str,
    scanned: ScannedPlate,
    turn: &RoomTurn,
) -> Result<Shown, PlateFailure> {
    let ScannedPlate { picture, made } = scanned;
    let Some(made) = made else {
        if !claim(turn).await {
            return Ok(not_shown(turn, name, true, None));
        }
        put_backdrop(port, &picture, None, PlateKind::Room).await?;
        return Ok(Shown {
            summary: format!(
                concat!(
                    "plate {}: kept {} and its scan, not rendered again; the characters in it ",
                    "are as named before"
                ),
                name, picture
            ),
            own_cast: true,
            captioned: None,
            on_screen: true,
            dropped: None,
            making: None,
        });
    };
    // A tighter shot that leaves someone out does not take them out of the scene: the frame is
    // shown with the cast of the picture it replaces, so they can go on speaking (run 61: an
    // extreme close-up named only the keeper, and three lines were refused).
    let own_cast = made.named == made.asked;
    let kept_cast = match (&turn.cast_from, own_cast) {
        (Some(room), false) => Some((room.clone(), layout_of(port, room).await?)),
        _ => None,
    };
    let making = Some(made.since.elapsed());
    if !claim(turn).await {
        return Ok(not_shown(turn, name, own_cast, making));
    }
    let cast_note = kept_cast.as_ref().map_or_else(String::new, |(room, _)| {
        format!(", with the cast of {room}")
    });
    match kept_cast {
        Some((_, layout)) => {
            put_backdrop(port, &picture, Some(layout), PlateKind::RoomKeptCast).await?;
        }
        None => put_backdrop(port, &picture, None, PlateKind::Room).await?,
    }
    Ok(Shown {
        summary: format!(
            "plate {name}: {picture} is behind the scene ({} regions found, {} of {} characters \
             named; {}){cast_note}",
            made.regions, made.named, made.asked, made.sheets_note
        ),
        own_cast,
        captioned: None,
        on_screen: true,
        dropped: None,
        making,
    })
}

/// One way of painting a declared plate: its name in the library, what it is painted from and the
/// picture it edits, when it edits one.
pub(super) struct Attempt<'a> {
    pub name: String,
    pub painting: Painting<'a>,
    pub base: Option<&'a str>,
}

/// What making a declared plate came to.
pub(super) struct DeclaredMade {
    /// What was done: shown, superseded, or refused for showing more people than the cast.
    pub shown: Shown,
    /// The accepted attempt's picture in the library, which keeps its own scan as its layout.
    pub picture: Option<String>,
    /// Which attempt (from 1) is the picture, or the last one made when it was refused.
    pub attempt: u8,
    /// How long each attempt made took.
    pub makings: Vec<Duration>,
    /// How many people the last attempt showed, when every attempt showed more than the cast.
    pub rejected: Option<usize>,
}

/// Makes a declared plate by `attempts` (the first as the caller chose, the second a fresh paint),
/// checking each against the cast (`people_check`): the first that shows no more people than the
/// `characters` is shown at its `turn`; when every attempt shows more, nothing is shown and the
/// scene stays as it is. Every attempt is made first in the backend's own line.
pub(super) async fn make_declared(
    port: u16,
    attempts: [Attempt<'_>; 2],
    characters: Vec<CharacterRequest>,
    turn: RoomTurn,
) -> Result<DeclaredMade, PlateFailure> {
    let present: Vec<String> = characters
        .iter()
        .map(|character| character.name.clone())
        .collect();
    let made = verified(characters.len(), |attempt| {
        let way = &attempts[usize::from(attempt) - 1];
        let characters = characters.clone();
        let urgency = (turn.priority(), turn.sheet(way.base));
        async move {
            let scanned = scan_plate_with(
                port,
                &way.name,
                &way.painting,
                characters,
                way.base,
                urgency,
            )
            .await?;
            let (people, making) = scanned
                .made
                .as_ref()
                .map_or((None, Duration::ZERO), |made| {
                    (Some(made.regions), made.since.elapsed())
                });
            Ok((scanned, people, making))
        }
    })
    .await?;
    let makings = made.makings;
    match made.verdict {
        Verified::Accepted { plate, attempt } => {
            let name = &attempts[usize::from(attempt) - 1].name;
            let picture = plate.picture.clone();
            let shown = show_scanned(port, name, plate, &turn).await?;
            drop_stale_bubbles(port, &present)
                .await
                .map_err(at(PlateStep::Backdrop))?;
            Ok(DeclaredMade {
                shown,
                picture: Some(picture),
                attempt,
                makings,
                rejected: None,
            })
        }
        Verified::Rejected { found } => Ok(DeclaredMade {
            shown: Shown {
                summary: format!(
                    "plate {}: made twice with {found} people for a cast of {}, not shown",
                    attempts[0].name,
                    present.len()
                ),
                own_cast: false,
                captioned: None,
                on_screen: false,
                dropped: None,
                making: None,
            },
            picture: None,
            attempt: super::people_check::ATTEMPTS,
            makings,
            rejected: Some(found),
        }),
    }
}

/// The reference sheets (`ref-<persona id>.png`) the media library holds for `characters`, in
/// their order; a character without one (the library answers 404) is left out. A request that
/// fails, or any other answer, is the failure of the step.
pub(super) async fn reference_sheets(
    port: u16,
    characters: &[CharacterRequest],
) -> Result<Vec<String>, PlateFailure> {
    let mut sheets = Vec::new();
    for character in characters {
        let sheet = reference_sheet_name(&character.name);
        if library_holds(port, &sheet)
            .await
            .map_err(at(PlateStep::References))?
        {
            sheets.push(sheet);
        }
    }
    Ok(sheets)
}

/// Whether the media library holds the picture `name` (it answers 404 when not); any other answer
/// is an error.
pub(super) async fn library_holds(port: u16, name: &str) -> Result<bool, DevError> {
    let answer = crate::client::head(port, &df_playground::path::media_file(name))
        .await
        .map_err(DevError::Client)?;
    match answer.status {
        StatusCode::OK => Ok(true),
        StatusCode::NOT_FOUND => Ok(false),
        status => Err(DevError::UnexpectedAnswer {
            status,
            body: String::new(),
            reason: Some(format!("the media library's answer for {name}")),
        }),
    }
}

/// The first of `items` (most noticeable first, the order the picture shows them) that the
/// backend's room for pictures takes once `taken` of them are spoken for (the people's sheets
/// first, then the place's things) and the picture to edit, if any, has its place. What does not
/// fit is left out, never described: a picture that shows few things draws them well.
pub(crate) fn items_that_fit<T>(
    limits: &BackendLimits,
    taken: usize,
    edits_a_picture: bool,
    items: Vec<T>,
) -> Vec<T> {
    let room = limits
        .maximum_pictures
        .saturating_sub(usize::from(edits_a_picture))
        .saturating_sub(taken);
    items.into_iter().take(room).collect()
}

/// What came of the reference sheets of inventory items a picture shows: each item as a sheet (its
/// picture when the library holds one), and the sheets asked for because the library has none
/// yet, or why one could not be asked for.
pub(crate) struct InventorySheets {
    pub sheets: Vec<Thing>,
    pub requested: Vec<String>,
    pub not_requested: Vec<String>,
}

/// The reference sheets of `shown`, items by their owner, in their order (the order the picture's
/// room takes them in). An item whose sheet (`world::inventory::sheet_name`) the
/// library does not hold yet is described in words this time, and its sheet is asked of the image
/// service in the background, so a later picture has it; one already being made is not asked
/// again. A picture never waits for a sheet.
pub(crate) async fn inventory_sheets(
    port: u16,
    shown: &[(String, crate::world::InventoryItem)],
) -> Result<InventorySheets, PlateFailure> {
    let mut made = InventorySheets {
        sheets: Vec::new(),
        requested: Vec::new(),
        not_requested: Vec::new(),
    };
    for (owner, item) in shown {
        let name = crate::world::sheet_name(owner, &item.id);
        let held = library_holds(port, &name)
            .await
            .map_err(at(PlateStep::References))?;
        if !held {
            match request_sheet(port, &name, item).await {
                Ok(()) => made.requested.push(name.clone()),
                Err(error) => made.not_requested.push(format!("{name} ({error})")),
            }
        }
        made.sheets.push(Thing {
            id: item.id.clone(),
            name: item.name.clone(),
            look: item.look.clone(),
            picture: held.then_some(name),
        });
    }
    Ok(made)
}

/// Asks for the reference sheet of each item `applied` brought in from none (a gain from a count
/// of zero), so a new item has its sheet before a picture shows it; a sheet the library holds is
/// not asked for again. Answers the lines for a transcript, one for each sheet asked for and one
/// when the inventory or the image service cannot be reached.
pub(crate) async fn sheets_for_gains(
    port: u16,
    owner: &str,
    applied: &[crate::world::InventoryApplied],
) -> Vec<String> {
    use crate::world::inventory::ChangeKind;
    let gained: Vec<&str> = applied
        .iter()
        .filter(|taken| taken.kind == ChangeKind::Add && taken.before == 0)
        .map(|taken| taken.item.as_str())
        .collect();
    if gained.is_empty() {
        return Vec::new();
    }
    let address = format!(
        "{}?owner={owner}",
        crate::world::inventory::routes::path::INVENTORY_OWNER
    );
    let view: Result<crate::world::inventory::request::InventoryView, DevError> =
        match get_json::<crate::world::inventory::request::InventoryReply<_>>(port, &address).await
        {
            Ok(reply) => reply_value(reply),
            Err(error) => Err(error),
        };
    let shown: Vec<(String, crate::world::InventoryItem)> = match view {
        Ok(view) => view
            .inventory
            .items
            .into_iter()
            .filter(|item| gained.contains(&item.id.as_str()))
            .map(|item| (owner.to_owned(), item))
            .collect(),
        Err(error) => return vec![format!("sheets of the new items not read: {error}")],
    };
    match inventory_sheets(port, &shown).await {
        Ok(made) => made
            .requested
            .iter()
            .map(|name| format!("sheet {name}: none yet, asked for"))
            .chain(
                made.not_requested
                    .iter()
                    .map(|refused| format!("sheet {refused}: not asked for")),
            )
            .collect(),
        Err((_, error)) => vec![format!("sheets of the new items not asked for: {error}")],
    }
}

/// Asks the image service for the reference sheet `name` of `item`, without waiting: the thing
/// alone from two sides on a plain ground, in the house look.
async fn request_sheet(
    port: u16,
    name: &str,
    item: &crate::world::InventoryItem,
) -> Result<(), DevError> {
    let (prompt, avoid) = crate::imagegen::cli::with_look(
        format!(
            "a reference sheet of one thing, {}: {}. The thing alone, whole and sharp, seen from \
             the front and at three-quarters, side by side on a plain mid-grey ground.",
            item.name, item.look
        ),
        None,
    )
    .map_err(DevError::Command)?;
    let request = GenerateRequest {
        prompt,
        name: name.trim_end_matches(".png").to_owned(),
        purpose: Some(Purpose::Portrait),
        aspect: None,
        wait: false,
        base: None,
        references: Vec::new(),
        things: Vec::new(),
        avoid,
        left_out: Vec::new(),
        priority: Priority::Normal,
        sheet: None,
    };
    let reply: GenerateReply<crate::imagegen::request::JobView> =
        post_json(port, image_path::GENERATE, &request).await?;
    reply_value(reply).map(|_| ())
}

/// The media library's name of the reference sheet of the persona `id`.
pub(super) fn reference_sheet_name(id: &str) -> String {
    format!("ref-{id}.png")
}

/// Generates the plate `request` asks for, waiting for it, and gives the stored picture's file
/// name.
async fn generate(port: u16, request: GenerateRequest) -> Result<String, PlateFailure> {
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
