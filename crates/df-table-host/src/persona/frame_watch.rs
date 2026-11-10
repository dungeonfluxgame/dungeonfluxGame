//! Frames during `persona converse frames=NAME`: the picture follows the talk. After each line
//! the frame judge (`POST /dev/persona/frame`) reads what the picture shows and the lines said
//! since it was made; when the visible change passes the gate, a new frame is made in the
//! background (by default an edit of the scene's first picture, so faces and the room stay the
//! same) while the talk goes on, and put behind the scene when it is ready. Up to `parallel`
//! frames are made at once (one by default); they are shown in the order they were started, each
//! after the one before it, and the scene's spinner shows while any is being made. Lines said
//! while no frame may start wait for the next judgement. Every setting is a command option
//! over the defaults of `persona/frames.json`; the transcript says when each frame was judged,
//! started and shown. A new inspection (the judge names an item someone is looking at closely)
//! cuts to an insert of the item at once, inside the hold; the picture the next frame starts from
//! stays the room's, so the cut comes back to the scene. An insert is not scanned (nobody's face
//! is in it): it is shown with the room's layout, so the cast can go on speaking, and the room
//! comes back after the insert hold (`frames.json` `show`, or `insert_hold=`) unless a new frame
//! replaced it first. Which close look is new is the judge's to say (`new_look`), not code's.
//! Before an insert is made, the coverage judge is asked whether the keeper of the item's stall
//! already knows how it looks up close; only when not is the backfill engine asked for it. What
//! it adds stays in the keeper's backstory (which the frame judge reads) and goes into the
//! insert's picture. A thing in sight at the place has no keeper to ask: the judge's picture of
//! it, from its look, is the insert's.
//!
//! `persona watch` keeps the picture moving on a schedule too: every `every` (`every=`, else
//! `frames.json` `show.every_seconds`, 30 s), on slots counted from the watch's start so neither
//! the hold nor the time a frame takes moves them, it asks for a frame of the room as the talk has
//! left it whatever changed, on a timer beside the lines, so it fires while the talk is slow.
//! Frames are made `parallel` at once (all of `MAXIMUM_PARALLEL_FRAMES` by default in the watch)
//! and shown in the order they started; a slot that comes while all are busy is missed, and said
//! so. Without `max_frames=` the watch makes frames for as long as it runs. A watch input
//! `@focus <text>` (what a close look at a found clue shows, from the narrative engine) cuts to an
//! insert of it at once, in the inspection shot, with `<text>` on the scene's caption bar (the
//! readable bar at the foot of the screen, which the page keeps every bubble clear of) for the
//! insert hold; the caption that was there is put back with the room. The watch records when
//! each frame starts and is shown and when an insert ends (`PERSONA frame_timing`, see `frame`).
//!
//! A watch input `@show <picture> | <caption> | <seconds>` cuts to a picture the library already
//! holds (a finished painting) as it is: no picture is made and no model is asked. It is an insert
//! with that picture and the room's layout (the cast and the bubbles stay), the caption on the
//! scene's caption bar, held for the seconds (10 by default), then the room returns. It is shown
//! at once like an emergency plate, but it is no new picture of the talk, so the plates waiting
//! for their slots are not dropped; an unknown picture is refused with a typed reason and logged
//! (`PERSONA frame_timing .. reason=reveal`, `PERSONA revealed`, `PERSONA refused verb=reveal`).
//!
//! A watch input `@declare <speaker id> | <json>` (printed by `persona declare`) is a character
//! declaring the physical action they are doing right now: it makes an emergency plate of it, a
//! picture of the scene showing the action in progress. No judge decides whether to cut, and no
//! slot is waited for: the plate is started at once whatever is being made (one emergency plate
//! at a time; a newer declaration replaces an older one still on its way), asks the image
//! backend for its own queue's front (`Priority::Emergency`), and goes up the moment it is made,
//! held for `declare_hold` (`show.declare_hold_seconds`, 20 s) like an insert. Every room plate
//! started before the declaration that is still waiting or being made is superseded: made, never
//! shown, since the talk it shows is older than the action declared (`plate::begin_declaration`).
//! Plates started after it follow the normal rules.

// TODO(DEV-12): tests on a fake host: frames made in parallel are shown in the order they were
// started (a later one that finishes first waits for the one before it), the spinner shows once
// and the title is put back when the last frame lands, the change since a frame only grows, and
// the hold and the most frames are kept; a frame stays up the least time before the next made
// alongside it; an insert cuts in inside the hold, keeps the room's cast, comes back to the
// newest room after the insert hold (whole, at the end too) and carries the keeper's backfill. A
// focus's caption goes up with its insert and the one before comes back with the room; a
// scheduled frame starts on the tick with no line said. A declaration through `declare` on a fake
// host (an image backend that records its request): the request's priority, the speaker's sheet
// first and then the sheets of what is in use, the prompt without a name, the `PERSONA declared`
// line and the `ended` moment after the hold (the screen's rules are tested in `declare_tests`).

use std::collections::VecDeque;
use std::sync::PoisonError;
use std::time::{Duration, Instant};

use df_content::persona::backfill::Gap;
use df_content::persona::{PersonaEdit, PersonaField};
use df_playground::lobby::model::SceneTitle;
use df_types::saturating_millis;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use super::converse::{Speaker, speakers_of};
use super::declare::{
    DECLARE_PREFIX, NO_DUPLICATES, WatchDeclared, declared_scene, held_by, plan_base,
};
use super::frame::{
    DeclaredRejected, DeclaredTiming, DroppedWhy, FrameAsk, FrameConfig, FrameJudgement,
    FrameMoment, FrameReason, FrameReply, FrameRequest, FrameScheduleRequest, FrameTimingReply,
    FrameTimingRequest, PERSONA_FRAME, PERSONA_FRAME_SCHEDULE, PERSONA_FRAME_TIMING, RevealRefusal,
    RevealedTiming, ShowSettings,
};
use super::plate::{
    Attempt, Captioned, InsertRequest, Painting, PlateFailure, Reveal, RoomTurn, SharedScreen,
    Shown, begin_declaration, claim_after_insert, claim_after_reveal, figures_in, fit_pictures,
    image_limits, inventory_sheets, items_that_fit, library_holds, make_declared, make_frame,
    make_insert, make_reveal, plate_request, put_caption_back, put_title_back, queue_reveal,
    reference_sheet_name, show_with_cast, start_spinner,
};
use super::request::{BackfillRequest, Backfilled, CoverageRequest, Covered, PersonaReply};
use super::routes::path as persona_path;
use super::story::{PERSONA_STORY, StoryReply};
use super::story_items::{ItemPicture, PERSONA_STORY_ITEM_PICTURE, PictureReply, thing_of};
use crate::cast::request::CastList;
use crate::cast::routes::path as cast_path;
use crate::command::bad_choice;
use crate::dev::DevError;
use crate::dev_client::{get_json, post_json, reply_value};
use crate::imagegen::{BackendLimits, GenerateRequestError};
use crate::locate::request::CharacterRequest;
use crate::movement::brief::BriefStyle;
use crate::world::request::{PlaceSet, ThingPicture, WorldReply};
use crate::world::routes::path as world_path;
use crate::world::{InventoryItem, PlaceItem};
use df_content::world_bible::Thing;

/// How long a frame stays at least before the next one starts, by default.
pub(super) const DEFAULT_HOLD_SECONDS: u64 = 30;
/// How many frames one conversation makes at most, by default.
pub(super) const DEFAULT_MAXIMUM_FRAMES: u32 = 6;
/// How many frames may be made at once, at most, and how many `persona watch` makes at once by
/// default: a frame takes about a minute or more to make (50 to 200 s in run frames-t1, 130 to
/// 160 s in dungeon run 3), so a new plate every 30 s needs several being made at once (four
/// showed one every 68 s in dungeon run 3). A scheduled frame's judge counts as one on its way,
/// so twelve keep up with a judge's minute and frames of up to 240 s (dungeon run 9: eight missed
/// slots with 60 s judges and 150 to 220 s frames).
pub(super) const MAXIMUM_PARALLEL_FRAMES: usize = 12;
/// How many schedule judges may be out at once: two answers within a slot keep the cadence, and
/// every judge out slows every other model call (dungeon run 15: with up to a dozen out, the
/// story's calls went from 7 s to 50 s).
const MAXIMUM_SCHEDULE_JUDGES: usize = 2;
/// How often `persona watch` looks, between lines, for frames that finished, an insert whose hold
/// is over and a scheduled frame that is due: well inside what a person notices.
const WATCH_TICK: Duration = Duration::from_millis(250);
/// How a close-up is framed: the item alone, over the whole picture. Never a plain strip left for
/// the words shown over it: the painter draws that strip as a flat band across the picture file
/// (roam run 2: a beige band over the bottom quarter of every insert).
const INSERT_FRAMING: &str = concat!(
    "The item alone fills the whole picture edge to edge, sharp from top to bottom, with no ",
    "person, border, bar or plain empty strip in view."
);
/// What an insert must not show beyond the house list: the strip and the people the framing rules
/// out, so the negative prompt says what the text asks.
const INSERT_AVOID: [&str; 2] = ["a plain empty band along an edge", "a person's face"];

/// The watch input that shows a found clue up close: `@focus <what the close look shows>`.
const FOCUS_PREFIX: &str = "@focus ";

/// How much shorter than the room a room plate leaves it the frame judge's picture of the scene is
/// asked to be: a frame settings file's own words can be longer than the built-in ones the room is
/// measured with, and the judge counts characters as Rust does, Forge as JavaScript does.
const PICTURE_TARGET_MARGIN: usize = 200;
/// The shortest picture of the scene the judge is asked for: fewer words would leave people out,
/// and a plate whose other text leaves less is refused by the backend, which says so.
const PICTURE_TARGET_FLOOR: usize = 400;

/// The schedule the toolbar's plate timer is told: the watch stopped, or runs with its next
/// scheduled frame due at `due` (none while one may not start).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Schedule {
    Stopped,
    Running { due: Option<Instant> },
}

/// The frame options of one `persona converse`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameOptions {
    /// Frames are stored as `<name>-1`, `<name>-2`, ...; a name already in the library is kept,
    /// not made again, so a new run takes a new name.
    pub name: String,
    /// The visible change, in percent, above which a frame is made; the file's when not given.
    pub gate_percent: Option<u8>,
    /// What the picture shows as the talk begins.
    pub scene: Option<String>,
    /// Edit the picture (true) or paint each frame anew (false).
    pub edit: bool,
    /// Edit the newest frame instead of the scene's first picture.
    pub from_last: bool,
    /// The least time between the start of one frame and the next.
    pub hold: Duration,
    pub maximum_frames: u32,
    /// How many frames may be made at once, 1 to `MAXIMUM_PARALLEL_FRAMES`.
    pub parallel: usize,
    /// How long an insert of an item stays on screen before the room comes back, instead of the
    /// settings' `show.insert_hold_seconds`.
    pub insert_hold: Option<Duration>,
    /// The model the judge asks instead of the conversation's.
    pub judge_model: Option<String>,
    /// How long after the last frame started `persona watch` starts one on its schedule, instead
    /// of the settings' `show.every_seconds`; zero makes none.
    pub every: Option<Duration>,
    /// How long an emergency plate (a declared action) stays up before the room's plates go on,
    /// instead of the settings' `show.declare_hold_seconds`.
    pub declare_hold: Option<Duration>,
    /// The soft cap on the pictures a plate request the host builds itself carries, set on the
    /// host when the watch starts (`refs_max=`, as `media backend refs=` does).
    pub refs_max: Option<usize>,
    /// The story's place the talk is at, so every room frame draws the things in sight there as
    /// their reference sheets have them (`persona story item`), the same in every picture.
    pub place: Option<String>,
}

