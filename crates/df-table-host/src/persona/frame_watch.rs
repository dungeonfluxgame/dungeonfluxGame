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
//! insert's picture.

// TODO(DEV-12): tests on a fake host: frames made in parallel are shown in the order they were
// started (a later one that finishes first waits for the one before it), the spinner shows once
// and the title is put back when the last frame lands, the change since a frame only grows, and
// the hold and the most frames are kept; a frame stays up the least time before the next made
// alongside it; an insert cuts in inside the hold, keeps the room's cast, comes back to the
// newest room after the insert hold (whole, at the end too) and carries the keeper's backfill.

use std::collections::VecDeque;
use std::sync::PoisonError;
use std::time::{Duration, Instant};

use df_content::persona::backfill::Gap;
use df_content::persona::{PersonaEdit, PersonaField};
use df_playground::lobby::model::SceneTitle;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::converse::{Speaker, speakers_of};
use super::frame::{FrameJudgement, FrameReply, FrameRequest, PERSONA_FRAME, ShowSettings};
use super::plate::{
    PlateFailure, RoomTurn, SharedScreen, Shown, claim_screen, make_frame, make_insert,
    put_title_back, show_with_cast, start_spinner,
};
use super::request::{BackfillRequest, Backfilled, CoverageRequest, Covered, PersonaReply};
use super::routes::path as persona_path;
use crate::cast::request::CastList;
use crate::cast::routes::path as cast_path;
use crate::dev::DevError;
use crate::dev_client::{get_json, post_json, reply_value};
use crate::locate::request::CharacterRequest;
use crate::movement::brief::BriefStyle;
use crate::world::PlaceItem;

/// How long a frame stays at least before the next one starts, by default.
pub(super) const DEFAULT_HOLD_SECONDS: u64 = 30;
/// How many frames one conversation makes at most, by default.
pub(super) const DEFAULT_MAXIMUM_FRAMES: u32 = 6;
/// How many frames may be made at once, at most.
pub(super) const MAXIMUM_PARALLEL_FRAMES: usize = 4;

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
}

/// The frame judge's answer to `request`, or its typed refusal.
async fn ask_judge(port: u16, request: &FrameRequest) -> Result<FrameJudgement, DevError> {
    let reply: FrameReply = post_json(port, PERSONA_FRAME, request).await?;
    reply_value(reply)
}

/// `persona watch`: frames for a talk spoken elsewhere, one `persona say` or `persona ask` per
/// line (as `development/market-thread.sh` does). Each line of standard input is
/// `<speaker id> | <line>`; it is judged and framed as in `persona converse frames=`. At the end of
/// input the frames still being made are waited for, and the transcript is given.
pub(super) async fn watch_lines(port: u16, options: FrameOptions) -> Result<String, DevError> {
    let listed: crate::cast::request::CastReply<CastList> = get_json(port, cast_path::CAST).await?;
    let cast = reply_value(listed)?;
    let picture = cast.picture.clone();
    let speakers = speakers_of(cast);
    let mut watch = FrameWatch::new(options, &speakers, &picture);
    let mut transcript = Vec::new();
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    let mut turn = 0;
    while let Some(entry) = input
        .next_line()
        .await
        .map_err(|error| DevError::UnreadableFile {
            path: "<stdin>".into(),
            message: error.to_string(),
        })?
    {
        let Some((id, line)) = entry.split_once(" | ") else {
            transcript.push(format!("  not '<speaker id> | <line>', skipped: {entry}"));
            continue;
        };
        let Some(speaker) = speakers.iter().find(|speaker| speaker.id == id.trim()) else {
            transcript.push(format!("  {} is not in the scene, skipped", id.trim()));
            continue;
        };
        turn += 1;
        transcript.push(format!(
            "[{turn:02}] {}: {}",
            speaker.display_name,
            line.trim()
        ));
        if let Some(judged) = watch
            .after_line(port, speaker, line.trim(), &mut transcript)
            .await
        {
            watch.consider(port, turn, judged, &mut transcript).await;
        }
    }
    watch.finish(port, &mut transcript).await;
    Ok(transcript.join("\n"))
}

/// What the keeper of an item's stall knows of how it looks up close, before an insert of it.
enum KeeperDetails {
    /// Their persona already holds it (the coverage judge said so); the frame judge, which reads
    /// their account, has it already.
    Known,
    /// The backfill engine added it: the text it wrote.
    Added(Vec<String>),
}

