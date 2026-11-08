//! Frames during `persona converse frames=NAME`: the picture follows the talk. After each line
//! the frame judge (`POST /dev/persona/frame`) reads what the picture shows and the lines said
//! since it was made; when the visible change passes the gate, a new frame is made in the
//! background (by default an edit of the scene's first picture, so faces and the room stay the
//! same) while the talk goes on, and put behind the scene when it is ready. Up to `parallel`
//! frames are made at once (one by default); they are shown in the order they were started, each
//! after the one before it, and the scene's spinner shows while any is being made. Lines said
//! while no frame may start wait for the next judgement. Every setting is a command option
//! over the defaults of `persona/frames.json`; the transcript says when each frame was judged,
//! started and shown.

// TODO(DEV-12): tests on a fake host: frames made in parallel are shown in the order they were
// started (a later one that finishes first waits for the one before it), the spinner shows once
// and the title is put back when the last frame lands, the change since a frame only grows, and
// the hold and the most frames are kept.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use df_playground::lobby::model::SceneTitle;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::converse::{Speaker, speakers_of};
use super::frame::{FrameJudgement, FrameReply, FrameRequest, PERSONA_FRAME};
use super::plate::{PlateFailure, make_frame, put_title_back, start_spinner};
use crate::cast::request::CastList;
use crate::cast::routes::path as cast_path;
use crate::dev::DevError;
use crate::dev_client::{get_json, post_json, reply_value};
use crate::locate::request::CharacterRequest;
use crate::movement::brief::BriefStyle;

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
    /// The model the judge asks instead of the conversation's.
    pub judge_model: Option<String>,
}

/// The frame judge's answer to `request`, or its typed refusal.
async fn ask_judge(port: u16, request: &FrameRequest) -> Result<FrameJudgement, DevError> {
    let reply: FrameReply = post_json(port, PERSONA_FRAME, request).await?;
    reply_value(reply)
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
    started: Instant,
    after_turn: u32,
    task: JoinHandle<Result<String, PlateFailure>>,
}

/// The state of the frames of one conversation.
pub(super) struct FrameWatch {
    options: FrameOptions,
    speakers: Vec<Speaker>,
    characters: Vec<CharacterRequest>,
    /// The scene's picture as the talk began, and the newest frame shown.
    first_picture: String,
    newest_picture: String,
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
    /// Fires when the newest frame started is shown (or failed): the next one waits for it.
    newest_shown: Option<oneshot::Receiver<()>>,
    /// The scene's title before the spinner, while frames are being made.
    title_before: Option<Option<SceneTitle>>,
    last_start: Option<Instant>,
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
            picture: described,
            since: Vec::new(),
            made: 0,
            last_shot: None,
            peak_percent: 0,
            in_flight: VecDeque::new(),
            newest_shown: None,
            title_before: None,
            last_start: None,
        }
    }

    /// Takes something that happens at the table into the lines since the newest frame: unlike
    /// talk, it is seen.
    pub(super) fn note_event(&mut self, what: &str) {
        self.since.push(format!("(Happening now: {what})"));
    }

    /// Takes `line` said by `speaker` into the lines since the newest frame, collects the frames
    /// that finished, and judges when a new one may start: fewer than `parallel` are being made,
    /// the hold has passed and the most frames are not made yet. The judgement, if one was asked.
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
        let held = self
            .last_start
            .is_some_and(|started| started.elapsed() < self.options.hold);
        if self.in_flight.len() >= self.options.parallel
            || held
            || self.made >= self.options.maximum_frames
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
        };
        Some(ask_judge(port, &request).await)
    }

    /// Reports `judged` after line `turn` and, when it passed the gate, starts the next frame.
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
        if !judgement.triggered {
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
            "  frame {number} ({name}) started after line {turn}, shot {}{}",
            judgement.shot,
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
        let after = self.newest_shown.take();
        let (shown, next_waits) = oneshot::channel();
        self.newest_shown = Some(next_waits);
        let task = tokio::spawn(async move {
            let made = make_frame(
                port,
                &task_name,
                &prompt,
                characters,
                base.as_deref(),
                after,
            )
            .await;
            // The next frame goes on whether this one was shown or failed.
            let _ = shown.send(());
            made
        });
        self.picture = judgement.next_picture;
        self.last_shot = Some(judgement.shot);
        self.since.clear();
        self.peak_percent = 0;
        self.last_start = Some(Instant::now());
        self.in_flight.push_back(InFlight {
            number,
            name,
            started: Instant::now(),
            after_turn: turn,
            task,
        });
    }

    /// Waits for the frames being made and reports them.
    pub(super) async fn finish(&mut self, port: u16, transcript: &mut Vec<String>) {
        self.collect(port, true, transcript).await;
    }

    /// Reports the frames that finished, oldest first (all of them when `wait`), and puts the
    /// scene's title back once none is being made.
    async fn collect(&mut self, port: u16, wait: bool, transcript: &mut Vec<String>) {
        while self
            .in_flight
            .front()
            .is_some_and(|frame| wait || frame.task.is_finished())
        {
            if let Some(frame) = self.in_flight.pop_front() {
                self.report(frame, transcript).await;
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
            Ok(Ok(_)) => {
                self.newest_picture = format!("{}.png", frame.name);
                transcript.push(format!(
                    "  frame {} ({}) shown {waited:.1} s after it started at line {}",
                    frame.number, frame.name, frame.after_turn
                ));
            }
            Ok(Err((step, error))) => transcript.push(format!(
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