impl FrameOptions {
    pub(super) fn with_every(self, every: Duration) -> Self {
        Self {
            every: Some(every),
            ..self
        }
    }

    pub(super) fn with_refs_max(self, maximum: usize) -> Self {
        Self {
            refs_max: Some(maximum),
            ..self
        }
    }

    pub(super) fn with_declare_hold(self, hold: Duration) -> Self {
        Self {
            declare_hold: Some(hold),
            ..self
        }
    }
}

/// One line of `persona watch`'s standard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WatchInput<'a> {
    /// `<speaker id> | <line>`: a line said.
    Line { speaker: &'a str, line: &'a str },
    /// `@focus [<item> | ]<text>`: what a close look at a found clue shows, and the id of the
    /// story's reference sheet of it (`persona story item`) when it has one.
    Focus {
        item: Option<&'a str>,
        text: &'a str,
    },
    /// `@declare <speaker id> | <json>`: a character declaring what they are doing right now (the
    /// JSON is a [`WatchDeclared`]).
    Declare { speaker: &'a str, body: &'a str },
    /// `@show <picture> | <caption> | <seconds>`: a picture the library already holds, shown as it
    /// is for `hold` seconds (none given: `DEFAULT_REVEAL_HOLD_SECONDS`).
    Show {
        picture: &'a str,
        caption: &'a str,
        hold: Option<u64>,
    },
    /// None of these, or a focus with no text.
    Unreadable,
}

/// How long a revealed picture is held when `@show` gives no seconds.
pub(super) const DEFAULT_REVEAL_HOLD_SECONDS: u64 = 10;

/// The watch input that shows a picture the library already holds.
const SHOW_PREFIX: &str = "@show ";

impl<'a> WatchInput<'a> {
    pub(super) fn parse(entry: &'a str) -> Self {
        if let Some(shown) = entry.strip_prefix(SHOW_PREFIX) {
            // `<picture> | <caption> | <seconds>`: the seconds are the last field when it is a
            // whole number, and a caption may hold ` | ` itself.
            let Some((picture, rest)) = shown.split_once(" | ") else {
                return Self::Unreadable;
            };
            let (caption, hold) = match rest.rsplit_once(" | ") {
                Some((caption, seconds)) => match seconds.trim().parse() {
                    Ok(seconds) => (caption, Some(seconds)),
                    Err(_) => (rest, None),
                },
                None => (rest, None),
            };
            return match picture.trim() {
                "" => Self::Unreadable,
                picture => Self::Show {
                    picture,
                    caption: caption.trim(),
                    hold,
                },
            };
        }
        if let Some(declared) = entry.strip_prefix(DECLARE_PREFIX) {
            return match declared.split_once(" | ") {
                Some((speaker, body)) if !speaker.trim().is_empty() && !body.trim().is_empty() => {
                    Self::Declare {
                        speaker: speaker.trim(),
                        body: body.trim(),
                    }
                }
                _ => Self::Unreadable,
            };
        }
        if let Some(look) = entry.strip_prefix(FOCUS_PREFIX) {
            let (item, text) = match look.split_once(" | ") {
                Some((item, text)) => (Some(item.trim()), text.trim()),
                None => (None, look.trim()),
            };
            return match (item, text) {
                (_, "") | (Some(""), _) => Self::Unreadable,
                (item, text) => Self::Focus { item, text },
            };
        }
        match entry.split_once(" | ") {
            Some((speaker, line)) => Self::Line {
                speaker: speaker.trim(),
                line: line.trim(),
            },
            None => Self::Unreadable,
        }
    }
}

/// The schedule's slots: slot `n` is due `n` times `every` after the watch `began` (slot 0 at
/// once), whatever was said or made since, so the time a frame takes to make never makes the
/// cadence drift and the hold never puts a slot off. How many slots from `next` on are due at
/// `now`; none when `every` is zero.
pub(super) fn slots_due(began: Instant, every: Duration, next: u32, now: Instant) -> u32 {
    if every.is_zero() {
        return 0;
    }
    let last_due = now.saturating_duration_since(began).as_millis() / every.as_millis();
    u32::try_from(last_due)
        .unwrap_or(u32::MAX)
        .saturating_add(1)
        .saturating_sub(next)
}

/// When slot `slot` of the schedule is due.
pub(super) fn slot_due(began: Instant, every: Duration, slot: u32) -> Instant {
    began + every.saturating_mul(slot)
}

/// The frame judge's answer to `request`, or its typed refusal.
pub(super) async fn ask_judge(
    port: u16,
    request: &FrameRequest,
) -> Result<FrameJudgement, DevError> {
    let reply: FrameReply = post_json(port, PERSONA_FRAME, request).await?;
    reply_value(reply)
}

/// `persona watch`: frames for a talk spoken elsewhere, one `persona say` or `persona ask` per
/// line (as `development/market-thread.sh` does). Each line of standard input is
/// `<speaker id> | <line>`, judged and framed as in `persona converse frames=`, or
/// `@focus <text>`, an insert of a found clue captioned with `<text>`. Beside the input a tick
/// (owned here, so it stops with the watch) reports the frames that finished, gives the screen back
/// after an insert's hold and starts the scheduled frames. At the end of input the frames still
/// being made are waited for, and the transcript is given.
pub(super) async fn watch_lines(port: u16, options: FrameOptions) -> Result<String, DevError> {
    let listed: crate::cast::request::CastReply<CastList> = get_json(port, cast_path::CAST).await?;
    let cast = reply_value(listed)?;
    let picture = cast.picture.clone();
    let speakers = speakers_of(cast);
    let refs_max = options.refs_max;
    let mut watch = FrameWatch::new(options, &speakers, &picture);
    let mut transcript = Vec::new();
    if let Some(maximum) = refs_max {
        match set_reference_cap(port, maximum).await {
            Ok(()) => transcript.push(format!("  at most {maximum} pictures to a plate request")),
            Err(error) => transcript.push(format!("  reference cap not set: {error}")),
        }
    }
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    let mut tick = tokio::time::interval(WATCH_TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut turn = 0;
    loop {
        // Reading a line is cancel safe: a tick that wins loses no input.
        let entry = tokio::select! {
            entry = input.next_line() => entry.map_err(|error| DevError::UnreadableFile {
                path: "<stdin>".into(),
                message: error.to_string(),
            })?,
            _ = tick.tick() => {
                watch.on_tick(port, turn, &mut transcript).await;
                continue;
            }
        };
        let Some(entry) = entry else {
            break;
        };
        match WatchInput::parse(&entry) {
            WatchInput::Unreadable => transcript.push(format!(
                "  not '<speaker id> | <line>', '@focus <text>', '@show <picture> | <caption> | \
                 <seconds>' or '@declare <speaker id> | <json>', skipped: {entry}"
            )),
            WatchInput::Show {
                picture,
                caption,
                hold,
            } => {
                let hold = Duration::from_secs(hold.unwrap_or(DEFAULT_REVEAL_HOLD_SECONDS));
                transcript.push(format!("  show: {picture}"));
                watch
                    .reveal(port, turn, (picture, caption, hold), &mut transcript)
                    .await;
            }
            WatchInput::Declare { speaker: id, body } => {
                let declared = serde_json::from_str::<WatchDeclared>(body);
                let Some(speaker) = speakers.iter().find(|speaker| speaker.id == id) else {
                    transcript.push(format!("  {id} is not in the scene, skipped"));
                    continue;
                };
                match declared {
                    Ok(declared) => {
                        turn += 1;
                        transcript.push(format!(
                            "[{turn:02}] {} (declaring): {}",
                            speaker.display_name, declared.line
                        ));
                        watch
                            .declare(port, turn, (speaker, declared), &mut transcript)
                            .await;
                    }
                    Err(error) => {
                        transcript.push(format!("  not a declaration from {id}, skipped: {error}"))
                    }
                }
            }
            WatchInput::Focus { item, text } => {
                transcript.push(format!("  focus: {text}"));
                watch.focus(port, turn, (item, text), &mut transcript).await;
            }
            WatchInput::Line { speaker: id, line } => {
                let Some(speaker) = speakers.iter().find(|speaker| speaker.id == id) else {
                    transcript.push(format!("  {id} is not in the scene, skipped"));
                    continue;
                };
                turn += 1;
                transcript.push(format!("[{turn:02}] {}: {line}", speaker.display_name));
                if let Some(request) = watch
                    .line_request(port, speaker, line, &mut transcript)
                    .await
                {
                    watch.ask_later(port, turn, request, None);
                }
            }
        }
    }
    watch.leave(port, &mut transcript).await;
    Ok(transcript.join("\n"))
}

/// Records a moment of the frame `frame` on the host (`POST /dev/persona/frame/timing`); the
/// `shown` moment of a declared plate also says who declared and how its time went.
async fn mark(
    port: u16,
    frame: &str,
    (moment, reason): (FrameMoment, FrameReason),
    took: Duration,
    declared: Option<DeclaredTiming>,
) -> Result<(), DevError> {
    let request = FrameTimingRequest {
        frame: frame.to_owned(),
        moment,
        reason,
        took_ms: saturating_millis(took),
        declared,
        revealed: None,
        dropped: None,
    };
    let reply: FrameTimingReply = post_json(port, PERSONA_FRAME_TIMING, &request).await?;
    reply_value(reply).map(|_| ())
}

/// Records a moment of a revealed picture `frame` (see [`mark`]), with the picture and its hold.
async fn mark_revealed(
    port: u16,
    frame: &str,
    (moment, took): (FrameMoment, Duration),
    revealed: RevealedTiming,
) -> Result<(), DevError> {
    let request = FrameTimingRequest {
        frame: frame.to_owned(),
        moment,
        reason: FrameReason::Reveal,
        took_ms: saturating_millis(took),
        declared: None,
        revealed: Some(revealed),
        dropped: None,
    };
    let reply: FrameTimingReply = post_json(port, PERSONA_FRAME_TIMING, &request).await?;
    reply_value(reply).map(|_| ())
}

/// Records that the frame `frame`, started, finished without going up and without failing (the
/// `dropped` moment): every frame that starts ends with exactly one of `shown`, `failed` and
/// `dropped`, so the page's timer never counts a frame that is over.
async fn mark_dropped(
    port: u16,
    frame: &str,
    reason: FrameReason,
    took: Duration,
    why: DroppedWhy,
) -> Result<(), DevError> {
    let request = FrameTimingRequest {
        frame: frame.to_owned(),
        moment: FrameMoment::Dropped,
        reason,
        took_ms: saturating_millis(took),
        declared: None,
        revealed: None,
        dropped: Some(why),
    };
    let reply: FrameTimingReply = post_json(port, PERSONA_FRAME_TIMING, &request).await?;
    reply_value(reply).map(|_| ())
}

/// Sets the soft cap on the pictures a plate request the host builds itself carries.
async fn set_reference_cap(port: u16, maximum: usize) -> Result<(), DevError> {
    use crate::imagegen::request::{GenerateReply, ReferenceCap};
    let Some(maximum) = std::num::NonZeroUsize::new(maximum) else {
        return Ok(());
    };
    let reply: GenerateReply<crate::imagegen::request::BackendView> = post_json(
        port,
        crate::imagegen::routes::path::REFERENCES,
        &ReferenceCap { maximum },
    )
    .await?;
    reply_value(reply).map(|_| ())
}

/// Why `picture` cannot be shown as it is: it is no picture file name, or the media library holds
/// none of that name. `None`: it can be shown.
async fn reveal_refusal(port: u16, picture: &str) -> Result<Option<RevealRefusal>, DevError> {
    if crate::locate::check_picture_name(picture).is_err() {
        return Ok(Some(RevealRefusal::InvalidName));
    }
    let held = library_holds(port, picture).await?;
    Ok((!held).then_some(RevealRefusal::UnknownPicture))
}

/// What the keeper of an item's stall knows of how it looks up close, before an insert of it.
enum KeeperDetails {
    /// Their persona already holds it (the coverage judge said so); the frame judge, which reads
    /// their account, has it already.
    Known,
    /// The backfill engine added it: the text it wrote.
    Added(Vec<String>),
}

/// How `keeper` knows `offered`, an item of their stall, up close: the coverage judge is asked
/// first, and
/// only when their persona does not hold it does the backfill engine add it, to their public
/// backstory, the memory of their own work (expertise stays authored and is never backfilled, as
/// `planning/persona-voice-and-world-bible.md` has it), so later lines and inserts agree and a
/// second look at the same item adds nothing.
async fn keeper_details(
    port: u16,
    keeper: &str,
    offered: &PlaceItem,
    show: &ShowSettings,
) -> Result<KeeperDetails, DevError> {
    // What is asked is data (`frames.json` `show.detail_topic`); the item is named after it.
    let look = format!("{} The item: {}.", show.detail_topic, offered.name);
    let coverage = CoverageRequest {
        character: keeper.to_owned(),
        question: look.clone(),
        guidance: None,
        model: None,
    };
    let covered: PersonaReply<Covered> =
        post_json(port, persona_path::PERSONA_COVERAGE, &coverage).await?;
    if reply_value(covered)?.enough {
        return Ok(KeeperDetails::Known);
    }
    let request = BackfillRequest {
        character: keeper.to_owned(),
        gaps: vec![Gap {
            field: PersonaField::Backstory,
            topic: look,
            filled: None,
        }],
        guidance: None,
        model: None,
    };
    let reply: PersonaReply<Backfilled> =
        post_json(port, persona_path::PERSONA_BACKFILL, &request).await?;
    Ok(KeeperDetails::Added(
        reply_value(reply)?
            .edits
            .into_iter()
            // A backfill only adds: its edits are added entries or appended text.
            .filter_map(|edit| match edit {
                PersonaEdit::AddEntry { text, .. } | PersonaEdit::AppendText { text, .. } => {
                    Some(text)
                }
                _ => None,
            })
            .collect(),
    ))
}

/// `persona show-picture`: shows `picture`, one the library already holds, for `hold` as `@show`
/// does in a watch (an insert with `room`'s layout, `caption` on the caption bar, nothing made),
/// then puts `room` and the caption that was there back. For a call without a watch: it waits out
/// the hold. An unknown picture is refused and logged.
pub(super) async fn show_picture(
    port: u16,
    picture: &str,
    room: &str,
    caption: Option<&str>,
    hold: Duration,
) -> Result<String, DevError> {
    // The frame's name is a plain file name whatever the picture's is.
    let stem: String = picture
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect();
    let name = format!("show-{stem}");
    let revealed = |refused| RevealedTiming {
        picture: picture.to_owned(),
        hold_ms: saturating_millis(hold),
        refused,
        behind: 0,
    };
    if let Some(refusal) = reveal_refusal(port, picture).await? {
        mark_revealed(
            port,
            &name,
            (FrameMoment::Failed, Duration::ZERO),
            revealed(Some(refusal)),
        )
        .await?;
        return Err(DevError::Command(bad_choice(
            "picture",
            picture,
            format!("a picture of the media library ({})", refusal.key()),
        )));
    }
    mark_revealed(
        port,
        &name,
        (FrameMoment::Started, Duration::ZERO),
        revealed(None),
    )
    .await?;
    let started = Instant::now();
    let reveal = Reveal {
        name: &name,
        picture,
        room,
        caption,
        hold,
    };
    let screen = SharedScreen::default();
    let ticket = queue_reveal(&screen);
    let (summary, captioned) = make_reveal(port, &screen, reveal, ticket)
        .await
        .map_err(|(_, error)| *error)?;
    mark_revealed(
        port,
        &name,
        (FrameMoment::Shown, started.elapsed()),
        revealed(None),
    )
    .await?;
    tokio::time::sleep(hold).await;
    if let Some(captioned) = captioned {
        put_caption_back(port, captioned.before).await?;
    }
    show_with_cast(port, room, room)
        .await
        .map_err(|(_, error)| *error)?;
    mark_revealed(
        port,
        &name,
        (FrameMoment::Ended, started.elapsed()),
        revealed(None),
    )
    .await?;
    Ok(format!(
        "{summary}; the room is back after {} s",
        hold.as_secs()
    ))
}

/// `persona frame`: judges `lines` against the picture `picture` describes, for the characters
/// of the shown scene, and says what changed and whether it passes the gate.
pub(super) async fn judge_frame(
    port: u16,
    picture: String,
    lines: Vec<String>,
    gate_percent: Option<u8>,
    model: Option<String>,
) -> Result<String, DevError> {
    let listed: crate::cast::request::CastReply<CastList> = get_json(port, cast_path::CAST).await?;
    let cast = reply_value(listed)?;
    let speakers = speakers_of(cast);
    let request = FrameRequest {
        picture,
        lines,
        speakers: speakers.clone(),
        gate_percent,
        model,
        last_shot: None,
        peak_percent: 0,
        last_inspected: None,
        ask: FrameAsk::Line {},
        picture_target_characters: None,
    };
    let judged = ask_judge(port, &request).await?;
    let mut text = vec![format!(
        "{}% changed (gate {}): {}",
        judged.changed_percent,
        judged.gate_percent,
        if judged.triggered {
            "a new frame"
        } else {
            "the picture stays"
        }
    )];
    for change in &judged.changes {
        text.push(format!(
            "  {} {}: {}",
            shown_name(&speakers, change.character.as_deref()),
            change.kind,
            change.now
        ));
    }
    if let Some(offered) = &judged.inspecting {
        text.push(format!("  inspecting: {} ({})", offered.name, offered.id));
    }
    text.push(format!("  shot: {} ({})", judged.shot, judged.shot_look));
    text.push(format!("  next picture: {}", judged.next_picture));
    Ok(text.join("\n"))
}

/// The name a change is about: the character's display name, or the room.
fn shown_name<'a>(speakers: &'a [Speaker], character: Option<&str>) -> &'a str {
    character
        .and_then(|id| speakers.iter().find(|speaker| speaker.id == id))
        .map_or("the room", |speaker| speaker.display_name.as_str())
}