/// How the keeper of `offered`'s stall knows it up close: the coverage judge is asked first, and
/// only when their persona does not hold it does the backfill engine add it, to their public
/// backstory, the memory of their own work (expertise stays authored and is never backfilled, as
/// `planning/persona-voice-and-world-bible.md` has it), so later lines and inserts agree and a
/// second look at the same item adds nothing.
async fn keeper_details(
    port: u16,
    offered: &PlaceItem,
    show: &ShowSettings,
) -> Result<KeeperDetails, DevError> {
    // What is asked is data (`frames.json` `show.detail_topic`); the item is named after it.
    let look = format!("{} The item: {}.", show.detail_topic, offered.item.name);
    let coverage = CoverageRequest {
        character: offered.keeper.clone(),
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
        character: offered.keeper.clone(),
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
        text.push(format!(
            "  inspecting: {} ({})",
            offered.item.name, offered.item.id
        ));
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

/// A frame being made: its number, when it started and after which line, and the task.
struct InFlight {
    number: u32,
    name: String,
    /// Whether it is an insert (a cut to an item) rather than a frame of the room.
    insert: bool,
    started: Instant,
    after_turn: u32,
    /// What making it came to, and when it finished (for a frame made, when it went on screen).
    task: JoinHandle<(Result<Shown, PlateFailure>, Instant)>,
}

/// The state of the frames of one conversation.
pub(super) struct FrameWatch {
    options: FrameOptions,
    speakers: Vec<Speaker>,
    characters: Vec<CharacterRequest>,
    /// The scene's picture as the talk began, and the newest frame shown.
    first_picture: String,
    newest_picture: String,
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
    /// Fires when the newest room frame started is shown (or failed): the next one waits for it.
    newest_shown: Option<oneshot::Receiver<()>>,
    /// When the last picture went up and until when an insert holds the screen, shared with the
    /// frames being made.
    screen: SharedScreen,
    /// The scene's title before the spinner, while frames are being made.
    title_before: Option<Option<SceneTitle>>,
    last_start: Option<Instant>,
    /// The item last said to be looked at closely, so the same look makes no second insert until
    /// another item is looked at.
    last_inspected: Option<String>,
    /// When the insert on screen was shown.
    insert_shown: Option<Instant>,
    /// How frames are shown, from the newest judgement (`frames.json` `show`).
    show: ShowSettings,
}

/// A conversation that ends early (a turn failed) drops its watch: the frames still being made
/// are stopped, so no picture lands after the talk and nothing runs unowned.
impl Drop for FrameWatch {
    fn drop(&mut self) {
        for frame in &self.in_flight {
            frame.task.abort();
        }
    }
}

impl FrameWatch {
    /// A watch over the scene whose backdrop is `picture`, with these speakers.
    pub(super) fn new(options: FrameOptions, speakers: &[Speaker], picture: &str) -> Self {
        let people: Vec<String> = speakers
            .iter()
            .map(|speaker| format!("{} ({})", speaker.display_name, speaker.job))
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
            cast_picture: picture.to_owned(),
            picture: described,
            since: Vec::new(),
            made: 0,
            last_shot: None,
            peak_percent: 0,
            in_flight: VecDeque::new(),
            newest_shown: None,
            screen: SharedScreen::default(),
            title_before: None,
            last_start: None,
            last_inspected: None,
            insert_shown: None,
            show: ShowSettings::default(),
        }
    }

    /// Takes something that happens at the table into the lines since the newest frame: unlike
    /// talk, it is seen.
    pub(super) fn note_event(&mut self, what: &str) {
        self.since.push(format!("(Happening now: {what})"));
    }

    /// Takes `line` said by `speaker` into the lines since the newest frame, collects the frames
    /// that finished, and judges when a new one may start: the most frames are not made yet, and
    /// fewer than `parallel` room frames or fewer than `parallel` inserts are being made (inserts
    /// have places of their own, so a close look is not lost to slow room frames: run 60 lost
    /// two). It is judged
    /// inside the hold too, since an inspection cuts in at once. The judgement, if one was asked.
    pub(super) async fn after_line(
        &mut self,
        port: u16,
        speaker: &Speaker,
        line: &str,
        transcript: &mut Vec<String>,
    ) -> Option<Result<FrameJudgement, DevError>> {
        self.since
            .push(format!("{}: {}", speaker.display_name, line));
        self.collect(port, false, transcript).await;
        self.end_insert(port, false, transcript).await;
        if self.made >= self.options.maximum_frames
            || (self.rooms_in_flight() >= self.options.parallel
                && self.inserts_in_flight() >= self.options.parallel)
        {
            return None;
        }
        let request = FrameRequest {
            picture: self.picture.clone(),
            lines: self.since.clone(),
            speakers: self.speakers.clone(),
            gate_percent: self.options.gate_percent,
            model: self.options.judge_model.clone(),
            last_shot: self.last_shot.clone(),
            peak_percent: self.peak_percent,
            last_inspected: self.last_inspected.clone(),
        };
        Some(ask_judge(port, &request).await)
    }

    /// Reports `judged` after line `turn` and starts the next frame when it calls for one: a new
    /// inspection at once, a change past the gate once the hold has passed.
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
            "  frame judge after line {turn}: {}% changed, {}% since the frame (gate {}){}",
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
            self.last_inspected = Some(offered.item.id.clone());
        }
        self.show = judgement.show.clone();
        self.screen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .minimum_shown = Duration::from_secs(self.show.minimum_shown_seconds);
        let held = self
            .last_start
            .is_some_and(|started| started.elapsed() < self.options.hold);
        let room_full = self.rooms_in_flight() >= self.options.parallel;
        let can_start = if judgement.inspection {
            self.inserts_in_flight() < self.options.parallel
        } else {
            !held && !room_full
        };
        if !judgement.triggered || !can_start {
            return;
        }
        self.made += 1;
        let number = self.made;
        let name = format!("{}-{number}", self.options.name);
        let style = BriefStyle::default();
        let (prompt, base) = if self.options.edit {
            let base = if self.options.from_last {
                self.newest_picture.clone()
            } else {
                self.first_picture.clone()
            };
            (
                format!(
                    "{} The look stays {} this scene. Frame it as {}. The scene now: {} {}",
                    judgement.edit_rule,
                    style.direction.to_lowercase(),
                    judgement.shot_look,
                    judgement.next_picture,
                    style.writing_ban
                ),
                Some(base),
            )
        } else {
            (
                format!(
                    "{} {} Frame it as {}. {}",
                    style.direction, judgement.next_picture, judgement.shot_look, style.writing_ban
                ),
                None,
            )
        };
        transcript.push(format!(
            "  frame {number} ({name}) started after line {turn}, shot {}{}{}",
            judgement.shot,
            judgement
                .inspecting
                .as_ref()
                .filter(|_| judgement.inspection)
                .map_or_else(String::new, |offered| format!(" on {}", offered.item.name)),
            base.as_deref().map_or_else(
                || ", painted anew".to_owned(),
                |base| format!(", editing {base}")
            )
        ));
        if self.in_flight.is_empty() {
            match start_spinner(port).await {
                Ok(before) => self.title_before = Some(before),
                Err(error) => transcript.push(format!("  spinner not shown: {error}")),
            }
        }
        let characters = self.characters.clone();
        let task_name = name.clone();
        // An insert takes the cast of the newest room picture, the one it cuts away from, and
        // waits for no room frame; a room frame waits for the room frame before it.
        let insert = judgement.inspection;
        let (shown, previous) = if insert {
            (None, None)
        } else {
            let (shown, next_waits) = oneshot::channel();
            (Some(shown), self.newest_shown.replace(next_waits))
        };
        let screen = self.screen.clone();
        let insert_hold = self.insert_hold();
        let show = self.show.clone();
        let cut = insert
            .then(|| self.cast_picture.clone())
            .zip(judgement.inspecting.clone());
        let cast_from = Some(self.cast_picture.clone());
        let task = tokio::spawn(async move {
            let made = match cut {
                Some((room, offered)) => {
                    // The insert waits for the item's details: a failed backfill still cuts to
                    // the item, from the judge's picture alone, and says so.
                    let details = keeper_details(port, &offered, &show).await;
                    let prompt = match &details {
                        Ok(KeeperDetails::Added(details)) if !details.is_empty() => {
                            format!("{prompt} {} {}", show.detail_lead, details.join(" "))
                        }
                        _ => prompt,
                    };
                    let made = make_insert(
                        port,
                        &task_name,
                        &prompt,
                        base.as_deref(),
                        &room,
                        (&screen, insert_hold),
                    )
                    .await;
                    made.map(|summary| Shown {
                        own_cast: false,
                        summary: match details {
                            Ok(KeeperDetails::Known) => format!(
                                "{summary}; {} already knew the {} up close, nothing backfilled",
                                offered.keeper, offered.item.name
                            ),
                            Ok(KeeperDetails::Added(details)) => format!(
                                "{summary}; {} detail(s) of the {} backfilled into {}",
                                details.len(),
                                offered.item.name,
                                offered.keeper
                            ),
                            Err(error) => format!("{summary}; no details backfilled ({error})"),
                        },
                    })
                }
                None => {
                    make_frame(
                        port,
                        &task_name,
                        &prompt,
                        characters,
                        base.as_deref(),
                        RoomTurn {
                            previous,
                            screen,
                            cast_from,
                        },
                    )
                    .await
                }
            };
            // The next room frame goes on once this one is shown; a failed one lets it go on at
            // once (the sender is dropped).
            if made.is_ok()
                && let Some(shown) = shown
            {
                let _ = shown.send(());
            }
            (made, Instant::now())
        });
        // An insert is a cut to the item: the next frame starts again from the room as it was.
        if !judgement.inspection {
            self.picture = judgement.next_picture;
        }
        self.last_shot = Some(judgement.shot);
        self.since.clear();
        self.peak_percent = 0;
        self.last_start = Some(Instant::now());
        self.in_flight.push_back(InFlight {
            number,
            name,
            insert,
            started: Instant::now(),
            after_turn: turn,
            task,
        });
    }

    /// Waits for the frames being made and reports them, and brings the room back after an
    /// insert still on screen once it has been up for its whole hold.
    pub(super) async fn finish(&mut self, port: u16, transcript: &mut Vec<String>) {
        self.collect(port, true, transcript).await;
        self.end_insert(port, true, transcript).await;
    }

    /// Puts the newest room picture back after the insert on screen once it has been up for
    /// the insert hold; when `finishing`, waits out what is left of the hold first.
    async fn end_insert(&mut self, port: u16, finishing: bool, transcript: &mut Vec<String>) {
        let Some(shown) = self.insert_shown else {
            return;
        };
        let left = self.insert_hold().saturating_sub(shown.elapsed());
        if !left.is_zero() {
            if !finishing {
                return;
            }
            tokio::time::sleep(left).await;
        }
        claim_screen(&self.screen, false, None).await;
        let up = shown.elapsed().as_secs_f32();
        let room = self.newest_picture.clone();
        match show_with_cast(port, &room, &self.cast_picture).await {
            Ok(()) => transcript.push(format!("  back to {room} after the insert ({up:.1} s up)")),
            Err((step, error)) => transcript.push(format!(
                "  {room} not put back after the insert at {}: {error}",
                step.key()
            )),
        }
        self.insert_shown = None;
    }

    /// The room frames being made.
    fn rooms_in_flight(&self) -> usize {
        self.in_flight.iter().filter(|frame| !frame.insert).count()
    }

    /// The inserts being made: a pool of `parallel` beside the room frames, so a close look is
    /// not lost while another insert is still being made (run 62 lost two).
    fn inserts_in_flight(&self) -> usize {
        self.in_flight.iter().filter(|frame| frame.insert).count()
    }

    /// How long an insert stays up: the command's `insert_hold=`, or the settings'.
    fn insert_hold(&self) -> Duration {
        self.options
            .insert_hold
            .unwrap_or(Duration::from_secs(self.show.insert_hold_seconds))
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
                self.report(frame, transcript).await;
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

    async fn report(&mut self, frame: InFlight, transcript: &mut Vec<String>) {
        let made = frame.task.await;
        let waited = frame.started.elapsed().as_secs_f32();
        match made {
            Ok((Ok(shown), shown_at)) => {
                // An insert is a cut away from the room: the room stays the newest picture.
                if frame.insert {
                    transcript.push(format!("  {}", shown.summary));
                    self.insert_shown = Some(shown_at);
                } else {
                    self.newest_picture = format!("{}.png", frame.name);
                    if shown.own_cast {
                        self.cast_picture = self.newest_picture.clone();
                    } else {
                        transcript.push(format!("  {}", shown.summary));
                    }
                    self.insert_shown = None;
                }
                transcript.push(format!(
                    "  frame {} ({}) shown {waited:.1} s after it started at line {}",
                    frame.number, frame.name, frame.after_turn
                ));
            }
            Ok((Err((step, error)), _)) => transcript.push(format!(
                "  frame {} ({}) failed at {} after {waited:.1} s: {error}",
                frame.number,
                frame.name,
                step.key()
            )),
            Err(_) => transcript.push(format!(
                "  frame {} ({}) stopped without an answer",
                frame.number, frame.name
            )),
        }
    }
}