/// A frame being made: its number, why it is made (an insert, a cut to one thing, or a frame of
/// the room), when it started and after which line, and the task.
struct InFlight {
    number: u32,
    name: String,
    reason: FrameReason,
    started: Instant,
    after_turn: u32,
    /// How long it holds the screen when that is not the insert hold (a revealed picture).
    hold: Option<Duration>,
    task: JoinHandle<Made>,
}

/// What making a frame came to, when it finished (for a frame made, when it went on screen), and
/// whether its `shown` moment was recorded.
struct Made {
    made: Result<Shown, PlateFailure>,
    finished: Instant,
    timing: Result<(), DevError>,
    /// The picture that went up, when it is not the frame's own name (a declared plate made again).
    picture: Option<String>,
}

/// A frame judge asked and not answered yet (on the schedule, after a line, or for a focus), owned
/// by the watch so it stops with it. The watch never waits on one: it is asked beside the frames
/// being made and the lines read (dungeon run 8: awaited in the tick, a judge's minute let a frame
/// start only once a minute; dungeon run 13: awaited after each line, five judges in a row held
/// the ticks for 340 s, then twelve frames started at once and the screen went unchanged 307 s).
// TODO(DEV-12): a test that a slow judge holds back no tick, start or later line.
struct Judging {
    /// The last line said when it was asked, and how many lines since the newest frame it was
    /// shown: those said after are kept for the next judgement.
    turn: u32,
    lines_judged: usize,
    asked: Instant,
    /// What it was asked for: a line, the schedule or a focus.
    reason: FrameReason,
    /// The clue a focus judge was asked for, put back for its frame when it answers.
    focus: Option<FocusOn>,
    task: JoinHandle<Result<FrameJudgement, DevError>>,
}

/// The insert on screen: its frame name, why it was made, and when it went up.
struct InsertUp {
    name: String,
    reason: FrameReason,
    shown: Instant,
    /// How long it holds the screen when that is not the insert hold (a revealed picture).
    hold: Option<Duration>,
}

/// The state of the frames of one conversation.
pub(super) struct FrameWatch {
    options: FrameOptions,
    speakers: Vec<Speaker>,
    characters: Vec<CharacterRequest>,
    /// The scene's picture as the talk began, and the newest frame shown.
    first_picture: String,
    newest_picture: String,
    /// The frame number of the newest picture (0: the scene's own).
    newest_number: u32,
    /// The newest picture whose own scan names the whole cast: a frame or insert that leaves
    /// someone out is shown with its cast, so nobody leaves the scene because of the camera.
    cast_picture: String,
    /// What the newest frame shows, in words.
    picture: String,
    /// The lines said since the newest frame was judged, as `Name: line`.
    since: Vec<String>,
    made: u32,
    /// The camera shot of the newest frame, so the judge can vary the next one.
    last_shot: Option<String>,
    /// The most change any judgement saw since the newest frame: the change since a picture only
    /// grows as more is said, so a quiet line never lowers it.
    peak_percent: u8,
    /// The frames being made, oldest first.
    in_flight: VecDeque<InFlight>,
    /// When the last picture went up and until when an insert holds the screen, shared with the
    /// frames being made.
    screen: SharedScreen,
    /// The scene's title before the spinner, while frames are being made.
    title_before: Option<Option<SceneTitle>>,
    last_start: Option<Instant>,
    /// The item last said to be looked at closely, so the same look makes no second insert until
    /// another item is looked at.
    last_inspected: Option<String>,
    /// The insert on screen.
    insert_up: Option<InsertUp>,
    /// The emergency plate on screen, until its hold is over.
    declared_up: Option<InsertUp>,
    /// How many pictures the library already held have been shown (`reveal`), for their names.
    reveals: u32,
    /// The caption an insert's clue replaced on the caption bar, while the clue is up.
    captioned: Option<Captioned>,
    /// When the watch began, and when it last asked for a frame on the schedule.
    began: Instant,
    /// The schedule's next slot, and how many slots came while `parallel` room frames were being
    /// made (or the most were made), so none could start.
    next_slot: u32,
    missed: u32,
    /// The schedule last told the host (its interval, when the next slot is due and the slots
    /// missed), so it is told only a change.
    told_schedule: Option<(Duration, Option<Instant>, u32)>,
    /// How frames are shown, from the newest judgement (`frames.json` `show`).
    show: ShowSettings,
    /// The focus asked for and not yet started: its caption and the thing's reference sheet.
    focus: Option<FocusOn>,
    /// How long a room frame takes to make, as the frames made so far averaged it: frames are
    /// started ahead of the schedule so one is ready at each slot (`needs_lead`).
    room_making: Option<Duration>,
    /// The judges asked on the schedule and not answered yet, oldest first, and how long one takes
    /// to answer, averaged as `room_making` is: a scheduled frame takes both.
    judging: VecDeque<Judging>,
    judge_taking: Option<Duration>,
    /// When a room frame was last started on a line's cue: a schedule judge asked before it
    /// judged a picture that frame has replaced (an insert replaces no room picture).
    last_cued: Option<Instant>,
}

/// Whether a room frame should start now, ahead of its slot, so one is ready at each showing slot
/// however long they take to make (dungeon run 5: one start a slot left 53 s gaps when frames took
/// 85 to 150 s): fewer are being made than making one spans slots, plus two, and the last start
/// was half a slot ago or more. Never without a schedule or before one has been made.
pub(super) fn needs_lead(
    in_flight: usize,
    making: Option<Duration>,
    every: Duration,
    since_start: Duration,
) -> bool {
    let Some(making) = making else {
        return false;
    };
    if every.is_zero() {
        return false;
    }
    let spans = making.as_millis().div_ceil(every.as_millis());
    // Two more than making spans, for the spread of making times (dungeon run 6: 70 to 130 s
    // left an 81 s gap with one more).
    let wanted = usize::try_from(spans)
        .unwrap_or(usize::MAX)
        .saturating_add(2);
    in_flight < wanted && since_start >= every / 2
}

/// A focus on a found clue: the words it is captioned with, and the story's reference sheet of
/// the thing when it has one.
#[derive(Debug, Clone)]
struct FocusOn {
    caption: String,
    sheet: Option<Thing>,
}

/// Where the reference sheet of a thing shown up close keeps its first close-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SheetHome {
    /// The stored story (a found clue's thing).
    Story,
    /// The active place, designed in play (a thing in sight there).
    Place,
}

/// The story's reference sheet `item`, from the stored story; `None` when it keeps none.
async fn sheet_of(port: u16, item: &str) -> Result<Option<Thing>, DevError> {
    let reply: StoryReply = get_json(port, PERSONA_STORY).await?;
    Ok(reply_value(reply)?
        .items
        .into_iter()
        .find(|sheet| sheet.id == item)
        .map(thing_of))
}

/// The story's reference sheets of the things at `place`.
async fn sheets_at(port: u16, place: &str) -> Result<Vec<Thing>, DevError> {
    let reply: StoryReply = get_json(port, PERSONA_STORY).await?;
    Ok(reply_value(reply)?
        .items
        .into_iter()
        .filter(|sheet| sheet.place == place)
        .map(thing_of)
        .collect())
}

/// The reference sheet of `offered` when it is a thing in sight (it has a look): a thing a stall
/// sells is drawn from its keeper's account instead.
fn place_sheet(offered: &PlaceItem) -> Option<Thing> {
    Some(Thing {
        id: offered.id.clone(),
        name: offered.name.clone(),
        look: offered.look.clone()?,
        picture: offered.picture.clone(),
    })
}

/// The prompt of a room frame painted anew: the house look, the judge's picture of the scene, its
/// shot and how writing shows.
pub(super) fn painted_prompt(style: &BriefStyle, next_picture: &str, shot_look: &str) -> String {
    format!(
        "{} {next_picture} Frame it as {shot_look}. {}",
        style.direction, style.writing
    )
}

/// The prompt of a room frame: an edit of the picture before it, by `edit_rule` (the frame
/// settings' rule an edited frame follows), when one is given; otherwise painted anew.
pub(super) fn room_prompt(
    style: &BriefStyle,
    edit_rule: Option<&str>,
    shot_look: &str,
    next_picture: &str,
) -> String {
    match edit_rule {
        Some(edit_rule) => format!(
            "{edit_rule} The look stays {} this scene. Frame it as {shot_look}. The scene now: \
             {next_picture} {}",
            style.direction.to_lowercase(),
            style.writing
        ),
        None => painted_prompt(style, next_picture, shot_look),
    }
}

/// The length the frame judge's picture of the scene keeps to, when the image backend takes at
/// most `maximum_characters` of text and everything else a room plate sends is
/// `fixed_characters`: what is left, less `PICTURE_TARGET_MARGIN`, and never under
/// `PICTURE_TARGET_FLOOR`.
pub(super) fn picture_target(maximum_characters: usize, fixed_characters: usize) -> usize {
    maximum_characters
        .saturating_sub(fixed_characters)
        .saturating_sub(PICTURE_TARGET_MARGIN)
        .max(PICTURE_TARGET_FLOOR)
}

/// Keeps `picture`, a close-up just drawn, on the reference sheet `item` where it is kept.
async fn keep_sheet_picture(
    port: u16,
    home: SheetHome,
    item: &str,
    picture: &str,
) -> Result<(), DevError> {
    match home {
        SheetHome::Story => {
            let request = ItemPicture {
                item: item.to_owned(),
                picture: picture.to_owned(),
            };
            let reply: PictureReply = post_json(port, PERSONA_STORY_ITEM_PICTURE, &request).await?;
            reply_value(reply).map(|_| ())
        }
        SheetHome::Place => {
            let request = ThingPicture {
                item: item.to_owned(),
                picture: picture.to_owned(),
            };
            let reply: WorldReply<PlaceSet> =
                post_json(port, world_path::WORLD_PICTURE, &request).await?;
            reply_value(reply).map(|_| ())
        }
    }
}

/// A conversation that ends early (a turn failed) drops its watch: the frames still being made
/// are stopped, so no picture lands after the talk and nothing runs unowned.
impl Drop for FrameWatch {
    fn drop(&mut self) {
        for frame in &self.in_flight {
            frame.task.abort();
        }
        for judging in &self.judging {
            judging.task.abort();
        }
    }
}

impl FrameWatch {
    /// A watch over the scene whose backdrop is `picture`, with these speakers.
    pub(super) fn new(options: FrameOptions, speakers: &[Speaker], picture: &str) -> Self {
        let people: Vec<String> = speakers
            .iter()
            .map(|speaker| format!("a person ({})", speaker.job))
            .collect();
        let described = options.scene.clone().unwrap_or_else(|| {
            format!("The scene as the talk begins, with {}.", people.join("; "))
        });
        let characters = speakers
            .iter()
            .map(|speaker| CharacterRequest {
                name: speaker.id.clone(),
                display_name: Some(speaker.display_name.clone()),
                description: Some(speaker.job.clone()),
                portrait: None,
            })
            .collect();
        Self {
            options,
            speakers: speakers.to_vec(),
            characters,
            first_picture: picture.to_owned(),
            newest_picture: picture.to_owned(),
            newest_number: 0,
            cast_picture: picture.to_owned(),
            picture: described,
            since: Vec::new(),
            made: 0,
            last_shot: None,
            peak_percent: 0,
            in_flight: VecDeque::new(),
            screen: SharedScreen::default(),
            title_before: None,
            last_start: None,
            last_inspected: None,
            insert_up: None,
            declared_up: None,
            reveals: 0,
            captioned: None,
            began: Instant::now(),
            next_slot: 0,
            missed: 0,
            told_schedule: None,
            show: ShowSettings::default(),
            focus: None,
            room_making: None,
            judging: VecDeque::new(),
            judge_taking: None,
            last_cued: None,
        }
    }

    /// Takes something that happens at the table into the lines since the newest frame: unlike
    /// talk, it is seen.
    pub(super) fn note_event(&mut self, what: &str) {
        self.since.push(format!("(Happening now: {what})"));
    }

    /// Takes `line` said by `speaker` into the lines since the newest frame, collects the frames
    /// that finished, and gives the frame judge's request when a new one may start: the most
    /// frames are not made yet, and fewer than `parallel` room frames or fewer than `parallel`
    /// inserts are being made (inserts have places of their own, so a close look is not lost to
    /// slow room frames: run 60 lost two). It is asked inside the hold too, since an inspection
    /// cuts in at once. While a line's judge is out no other is asked: the lines said meanwhile
    /// wait for the next one, so a quick exchange asks no judge a line.
    pub(super) async fn line_request(
        &mut self,
        port: u16,
        speaker: &Speaker,
        line: &str,
        transcript: &mut Vec<String>,
    ) -> Option<FrameRequest> {
        self.since
            .push(format!("{}: {}", speaker.display_name, line));
        self.collect(port, false, transcript).await;
        self.end_insert(port, false, transcript).await;
        self.end_declared(port, false, transcript).await;
        if !self.line_judge_wanted() {
            return None;
        }
        Some(self.request(port, FrameAsk::Line {}, transcript).await)
    }

    /// Whether a line's judge may be asked: the most frames are not made yet, a room or an insert
    /// may still start, and no other line's judge is out.
    fn line_judge_wanted(&self) -> bool {
        self.made < self.options.maximum_frames
            && (self.rooms_in_flight() < self.options.parallel
                || self.inserts_in_flight() < self.options.parallel)
            && !self
                .judging
                .iter()
                .any(|judging| judging.reason == FrameReason::Line)
    }

    /// Cuts to an insert of what a close look at a found clue shows (`text`, said after line
    /// `turn`), captioned with it: the narrative engine's cue, so it starts whatever is being made.
    /// With `item`, the story's reference sheet of the thing, the close-up is drawn from the
    /// sheet's look, or is the sheet's own picture once it has one, so the thing looks the same
    /// each time; a sheet that cannot be read leaves the close-up to the clue's words.
    pub(super) async fn focus(
        &mut self,
        port: u16,
        turn: u32,
        (item, text): (Option<&str>, &str),
        transcript: &mut Vec<String>,
    ) {
        self.collect(port, false, transcript).await;
        self.end_insert(port, false, transcript).await;
        self.end_declared(port, false, transcript).await;
        let sheet = match item {
            Some(item) => match sheet_of(port, item).await {
                Ok(Some(sheet)) => Some(sheet),
                Ok(None) => {
                    transcript.push(format!("  the story keeps no sheet {item}"));
                    None
                }
                Err(error) => {
                    transcript.push(format!("  the sheet of {item} is not read: {error}"));
                    None
                }
            },
            None => None,
        };
        let look = sheet
            .as_ref()
            .map_or_else(|| text.to_owned(), |sheet| sheet.look.clone());
        let focus = FocusOn {
            caption: text.to_owned(),
            sheet,
        };
        let request = self
            .request(port, FrameAsk::Focus { look }, transcript)
            .await;
        self.ask_later(port, turn, request, Some(focus));
    }

    /// A character declares the action they are doing right now (`@declare`, after line `turn`):
    /// an emergency plate of the scene showing it. No judge is asked and no slot is waited for, and
    /// no cap on the plates being made holds it back (the most frames made still does: that cap is
    /// the spend). An emergency plate still on its way is replaced by this one, every room plate
    /// started before it is superseded (`begin_declaration`), and the plate is made after the
    /// speaker's reference sheet first and the sheets of what their hands use next, within the
    /// image backend's room for pictures, as an edit of the picture on screen (painted anew when
    /// there is none). The picture's words are the speaker's look and the action, never a name.
    pub(super) async fn declare(
        &mut self,
        port: u16,
        turn: u32,
        (speaker, declared): (&Speaker, WatchDeclared),
        transcript: &mut Vec<String>,
    ) {
        self.collect(port, false, transcript).await;
        self.end_insert(port, false, transcript).await;
        self.end_declared(port, false, transcript).await;
        if self.made >= self.options.maximum_frames {
            transcript.push(format!(
                "  declaration after line {turn} not drawn: the most frames ({}) are made",
                self.made
            ));
            return;
        }
        let held = match held_by(port, &speaker.id).await {
            Ok(held) => held,
            Err(error) => {
                transcript.push(format!(
                    "  the inventory of {} is not read, so nothing they hold is drawn: {error}",
                    speaker.id
                ));
                Vec::new()
            }
        };
        let in_use: Vec<(String, InventoryItem)> = declared
            .action
            .items_in_use
            .iter()
            .filter_map(|wanted| {
                held.iter()
                    .find(|item| item.id == *wanted && item.count > 0)
                    .map(|item| (speaker.id.clone(), item.clone()))
            })
            .collect();
        let names: Vec<String> = in_use.iter().map(|(_, item)| item.name.clone()).collect();
        let scene = declared_scene(
            &speaker.job,
            &declared.action,
            &names,
            self.characters.len(),
        );
        let request = FrameRequest {
            picture: self.picture.clone(),
            lines: Vec::new(),
            speakers: self.speakers.clone(),
            gate_percent: self.options.gate_percent,
            model: None,
            last_shot: self.last_shot.clone(),
            peak_percent: 0,
            last_inspected: None,
            ask: FrameAsk::Declare {
                scene: scene.clone(),
            },
            picture_target_characters: None,
        };
        let judgement = match ask_judge(port, &request).await {
            Ok(judgement) => judgement,
            Err(error) => {
                transcript.push(format!(
                    "  declaration after line {turn} not drawn, the frame settings are not read: \
                     {error}"
                ));
                return;
            }
        };
        self.show = judgement.show.clone();
        {
            let mut screen = self.screen.lock().unwrap_or_else(PoisonError::into_inner);
            screen.minimum_shown = Duration::from_secs(self.show.minimum_shown_seconds);
            screen.room_every = self.every();
        }
        // One emergency plate at a time: a newer declaration replaces an older one on its way.
        for older in self
            .in_flight
            .iter()
            .filter(|frame| frame.reason == FrameReason::Declared)
        {
            older.task.abort();
            transcript.push(format!(
                "  declared plate {} replaced by a newer declaration",
                older.name
            ));
            let why = DroppedWhy::Replaced;
            if let Err(error) = mark_dropped(
                port,
                &older.name,
                older.reason,
                older.started.elapsed(),
                why,
            )
            .await
            {
                transcript.push(format!("  end of {} not recorded: {error}", older.name));
            }
        }
        self.in_flight
            .retain(|frame| frame.reason != FrameReason::Declared);
        self.made += 1;
        let number = self.made;
        let name = format!("{}-{number}", self.options.name);
        let waiting = begin_declaration(&self.screen, number, &name);
        let superseded = self.rooms_in_flight();
        // An edit keeps what the picture on screen shows: one that holds more figures than the
        // scene's cast would carry the extra ones into the action, so it is painted anew from the
        // place's words instead.
        let on_screen = (!self.newest_picture.is_empty()).then(|| self.newest_picture.clone());
        let figures = match &on_screen {
            Some(picture) => figures_in(port, picture).await,
            None => None,
        };
        let plan = plan_base(on_screen.is_some(), figures, self.characters.len());
        let base = on_screen.filter(|_| plan.edit);
        let fresh_words = {
            let place = self.options.scene.as_deref().unwrap_or(&self.picture);
            format!("{place} {}", judgement.next_picture)
        };
        let scene_words = if plan.edit {
            judgement.next_picture.clone()
        } else {
            fresh_words.clone()
        };
        let style = BriefStyle::default();
        let prompt = room_prompt(
            &style,
            base.as_ref().map(|_| judgement.edit_rule.as_str()),
            &judgement.shot_look,
            &scene_words,
        );
        // A picture that shows more people than the cast is made once more, painted anew.
        let fresh_prompt = room_prompt(&style, None, &judgement.shot_look, &fresh_words);
        // The speaker's sheet comes first, so the backend's room for pictures never loses it.
        let mut characters = self.characters.clone();
        if let Some(at) = characters
            .iter()
            .position(|character| character.name == speaker.id)
        {
            let own = characters.remove(at);
            characters.insert(0, own);
        }
        let shown = match image_limits(port).await {
            Ok(limits) => items_that_fit(&limits, characters.len(), base.is_some(), in_use),
            Err(error) => {
                transcript.push(format!("  picture limits not read: {error}"));
                in_use
            }
        };
        let mut sheets = Vec::new();
        match inventory_sheets(port, &shown).await {
            Ok(made) => {
                for name in &made.requested {
                    transcript.push(format!("  sheet {name}: none yet, asked for"));
                }
                sheets.extend(made.sheets);
            }
            Err((_, error)) => transcript.push(format!("  inventory sheets not read: {error}")),
        }
        transcript.push(format!(
            "  frame {number} ({name}) started for a declaration after line {turn}, declared, \
             superseding {superseded} room plate(s) ({waiting} waiting), {}",
            base.as_deref().map_or_else(
                || "painted anew".to_owned(),
                |base| format!("editing {base}")
            )
        ));
        transcript.push(format!(
            "  declared plate base={} reason={} ({} figure(s) in the picture on screen, {} in the \
             cast)",
            plan.way(),
            plan.reason,
            figures.map_or_else(|| "unread".to_owned(), |found| found.to_string()),
            self.characters.len()
        ));
        if self.in_flight.is_empty() && self.every().is_zero() {
            match start_spinner(port).await {
                Ok(before) => self.title_before = Some(before),
                Err(error) => transcript.push(format!("  spinner not shown: {error}")),
            }
        }
        let since_start = self.last_start.unwrap_or(self.began).elapsed();
        if let Err(error) = mark(
            port,
            &name,
            (FrameMoment::Started, FrameReason::Declared),
            since_start,
            None,
        )
        .await
        {
            transcript.push(format!("  frame {number} start not recorded: {error}"));
        }
        let hold = self.declare_hold();
        let screen = self.screen.clone();
        let cast_from = Some(self.cast_picture.clone());
        let mut avoid = style.avoid;
        avoid.extend(NO_DUPLICATES.map(str::to_owned));
        let task_name = name.clone();
        let who = speaker.id.clone();
        let reason = FrameReason::Declared;
        let superseded_count = u32::try_from(superseded).unwrap_or(u32::MAX);
        let started = Instant::now();
        let cast = u32::try_from(characters.len()).unwrap_or(u32::MAX);
        let task = tokio::spawn(async move {
            let attempts = [
                Attempt {
                    name: task_name.clone(),
                    painting: Painting {
                        prompt: &prompt,
                        avoid: &avoid,
                        sheets: &sheets,
                    },
                    base: base.as_deref(),
                },
                Attempt {
                    name: format!("{task_name}-redo"),
                    painting: Painting {
                        prompt: &fresh_prompt,
                        avoid: &avoid,
                        sheets: &sheets,
                    },
                    base: None,
                },
            ];
            let made = make_declared(
                port,
                attempts,
                characters,
                RoomTurn {
                    number,
                    screen,
                    cast_from,
                    cued: false,
                    emergency: Some(hold),
                },
            )
            .await;
            let finished = Instant::now();
            let (made, picture, timing) = match made {
                Ok(done) => {
                    let took = finished - started;
                    let total: Duration = done.makings.iter().sum();
                    let note = DeclaredTiming {
                        who,
                        superseded: superseded_count,
                        queue_wait_ms: saturating_millis(took.saturating_sub(total)),
                        made_ms: saturating_millis(done.makings.last().copied().unwrap_or(took)),
                        base: plan.way().to_owned(),
                        reason: plan.reason.to_owned(),
                        attempt: done.attempt,
                        first_made_ms: done
                            .makings
                            .first()
                            .filter(|_| done.makings.len() > 1)
                            .map(|first| saturating_millis(*first)),
                        rejected: done.rejected.map(|people| DeclaredRejected {
                            people: u32::try_from(people).unwrap_or(u32::MAX),
                            cast,
                        }),
                    };
                    // A refused plate is a failed one: the room stays as it is.
                    let moment = if done.shown.on_screen {
                        Some(FrameMoment::Shown)
                    } else if done.rejected.is_some() {
                        Some(FrameMoment::Failed)
                    } else {
                        None
                    };
                    let timing = match moment {
                        Some(moment) => {
                            mark(port, &task_name, (moment, reason), took, Some(note)).await
                        }
                        None => match done.shown.dropped {
                            Some(why) => mark_dropped(port, &task_name, reason, took, why).await,
                            None => Ok(()),
                        },
                    };
                    (Ok(done.shown), done.picture, timing)
                }
                Err(failure) => (Err(failure), None, Ok(())),
            };
            Made {
                made,
                finished,
                timing,
                picture,
            }
        });
        // The newest picture shows the declared action; the talk before it is behind it.
        self.picture = scene_words;
        let cleared = self.since.len();
        self.since.clear();
        for judging in &mut self.judging {
            judging.lines_judged = judging.lines_judged.saturating_sub(cleared);
        }
        self.peak_percent = 0;
        self.last_cued = Some(started);
        self.in_flight.push_back(InFlight {
            number,
            name,
            reason,
            started,
            after_turn: turn,
            hold: None,
            task,
        });
    }

    /// Shows a picture the library already holds, as it is (`@show <picture> | <caption> |
    /// <seconds>`, after line `turn`): an insert of it with the room's layout, its caption on the
    /// caption bar, held for `hold`, then the room returns. Nothing is made and no model is asked.
    /// It is shown at once (`claim_reveal`), and is no new picture of the talk: the plates waiting
    /// for their slots are not dropped, and the picture the next edit starts from does not
    /// change. A picture the library does not hold is refused, logged with its reason, and
    /// nothing changes on screen.
    pub(super) async fn reveal(
        &mut self,
        port: u16,
        turn: u32,
        (picture, caption, hold): (&str, &str, Duration),
        transcript: &mut Vec<String>,
    ) {
        self.collect(port, false, transcript).await;
        self.end_insert(port, false, transcript).await;
        self.end_declared(port, false, transcript).await;
        let revealed = |refused, behind| RevealedTiming {
            picture: picture.to_owned(),
            hold_ms: saturating_millis(hold),
            refused,
            behind,
        };
        let name = format!("{}-show-{}", self.options.name, self.reveals + 1);
        match reveal_refusal(port, picture).await {
            Ok(None) => {}
            Ok(Some(refusal)) => {
                transcript.push(format!("  {picture} is not shown: {}", refusal.key()));
                let failed = (FrameMoment::Failed, Duration::ZERO);
                if let Err(error) =
                    mark_revealed(port, &name, failed, revealed(Some(refusal), 0)).await
                {
                    transcript.push(format!("  refusal not recorded: {error}"));
                }
                return;
            }
            Err(error) => {
                transcript.push(format!(
                    "  {picture} is not shown, the library is not read: {error}"
                ));
                return;
            }
        }
        // A revealed picture waits for the ones before it to hold their whole time: a sequence.
        let ticket = queue_reveal(&self.screen);
        let behind = u32::try_from(ticket.behind).unwrap_or(u32::MAX);
        self.reveals += 1;
        let since_start = self.last_start.unwrap_or(self.began).elapsed();
        if let Err(error) = mark_revealed(
            port,
            &name,
            (FrameMoment::Started, since_start),
            revealed(None, behind),
        )
        .await
        {
            transcript.push(format!("  start of {name} not recorded: {error}"));
        }
        transcript.push(format!(
            "  {name} started: {picture} for {:.1} s, no picture made, {behind} ahead of it",
            hold.as_secs_f32()
        ));
        let screen = self.screen.clone();
        let room = self.cast_picture.clone();
        let task_name = name.clone();
        let picture = picture.to_owned();
        let caption = (!caption.is_empty()).then(|| caption.to_owned());
        let started = Instant::now();
        let task = tokio::spawn(async move {
            let made = make_reveal(
                port,
                &screen,
                Reveal {
                    name: &task_name,
                    picture: &picture,
                    room: &room,
                    caption: caption.as_deref(),
                    hold,
                },
                ticket,
            )
            .await
            .map(|(summary, captioned)| Shown {
                own_cast: false,
                on_screen: true,
                dropped: None,
                making: None,
                summary,
                captioned,
            });
            let finished = Instant::now();
            let timing = match &made {
                Ok(_) => {
                    let shown = (FrameMoment::Shown, finished - started);
                    mark_revealed(
                        port,
                        &task_name,
                        shown,
                        RevealedTiming {
                            picture,
                            hold_ms: saturating_millis(hold),
                            refused: None,
                            behind,
                        },
                    )
                    .await
                }
                Err(_) => Ok(()),
            };
            Made {
                made,
                finished,
                timing,
                picture: None,
            }
        });
        self.in_flight.push_back(InFlight {
            number: 0,
            name,
            reason: FrameReason::Reveal,
            started,
            after_turn: turn,
            hold: Some(hold),
            task,
        });
    }

    /// Asks the frame judge `request` (after line `turn`) as an owned task, considered by the tick
    /// once it answers.
    fn ask_later(&mut self, port: u16, turn: u32, request: FrameRequest, focus: Option<FocusOn>) {
        self.judging.push_back(Judging {
            turn,
            lines_judged: self.since.len(),
            asked: Instant::now(),
            reason: request.ask.reason(),
            focus,
            task: tokio::spawn(async move { ask_judge(port, &request).await }),
        });
    }

    /// What the watch does between lines: reports the frames that finished, gives the screen back
    /// to the room once an insert's hold is over, and when a slot of the schedule comes, asks for
    /// a frame of the room (the last line said was `turn`). A slot that comes while `parallel`
    /// room frames are being made (or the most are made) is missed, and the transcript, the host's
    /// `frame_schedule` line and the toolbar's timer say so.
    pub(super) async fn on_tick(&mut self, port: u16, turn: u32, transcript: &mut Vec<String>) {
        self.collect(port, false, transcript).await;
        self.end_insert(port, false, transcript).await;
        self.end_declared(port, false, transcript).await;
        self.take_judgements(port, turn, transcript).await;
        let every = self.every();
        let due = slots_due(self.began, every, self.next_slot, Instant::now());
        // A judge being asked is a room frame on its way.
        let coming = self.rooms_in_flight() + self.judging.len();
        let judge_free = self
            .judging
            .iter()
            .filter(|judging| judging.reason == FrameReason::Schedule)
            .count()
            < MAXIMUM_SCHEDULE_JUDGES;
        let may_start = due > 0
            && judge_free
            && self.made < self.options.maximum_frames
            && coming < self.options.parallel;
        self.next_slot = self.next_slot.saturating_add(due);
        let missed = due.saturating_sub(u32::from(may_start));
        if missed > 0 {
            self.missed = self.missed.saturating_add(missed);
            transcript.push(format!(
                "  {missed} scheduled slot(s) missed after line {turn}: {} room frame(s) being \
                 made, {} made",
                self.rooms_in_flight(),
                self.made
            ));
        }
        let next = (!every.is_zero()).then(|| slot_due(self.began, every, self.next_slot));
        self.tell_schedule(port, Schedule::Running { due: next }, transcript)
            .await;
        let last_asked = self.judging.back().map(|judging| judging.asked);
        let since_start = self
            .last_start
            .max(last_asked)
            .unwrap_or(self.began)
            .elapsed();
        let taking = self
            .room_making
            .map(|making| making + self.judge_taking.unwrap_or_default());
        let lead = !may_start
            && self.made < self.options.maximum_frames
            && judge_free
            && coming < self.options.parallel
            && needs_lead(coming, taking, every, since_start);
        if lead {
            transcript.push(format!(
                "  a room frame starts ahead of its slot: {} being made, {} judged",
                self.rooms_in_flight(),
                self.judging.len()
            ));
        }
        if may_start || lead {
            let request = self.request(port, FrameAsk::Schedule {}, transcript).await;
            self.ask_later(port, turn, request, None);
        }
    }

    /// Considers the judgements that came back, in the order they were asked, and counts how long
    /// each took into the average. A schedule judgement overtaken by a frame cued by a line since
    /// it was asked is dropped, and the lines said while a judge worked stay for the next
    /// judgement, never cleared unjudged.
    async fn take_judgements(&mut self, port: u16, turn: u32, transcript: &mut Vec<String>) {
        while let Some(index) = self
            .judging
            .iter()
            .position(|judging| judging.task.is_finished())
        {
            let Some(judging) = self.judging.remove(index) else {
                break;
            };
            let took = judging.asked.elapsed();
            self.judge_taking = Some(match self.judge_taking {
                Some(average) => (average * 3 + took) / 4,
                None => took,
            });
            let judged = match judging.task.await {
                Ok(judged) => judged,
                Err(_) => {
                    transcript.push(format!(
                        "  frame judge asked after line {} stopped without an answer",
                        judging.turn
                    ));
                    continue;
                }
            };
            // A judgement of the room asked before a frame a line cued since judged lines that
            // frame has moved past (a focus is of its clue, whenever it answers).
            let room = judging.reason != FrameReason::Focus;
            if room && self.last_cued.is_some_and(|cued| cued > judging.asked) {
                transcript.push(format!(
                    "  frame judge asked after line {} dropped: a frame was cued since",
                    judging.turn
                ));
                continue;
            }
            let judged_lines = judging.lines_judged.min(self.since.len());
            let later = self.since.split_off(judged_lines);
            if judging.focus.is_some() {
                self.focus = judging.focus;
            }
            self.consider(port, judging.turn, judged, transcript).await;
            // Lines said while a line's judge worked are judged now, not at the next line (a
            // close look said as the talk stops would wait for the schedule).
            let fresh = !later.is_empty();
            self.since.extend(later);
            if judging.reason == FrameReason::Line && fresh && self.line_judge_wanted() {
                let request = self.request(port, FrameAsk::Line {}, transcript).await;
                self.ask_later(port, turn, request, None);
            }
        }
    }

    /// The frame judge's request for `ask`, over what the picture shows and the lines since, with
    /// the length its picture of the scene keeps to when the image backend bounds its text.
    async fn request(
        &self,
        port: u16,
        ask: FrameAsk,
        transcript: &mut Vec<String>,
    ) -> FrameRequest {
        let picture_target_characters = match ask {
            // A focus and a declaration ask no model: their picture is given.
            FrameAsk::Focus { .. } | FrameAsk::Declare { .. } => None,
            FrameAsk::Line {} | FrameAsk::Schedule {} => {
                self.picture_target(port, transcript).await
            }
        };
        FrameRequest {
            picture: self.picture.clone(),
            lines: self.since.clone(),
            speakers: self.speakers.clone(),
            gate_percent: self.options.gate_percent,
            model: self.options.judge_model.clone(),
            last_shot: self.last_shot.clone(),
            peak_percent: self.peak_percent,
            last_inspected: self.last_inspected.clone(),
            ask,
            picture_target_characters,
        }
    }

    /// The length the judge's picture of the scene keeps to (`picture_target`) when the host's
    /// image backend bounds its text (owner, 2026-11-09: about 900 characters on Forge); none for
    /// a backend without a bound, or when what it is measured with cannot be had, which the
    /// transcript says.
    async fn picture_target(&self, port: u16, transcript: &mut Vec<String>) -> Option<usize> {
        let limits = match image_limits(port).await {
            Ok(limits) => limits,
            Err(error) => {
                transcript.push(format!(
                    "  the frame judge is given no target length: {error}"
                ));
                return None;
            }
        };
        let maximum = limits.maximum_text_characters?;
        let sheets = match &self.options.place {
            Some(place) => match sheets_at(port, place).await {
                Ok(sheets) => sheets,
                Err(error) => {
                    transcript.push(format!("  the sheets of {place} are not read: {error}"));
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        match self.fixed_characters(&limits, &sheets) {
            Ok(fixed) => Some(picture_target(maximum, fixed)),
            Err(error) => {
                transcript.push(format!(
                    "  the frame judge is given no target length: {error}"
                ));
                None
            }
        }
    }

    /// How long the text of a room plate of this watch is with no picture of the scene, counted as
    /// the backend counts it: the house look, the longest built-in shot (and the built-in rule
    /// of an edit), the `sheets` at the place as `fit_pictures` attaches or describes them when
    /// every character has a reference sheet, the references' lines and the purpose's direction.
    pub(super) fn fixed_characters(
        &self,
        limits: &BackendLimits,
        sheets: &[Thing],
    ) -> Result<usize, GenerateRequestError> {
        let style = BriefStyle::default();
        let config = FrameConfig::default();
        let shot_look = config
            .shots
            .iter()
            .map(|shot| shot.look.as_str())
            .max_by_key(|look| look.len())
            .unwrap_or_default();
        let base = self.options.edit.then_some(self.first_picture.as_str());
        let edit_rule = base.map(|_| config.edit_rule.as_str());
        let prompt = room_prompt(&style, edit_rule, shot_look, "");
        let people = self
            .characters
            .iter()
            .map(|character| reference_sheet_name(&character.name))
            .collect();
        let room = limits
            .maximum_pictures
            .saturating_sub(usize::from(base.is_some()));
        let painting = Painting {
            prompt: &prompt,
            avoid: &style.avoid,
            sheets,
        };
        let fitted = fit_pictures(room, people, sheets);
        let request = plate_request(&self.options.name, &painting, base, fitted);
        Ok(request.checked()?.sent_text(limits).encode_utf16().count())
    }

    /// Reports `judged` after line `turn` and starts the next frame when it calls for one: a focus
    /// or a new inspection at once (an inspection while fewer than `parallel` inserts are being
    /// made), a change past the gate or the schedule once the hold has passed.
    pub(super) async fn consider(
        &mut self,
        port: u16,
        turn: u32,
        judged: Result<FrameJudgement, DevError>,
        transcript: &mut Vec<String>,
    ) {
        let judgement = match judged {
            Ok(judgement) => judgement,
            Err(error) => {
                transcript.push(format!("  frame judge failed after line {turn}: {error}"));
                return;
            }
        };
        let reason = match &judgement.ask {
            FrameAsk::Focus { .. } => FrameReason::Focus,
            _ if judgement.inspection => FrameReason::Inspection,
            ask => ask.reason(),
        };
        let when = match judgement.ask.reason() {
            FrameReason::Schedule => format!("on the schedule, after line {turn}"),
            FrameReason::Focus => format!("for a focus, after line {turn}"),
            FrameReason::Line | FrameReason::Inspection => format!("after line {turn}"),
            FrameReason::Declared => format!("for a declaration, after line {turn}"),
            FrameReason::Reveal => format!("for a revealed picture, after line {turn}"),
            FrameReason::Walk => format!("for a walk, after line {turn}"),
        };
        let changes: Vec<String> = judgement
            .changes
            .iter()
            .map(|change| {
                format!(
                    "{} {}: {}",
                    shown_name(&self.speakers, change.character.as_deref()),
                    change.kind,
                    change.now
                )
            })
            .collect();
        transcript.push(format!(
            "  frame judge {when}: {}% changed, {}% since the frame (gate {}){}",
            judgement.changed_percent,
            judgement.since_percent,
            judgement.gate_percent,
            if changes.is_empty() {
                String::new()
            } else {
                format!("; {}", changes.join("; "))
            }
        ));
        self.peak_percent = judgement.since_percent;
        // The last item looked at is told to the judge, which says whether a look is new; a line
        // with no close look keeps it (run 59 cut to the Verdant twice).
        if let Some(offered) = &judgement.inspecting {
            self.last_inspected = Some(offered.id.clone());
        }
        self.show = judgement.show.clone();
        {
            let mut screen = self.screen.lock().unwrap_or_else(PoisonError::into_inner);
            screen.minimum_shown = Duration::from_secs(self.show.minimum_shown_seconds);
            screen.room_every = self.every();
        }
        let held = self
            .last_start
            .is_some_and(|started| started.elapsed() < self.options.hold);
        let room_full = self.rooms_in_flight() >= self.options.parallel;
        let can_start = match reason {
            FrameReason::Focus
            | FrameReason::Declared
            | FrameReason::Reveal
            | FrameReason::Walk => true,
            FrameReason::Inspection => self.inserts_in_flight() < self.options.parallel,
            FrameReason::Line => !held && !room_full,
            // A slot of the schedule keeps the cadence: the hold never puts it off.
            FrameReason::Schedule => !room_full,
        };
        if !judgement.triggered || !can_start {
            return;
        }
        self.made += 1;
        let number = self.made;
        let name = format!("{}-{number}", self.options.name);
        let style = BriefStyle::default();
        // An insert is never an edit of the room: its rule keeps the people's faces as they are,
        // and an insert shows none (roam run 2: three of six were the wide plate with the person
        // still standing in it). It is painted anew from the item's look.
        let insert = reason.is_insert();
        let base = (self.options.edit && !insert).then(|| {
            if self.options.from_last {
                self.newest_picture.clone()
            } else {
                self.first_picture.clone()
            }
        });
        let prompt = room_prompt(
            &style,
            base.as_ref().map(|_| judgement.edit_rule.as_str()),
            &judgement.shot_look,
            &judgement.next_picture,
        );
        // An inspection is of an item offered here; a focus is of what its words describe.
        let offered = judgement
            .inspecting
            .clone()
            .filter(|_| reason == FrameReason::Inspection);
        // A focus is captioned with the clue's words, and drawn from its sheet when it has one.
        let focus = match &judgement.ask {
            FrameAsk::Focus { look } => Some(self.focus.take().unwrap_or_else(|| FocusOn {
                caption: look.clone(),
                sheet: None,
            })),
            FrameAsk::Line {} | FrameAsk::Schedule {} | FrameAsk::Declare { .. } => None,
        };
        let caption = focus.as_ref().map(|focus| focus.caption.clone());
        // A focus is drawn from the story's sheet of its thing; an inspection of a thing in sight
        // from the place's, so a second close look shows the same close-up.
        let sheet = match focus {
            Some(focus) => focus.sheet.map(|sheet| (sheet, SheetHome::Story)),
            None => offered
                .as_ref()
                .and_then(place_sheet)
                .map(|sheet| (sheet, SheetHome::Place)),
        };
        transcript.push(format!(
            "  frame {number} ({name}) started {when}, {}, shot {}{}{}",
            reason.key(),
            judgement.shot,
            offered
                .as_ref()
                .map_or_else(String::new, |offered| format!(" on {}", offered.name)),
            base.as_deref().map_or_else(
                || ", painted anew".to_owned(),
                |base| format!(", editing {base}")
            )
        ));
        // A watch on a schedule always has a plate coming, so the spinner would never leave the
        // screen (dungeon run 5); only frames made on cue show it.
        if self.in_flight.is_empty() && self.every().is_zero() {
            match start_spinner(port).await {
                Ok(before) => self.title_before = Some(before),
                Err(error) => transcript.push(format!("  spinner not shown: {error}")),
            }
        }
        let since_start = self.last_start.unwrap_or(self.began).elapsed();
        if let Err(error) = mark(
            port,
            &name,
            (FrameMoment::Started, reason),
            since_start,
            None,
        )
        .await
        {
            transcript.push(format!("  frame {number} start not recorded: {error}"));
        }
        let characters = self.characters.clone();
        let task_name = name.clone();
        // A close-up is framed as the item alone filling the picture.
        let prompt = if insert {
            format!("{prompt} {INSERT_FRAMING}")
        } else {
            prompt
        };
        // A room frame is painted anew, so it is shown how each thing in sight looks, from the
        // story's reference sheets of the place (owner, 2026-11-08: item details kept from plate
        // to plate; a wheel came back as a different object): a sheet's close-up is attached as
        // a picture when there is room, and a sheet without one is told in words (owner,
        // 2026-11-09).
        let mut sheets = match (&self.options.place, insert) {
            (Some(place), false) => match sheets_at(port, place).await {
                Ok(sheets) => sheets,
                Err(error) => {
                    transcript.push(format!("  the sheets of {place} are not read: {error}"));
                    Vec::new()
                }
            },
            _ => Vec::new(),
        };
        // Then what the people hold that the judge says the picture shows, in its order; a
        // missing sheet is told in words and asked for.
        if !insert && !judgement.items_in_view.is_empty() {
            let shown: Vec<(String, InventoryItem)> = judgement
                .items_in_view
                .iter()
                .map(|shown| (shown.owner.clone(), shown.item.clone()))
                .collect();
            let shown = match image_limits(port).await {
                Ok(limits) => items_that_fit(
                    &limits,
                    self.characters.len() + sheets.len(),
                    base.is_some(),
                    shown,
                ),
                Err(error) => {
                    transcript.push(format!("  picture limits not read: {error}"));
                    shown
                }
            };
            match inventory_sheets(port, &shown).await {
                Ok(made) => {
                    for name in &made.requested {
                        transcript.push(format!("  sheet {name}: none yet, asked for"));
                    }
                    sheets.extend(made.sheets);
                }
                Err((_, error)) => {
                    transcript.push(format!("  inventory sheets not read: {error}"));
                }
            }
        }
        let mut avoid = style.avoid;
        if insert {
            avoid.extend(INSERT_AVOID.map(str::to_owned));
        }
        let screen = self.screen.clone();
        let insert_hold = self.insert_hold();
        let show = self.show.clone();
        // An insert takes the cast of the newest room picture, the one it cuts away from, and
        // waits for no room frame; a room frame waits for its slot (`claim_room`).
        let cut = insert.then(|| (self.cast_picture.clone(), offered, caption, sheet));
        let cast_from = Some(self.cast_picture.clone());
        let started = Instant::now();
        let task = tokio::spawn(async move {
            let made = match cut {
                Some((room, offered, caption, sheet)) => {
                    // The insert waits for the details of an item its keeper knows: a failed
                    // backfill still cuts to the item, from the judge's picture alone, and says
                    // so. A thing in sight has no keeper; its look is in the judge's picture.
                    let kept = offered
                        .as_ref()
                        .and_then(|offered| Some((offered.keeper.as_deref()?, offered)));
                    let details = match kept {
                        Some((keeper, offered)) => {
                            Some(keeper_details(port, keeper, offered, &show).await)
                        }
                        None => None,
                    };
                    let prompt = match &details {
                        Some(Ok(KeeperDetails::Added(details))) if !details.is_empty() => {
                            format!("{prompt} {} {}", show.detail_lead, details.join(" "))
                        }
                        _ => prompt,
                    };
                    let drawn = sheet
                        .as_ref()
                        .and_then(|(sheet, _)| sheet.picture.as_deref());
                    let request = InsertRequest {
                        name: &task_name,
                        prompt: &prompt,
                        avoid: &avoid,
                        drawn,
                        room: &room,
                        caption: caption.as_deref(),
                        caption_title: sheet.as_ref().map(|(sheet, _)| sheet.name.as_str()),
                        hold: insert_hold,
                    };
                    let inserted = make_insert(port, &screen, request).await;
                    // A close-up drawn now is kept on its sheet, so the next one is the same.
                    let kept = match (&inserted, &sheet) {
                        (Ok((_, picture, _)), Some((sheet, home))) if sheet.picture.is_none() => {
                            keep_sheet_picture(port, *home, &sheet.id, picture)
                                .await
                                .err()
                        }
                        _ => None,
                    };
                    inserted.map(|(summary, _, captioned)| Shown {
                        own_cast: false,
                        on_screen: true,
                        dropped: None,
                        making: None,
                        summary: match kept {
                            None => insert_summary(summary, offered.as_ref(), details),
                            Some(error) => format!(
                                "{}; its sheet did not keep the picture: {error}",
                                insert_summary(summary, offered.as_ref(), details)
                            ),
                        },
                        captioned,
                    })
                }
                None => {
                    let painting = Painting {
                        prompt: &prompt,
                        avoid: &avoid,
                        sheets: &sheets,
                    };
                    make_frame(
                        port,
                        &task_name,
                        &painting,
                        characters,
                        base.as_deref(),
                        RoomTurn {
                            number,
                            screen,
                            cast_from,
                            cued: reason == FrameReason::Line,
                            emergency: None,
                        },
                    )
                    .await
                }
            };
            let finished = Instant::now();
            let timing = match &made {
                Ok(shown) if shown.on_screen => {
                    let took = finished - started;
                    mark(port, &task_name, (FrameMoment::Shown, reason), took, None).await
                }
                Ok(Shown {
                    dropped: Some(why), ..
                }) => mark_dropped(port, &task_name, reason, finished - started, *why).await,
                Ok(_) | Err(_) => Ok(()),
            };
            Made {
                made,
                finished,
                timing,
                picture: None,
            }
        });
        // An insert is a cut to the item: the next frame starts again from the room as it was. A
        // focus is no judgement of the lines: they wait, as they were, for the next room frame.
        if reason != FrameReason::Focus {
            if !insert {
                self.picture = judgement.next_picture;
            }
            self.last_shot = Some(judgement.shot);
            // The judges still out count from the lines after those cleared.
            let cleared = self.since.len();
            self.since.clear();
            for judging in &mut self.judging {
                judging.lines_judged = judging.lines_judged.saturating_sub(cleared);
            }
            self.peak_percent = 0;
        }
        self.last_start = Some(started);
        if reason == FrameReason::Line {
            self.last_cued = Some(started);
        }
        self.in_flight.push_back(InFlight {
            number,
            name,
            reason,
            started,
            after_turn: turn,
            hold: None,
            task,
        });
    }

    /// The talk has moved on (the watch's input ended: play left the place): the frames still
    /// being made are of a place nobody is at, so they are dropped, never shown (dungeon run 5:
    /// waiting for them kept the party in the last place for minutes after they set off). What
    /// finished is reported; the title, a clue's caption and the schedule are put back, and an
    /// insert still up is left for the next place's picture to replace.
    pub(super) async fn leave(&mut self, port: u16, transcript: &mut Vec<String>) {
        self.collect(port, false, transcript).await;
        self.stop_judging(transcript);
        for frame in self.in_flight.drain(..) {
            frame.task.abort();
            transcript.push(format!(
                "  frame {} ({}) dropped: the talk moved on before it was made",
                frame.number, frame.name
            ));
            let why = DroppedWhy::Abandoned;
            if let Err(error) = mark_dropped(
                port,
                &frame.name,
                frame.reason,
                frame.started.elapsed(),
                why,
            )
            .await
            {
                transcript.push(format!("  end of {} not recorded: {error}", frame.name));
            }
        }
        if let Some(before) = self.title_before.take()
            && let Err(error) = put_title_back(port, before).await
        {
            transcript.push(format!("  title not put back: {error}"));
        }
        self.insert_up = None;
        self.declared_ended(port, Instant::now(), transcript).await;
        self.restore_caption(port, transcript).await;
        self.tell_schedule(port, Schedule::Stopped, transcript)
            .await;
    }

    /// Waits for the frames being made and reports them, and brings the room back after an
    /// insert still on screen once it has been up for its whole hold.
    pub(super) async fn finish(&mut self, port: u16, transcript: &mut Vec<String>) {
        self.stop_judging(transcript);
        self.collect(port, true, transcript).await;
        self.end_insert(port, true, transcript).await;
        self.end_declared(port, true, transcript).await;
        self.tell_schedule(port, Schedule::Stopped, transcript)
            .await;
    }

    /// Stops the schedule's judges not answered yet: the talk is over, so no frame is started.
    fn stop_judging(&mut self, transcript: &mut Vec<String>) {
        for judging in self.judging.drain(..) {
            judging.task.abort();
            transcript.push(format!(
                "  frame judge asked after line {} dropped: the talk moved on",
                judging.turn
            ));
        }
    }

    /// Tells the host the schedule, for the toolbar's plate timer, when it changed.
    async fn tell_schedule(&mut self, port: u16, schedule: Schedule, transcript: &mut Vec<String>) {
        let every = self.every();
        let (running, due) = match schedule {
            Schedule::Running { due } => (true, due),
            Schedule::Stopped => (false, None),
        };
        let told = running.then_some((every, due, self.missed));
        if running && told == self.told_schedule {
            return;
        }
        let request = FrameScheduleRequest {
            running,
            every_ms: saturating_millis(every),
            due_in_ms: due
                .map(|due| saturating_millis(due.saturating_duration_since(Instant::now()))),
            missed: self.missed,
        };
        let reply: Result<FrameTimingReply, DevError> =
            post_json(port, PERSONA_FRAME_SCHEDULE, &request).await;
        match reply.and_then(reply_value) {
            Ok(_) => self.told_schedule = told,
            Err(error) => transcript.push(format!("  schedule not told to the host: {error}")),
        }
    }

    /// Puts the newest room picture back after the insert on screen once it has been up for
    /// the insert hold; when `finishing`, waits out what is left of the hold first. The caption
    /// its clue replaced comes back first, so no clue's words lie over the room; a newer frame
    /// already up is left there.
    async fn end_insert(&mut self, port: u16, finishing: bool, transcript: &mut Vec<String>) {
        let Some(up) = &self.insert_up else {
            return;
        };
        let shown = up.shown;
        let revealed = up.reason == FrameReason::Reveal;
        // A revealed picture with another behind it is replaced by that one at the end of its hold,
        // not by the room: the next one's report ends it.
        if revealed
            && self
                .in_flight
                .iter()
                .any(|frame| frame.reason == FrameReason::Reveal)
        {
            return;
        }
        let hold = up.hold.unwrap_or_else(|| self.insert_hold());
        let left = hold.saturating_sub(shown.elapsed());
        if !left.is_zero() {
            if !finishing {
                return;
            }
            tokio::time::sleep(left).await;
        }
        // A room frame that waited out the hold went up first: it ends the insert when it is
        // reported, and the clue's words leave with the insert at once.
        let room_back = if revealed {
            claim_after_reveal(&self.screen, shown).await
        } else {
            claim_after_insert(&self.screen, shown).await
        };
        self.restore_caption(port, transcript).await;
        if !room_back {
            return;
        }
        let room = self.newest_picture.clone();
        match show_with_cast(port, &room, &self.cast_picture).await {
            Ok(()) => {
                let up = shown.elapsed().as_secs_f32();
                transcript.push(format!("  back to {room} after the insert ({up:.1} s up)"));
            }
            Err((step, error)) => transcript.push(format!(
                "  {room} not put back after the insert at {}: {error}",
                step.key()
            )),
        }
        self.insert_ended(port, Instant::now(), transcript).await;
    }

    /// Records that the insert on screen gave the screen up at `ended`.
    async fn insert_ended(&mut self, port: u16, ended: Instant, transcript: &mut Vec<String>) {
        let Some(up) = self.insert_up.take() else {
            return;
        };
        let took = ended.saturating_duration_since(up.shown);
        if let Err(error) = mark(port, &up.name, (FrameMoment::Ended, up.reason), took, None).await
        {
            transcript.push(format!("  end of insert {} not recorded: {error}", up.name));
        }
    }

    /// Records that the emergency plate on screen has been up for its whole hold (`ended`); when
    /// `finishing`, waits out what is left of the hold first. The screen is not changed: the room's
    /// plates go on from here by their own rules.
    async fn end_declared(&mut self, port: u16, finishing: bool, transcript: &mut Vec<String>) {
        let Some(up) = &self.declared_up else {
            return;
        };
        let left = self.declare_hold().saturating_sub(up.shown.elapsed());
        if !left.is_zero() {
            if !finishing {
                return;
            }
            tokio::time::sleep(left).await;
        }
        self.declared_ended(port, Instant::now(), transcript).await;
    }

    /// Records that the emergency plate on screen gave up the screen's hold at `ended`.
    async fn declared_ended(&mut self, port: u16, ended: Instant, transcript: &mut Vec<String>) {
        let Some(up) = self.declared_up.take() else {
            return;
        };
        let took = ended.saturating_duration_since(up.shown);
        if let Err(error) = mark(port, &up.name, (FrameMoment::Ended, up.reason), took, None).await
        {
            transcript.push(format!(
                "  end of declared plate {} not recorded: {error}",
                up.name
            ));
        }
    }

    /// Puts back the caption an insert's clue replaced, when one is up.
    async fn restore_caption(&mut self, port: u16, transcript: &mut Vec<String>) {
        let Some(captioned) = self.captioned.take() else {
            return;
        };
        if let Err(error) = put_caption_back(port, captioned.before).await {
            transcript.push(format!("  caption not put back after the insert: {error}"));
        }
    }

    /// Counts a room frame's making time into the average (each new one a quarter of it).
    fn made_room(&mut self, making: Option<Duration>) {
        if let Some(making) = making {
            self.room_making = Some(match self.room_making {
                Some(average) => (average * 3 + making) / 4,
                None => making,
            });
        }
    }

    /// The room frames being made.
    fn rooms_in_flight(&self) -> usize {
        self.in_flight
            .iter()
            .filter(|frame| !frame.reason.is_insert() && frame.reason != FrameReason::Declared)
            .count()
    }

    /// The inserts being made: a pool of `parallel` beside the room frames, so a close look is
    /// not lost while another insert is still being made (run 62 lost two).
    fn inserts_in_flight(&self) -> usize {
        self.in_flight
            .iter()
            .filter(|frame| frame.reason.is_insert())
            .count()
    }

    /// How long an emergency plate holds the screen: the command's `declare_hold=`, or the
    /// settings'.
    fn declare_hold(&self) -> Duration {
        self.options
            .declare_hold
            .unwrap_or(Duration::from_secs(self.show.declare_hold_seconds))
    }

    /// How long an insert stays up: the command's `insert_hold=`, or the settings'.
    fn insert_hold(&self) -> Duration {
        self.options
            .insert_hold
            .unwrap_or(Duration::from_secs(self.show.insert_hold_seconds))
    }

    /// How long after the last frame started one starts on the schedule: the command's `every=`,
    /// or the settings'.
    fn every(&self) -> Duration {
        self.options
            .every
            .unwrap_or(Duration::from_secs(self.show.every_seconds))
    }

    /// Reports the frames that finished, oldest first (all of them when `wait`; an insert that
    /// finished before a slower room frame is reported at once), and puts the scene's title back
    /// once none is being made.
    async fn collect(&mut self, port: u16, wait: bool, transcript: &mut Vec<String>) {
        let mut index = 0;
        while index < self.in_flight.len() {
            if (wait || self.in_flight[index].task.is_finished())
                && let Some(frame) = self.in_flight.remove(index)
            {
                self.report(port, frame, transcript).await;
            } else {
                index += 1;
            }
        }
        if self.in_flight.is_empty()
            && let Some(before) = self.title_before.take()
            && let Err(error) = put_title_back(port, before).await
        {
            transcript.push(format!("  title not put back: {error}"));
        }
    }

    async fn report(&mut self, port: u16, frame: InFlight, transcript: &mut Vec<String>) {
        let made = frame.task.await;
        let waited = frame.started.elapsed().as_secs_f32();
        match made {
            Ok(Made {
                made: Ok(shown),
                finished,
                timing,
                picture,
            }) => {
                if let Err(error) = timing {
                    transcript.push(format!(
                        "  frame {} shown, not recorded: {error}",
                        frame.number
                    ));
                }
                // An insert is a cut away from the room: the room stays the newest picture. One
                // that replaced an insert ended that one; a clue's caption stays with the clue
                // first shown (the caption before it is the one to put back), and an insert with
                // no caption shows no clue's words.
                if frame.reason.is_insert() {
                    transcript.push(format!("  {}", shown.summary));
                    self.insert_ended(port, finished, transcript).await;
                    match shown.captioned {
                        Some(captioned) => {
                            self.captioned.get_or_insert(captioned);
                        }
                        None => self.restore_caption(port, transcript).await,
                    }
                    self.insert_up = Some(InsertUp {
                        name: frame.name.clone(),
                        reason: frame.reason,
                        shown: finished,
                        hold: frame.hold,
                    });
                } else if !shown.on_screen {
                    // An emergency plate jumped the queue: its making time says nothing of a
                    // room plate's.
                    if frame.reason != FrameReason::Declared {
                        self.made_room(shown.making);
                    }
                    transcript.push(format!("  {}", shown.summary));
                } else {
                    if frame.reason == FrameReason::Declared {
                        // A declaration that goes up ends the one before it, and holds the screen.
                        self.declared_ended(port, finished, transcript).await;
                        self.declared_up = Some(InsertUp {
                            name: frame.name.clone(),
                            reason: frame.reason,
                            shown: finished,
                            hold: None,
                        });
                    } else {
                        self.made_room(shown.making);
                    }
                    // A plate older than one already up fills a slot, but the newest picture (the
                    // base of an edit, the room an insert goes back to) only moves forward.
                    if frame.number > self.newest_number {
                        self.newest_number = frame.number;
                        self.newest_picture =
                            picture.unwrap_or_else(|| format!("{}.png", frame.name));
                    }
                    if shown.own_cast && frame.number >= self.newest_number {
                        self.cast_picture = self.newest_picture.clone();
                    } else {
                        transcript.push(format!("  {}", shown.summary));
                    }
                    self.restore_caption(port, transcript).await;
                    self.insert_ended(port, finished, transcript).await;
                }
                transcript.push(format!(
                    "  frame {} ({}) shown {waited:.1} s after it started at line {}",
                    frame.number, frame.name, frame.after_turn
                ));
            }
            Ok(Made {
                made: Err((step, error)),
                finished,
                ..
            }) => {
                transcript.push(format!(
                    "  frame {} ({}) failed at {} after {waited:.1} s: {error}",
                    frame.number,
                    frame.name,
                    step.key()
                ));
                let took = finished - frame.started;
                if let Err(error) = mark(
                    port,
                    &frame.name,
                    (FrameMoment::Failed, frame.reason),
                    took,
                    None,
                )
                .await
                {
                    transcript.push(format!(
                        "  frame {} failure not recorded: {error}",
                        frame.number
                    ));
                }
            }
            Err(_) => {
                transcript.push(format!(
                    "  frame {} ({}) stopped without an answer",
                    frame.number, frame.name
                ));
                let why = DroppedWhy::Abandoned;
                if let Err(error) = mark_dropped(
                    port,
                    &frame.name,
                    frame.reason,
                    frame.started.elapsed(),
                    why,
                )
                .await
                {
                    transcript.push(format!("  end of {} not recorded: {error}", frame.name));
                }
            }
        }
    }
}

/// What an insert did: `summary`, and for an item its keeper knows, what their details came to.
fn insert_summary(
    summary: String,
    offered: Option<&PlaceItem>,
    details: Option<Result<KeeperDetails, DevError>>,
) -> String {
    let kept = offered.and_then(|offered| Some((offered.keeper.as_deref()?, &offered.name)));
    match (kept, details) {
        (Some((keeper, name)), Some(Ok(KeeperDetails::Known))) => {
            format!("{summary}; {keeper} already knew the {name} up close, nothing backfilled")
        }
        (Some((keeper, name)), Some(Ok(KeeperDetails::Added(details)))) => format!(
            "{summary}; {} detail(s) of the {name} backfilled into {keeper}",
            details.len(),
        ),
        (_, Some(Err(error))) => format!("{summary}; no details backfilled ({error})"),
        _ => summary,
    }
}
