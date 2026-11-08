//! `persona converse`: the characters of the shown scene talk to each other.
//!
//! A dev tool over the routes that already exist. Each turn is one `persona say` (so the line is
//! worded from the persona, voiced when asked, and cued with the thinking indicator and the bubble
//! exactly as every other line is); this module only chooses who speaks, tells them what was said
//! and waits for the line to be read or heard. It holds no prompt text beyond the scene's setting
//! and the instruction of the turn, never reads a player's words, and decides nothing about the
//! story: the narrative engine will make the same choices for the production table.
//!
//! The next speaker is whoever the last line was directed at, unless they spoke in the last
//! [`TURNS_BEFORE_AGAIN`] turns, and then the character who has been quiet longest. Who a line is
//! directed at is Jev's decision (`address`, `POST /dev/persona/addressed`), never a match of
//! names; a reading that fails falls back to taking turns.
//!
//! `with=` chooses who talks; `events=` is a file of what happens at which turn (something seen,
//! which the speakers of that turn and the next react to first, or a dev command run before the
//! turn's line); `frames=` lets the picture follow the talk (`frame_watch`). A character is told
//! the others by who they are in the scene, never by a name they have not learned
//! (`known_names` in their state).

use std::time::Duration;

use df_playground::copy::{ReadingPace, caption_dwell_ms, word_count};
use serde::{Deserialize, Serialize};

use super::address::{AddressReply, AddressRequest, PERSONA_ADDRESSED};
use super::frame_watch::{FrameOptions, FrameWatch};
use super::plate::make_plate;
use super::request::{PersonaSummary, SayRequest, Spoken};
use super::routes::path;
use crate::cast::request::CastList;
use crate::cast::routes::path as cast_path;
use crate::dev::DevError;
use crate::dev_client::{get_json, post_json};
use crate::speech::Voiced;

/// How many turns a conversation runs when it is not told.
pub const DEFAULT_TURNS: u32 = 8;
/// The most turns one command runs; far above any use, so only a typo is stopped.
pub const MAXIMUM_TURNS: u32 = 10_000;
/// How many of the latest lines a speaker is told.
const HISTORY_LINES: usize = 20;
/// A character who spoke in the last this many turns lets the quietest one answer instead.
const TURNS_BEFORE_AGAIN: usize = 3;
/// Bytes of one second of the speech adapter's only output format (`mp3_22050_32`).
const VOICE_BYTES_PER_SECOND: u64 = 4_000;
/// The fewest characters at the table.
const MINIMUM_SPEAKERS: usize = 2;
/// A character at the table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Speaker {
    /// The persona id, which is the character's name in the scene.
    pub id: String,
    pub display_name: String,
    pub job: String,
}

/// One line said at the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub speaker: String,
    pub line: String,
}

/// Why a conversation cannot start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConverseError {
    /// Fewer than two characters of the shown scene have a stored persona.
    TooFewSpeakers { found: usize },
    /// The opener is not a character at the table.
    UnknownOpener { opener: String },
    /// A character named to talk is not in the scene with a persona.
    UnknownSpeaker { id: String },
    /// Line `line` of an events file is not `<turn> | <what happens>`.
    InvalidEvent { line: usize },
}

impl std::fmt::Display for ConverseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooFewSpeakers { found } => write!(
                formatter,
                "a conversation needs {MINIMUM_SPEAKERS} characters of the shown scene with a \
                 persona; {found} found"
            ),
            Self::UnknownOpener { opener } => {
                write!(formatter, "'{opener}' is not a character at the table")
            }
            Self::UnknownSpeaker { id } => write!(
                formatter,
                "'{id}' is not a character of the shown scene with a persona"
            ),
            Self::InvalidEvent { line } => write!(
                formatter,
                "line {line} of the events file is not `<turn> | <what happens>`"
            ),
        }
    }
}

impl std::error::Error for ConverseError {}

/// The characters at the table and the rules of who speaks next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    speakers: Vec<Speaker>,
}

impl Table {
    /// The table of `speakers`, left to right, or why there is none.
    pub fn new(speakers: Vec<Speaker>) -> Result<Self, ConverseError> {
        if speakers.len() < MINIMUM_SPEAKERS {
            return Err(ConverseError::TooFewSpeakers {
                found: speakers.len(),
            });
        }
        Ok(Self { speakers })
    }

    /// The characters, left to right.
    pub(super) fn speakers(&self) -> &[Speaker] {
        &self.speakers
    }

    /// The speaker called `id`, or the first when none is named.
    pub fn opener(&self, id: Option<&str>) -> Result<&Speaker, ConverseError> {
        match id {
            None => Ok(&self.speakers[0]),
            Some(id) => self
                .speakers
                .iter()
                .find(|speaker| speaker.id == id)
                .ok_or_else(|| ConverseError::UnknownOpener {
                    opener: id.to_owned(),
                }),
        }
    }

    /// Who speaks after `history` (which must not be empty): the character `addressed` (the id the
    /// last line was directed at), unless they spoke in the last [`TURNS_BEFORE_AGAIN`] turns, and
    /// then whoever has been quiet longest. It is never the last speaker.
    pub fn next_speaker(&self, history: &[Turn], addressed: Option<&str>) -> &Speaker {
        let last = history.last().map_or("", |turn| turn.speaker.as_str());
        let silent_for = |speaker: &Speaker| {
            history
                .iter()
                .rev()
                .position(|turn| turn.speaker == speaker.id)
                .unwrap_or(usize::MAX)
        };
        let named = addressed
            .and_then(|id| self.speakers.iter().find(|speaker| speaker.id == id))
            .filter(|speaker| speaker.id != last)
            .filter(|speaker| silent_for(speaker) >= TURNS_BEFORE_AGAIN);
        named.unwrap_or_else(|| {
            // The quietest, the one seated furthest left among equals.
            self.speakers
                .iter()
                .enumerate()
                .filter(|(_, speaker)| speaker.id != last)
                .max_by_key(|(seat, speaker)| (silent_for(speaker), std::cmp::Reverse(*seat)))
                .map_or(&self.speakers[0], |(_, speaker)| speaker)
        })
    }

    /// The latest [`HISTORY_LINES`] lines of `history` as "id: line", oldest first: the line's
    /// `recent`. The line pipeline shows each label as the hearer knows that person (`handles`):
    /// an id may carry a name.
    pub(super) fn recent_lines(&self, history: &[Turn]) -> Vec<String> {
        let skip = history.len().saturating_sub(HISTORY_LINES);
        history[skip..]
            .iter()
            .map(|turn| format!("{}: {}", turn.speaker, turn.line))
            .collect()
    }

    /// What a speaker is told for their turn: that everyone can hear, the topic when there is
    /// one, what is happening, and the instruction. Who is here and the latest lines are not in
    /// this text: they go as the line's `present`, `others` and `recent`, where someone whose name
    /// the speaker does not know is a handle (an id may carry a name: `mara_adventurer`). No
    /// place is named either: the place is the world's, not a fixed inn.
    pub fn situation(topic: Option<&str>, happening: Option<&str>, history: &[Turn]) -> String {
        let mut text = "Everyone here can hear everyone.".to_owned();
        if let Some(topic) = topic {
            text.push_str(&format!(" What is on the table: {topic}"));
        }
        if let Some(happening) = happening {
            text.push_str(&format!(
                " Happening right now, as you speak: {happening} Your line reacts to it as it \
                 happens."
            ));
        }
        if history.is_empty() {
            text.push_str(" You speak first: open the conversation in your own voice.");
            return text;
        }
        let first = match happening {
            Some(happening) => format!("React first to what is happening right now ({happening})"),
            None => "React to the last thing said (the newest of recent)".to_owned(),
        };
        text.push_str(&format!(
            " {first}, in your own voice; you may address someone by a name you know. One or \
             two short sentences. Do not repeat what was already said."
        ));
        text
    }
}

/// How long to wait before the next line so this one can be read or heard: the caption dwell of
/// its words, or the length of its recording when that is longer.
pub fn pause_after(line: &str, voice: Option<&Voiced>) -> Duration {
    let read_ms = caption_dwell_ms(word_count(line), ReadingPace::Normal);
    let heard_ms = match voice {
        Some(Voiced::Played { bytes, .. }) => bytes.saturating_mul(1_000) / VOICE_BYTES_PER_SECOND,
        Some(Voiced::Failed { .. }) | None => 0,
    };
    Duration::from_millis(read_ms.max(heard_ms))
}

/// What one `persona converse` command asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConverseRequest {
    pub turns: u32,
    pub opener: Option<String>,
    pub topic: Option<String>,
    pub model: Option<String>,
    pub speak: bool,
    /// Wait after each line so it can be read or heard before the next replaces its bubble.
    pub pace: bool,
    /// The name of a new tv backdrop to make from the talk once it is over, and put behind the
    /// scene (see `plate`).
    pub plate: Option<String>,
    /// New frames during the talk, when the picture would visibly change (see `frame_watch`).
    pub frames: Option<FrameOptions>,
    /// Who talks: these characters of the scene, by id; everyone with a persona when empty.
    pub with: Vec<String>,
    /// A file of what happens while they talk, one `<turn> | <what happens>` a line.
    pub events: Option<std::path::PathBuf>,
}

/// Something that happens at a turn of the talk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneEvent {
    pub turn: u32,
    pub happens: Happening,
}

/// What an event is: something seen at the table (the speakers of that turn and the next react to
/// it, and the frame judge counts it as seen), or a dev command run before that turn's line (a
/// stance switched on mid-scene), as its words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Happening {
    Seen(String),
    Command(Vec<String>),
}

impl SceneEvent {
    /// What is seen, when the event is something seen.
    pub fn seen(&self) -> Option<&str> {
        match &self.happens {
            Happening::Seen(what) => Some(what),
            Happening::Command(_) => None,
        }
    }
}

/// The events of an events file: one `<turn> | <what happens>` or `<turn> | ! <dev command>` a
/// line, `#` lines and blank lines skipped, in the order of their turns.
pub fn parse_events(text: &str) -> Result<Vec<SceneEvent>, ConverseError> {
    let mut events = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let invalid = || ConverseError::InvalidEvent { line: index + 1 };
        let (turn, what) = line.split_once('|').ok_or_else(invalid)?;
        let turn = turn.trim().parse::<u32>().map_err(|_| invalid())?;
        let what = what.trim();
        if what.is_empty() {
            return Err(invalid());
        }
        let happens = match what.strip_prefix('!') {
            Some(command) => Happening::Command(
                crate::command::split_words(command.trim())
                    .ok()
                    .filter(|words| !words.is_empty() && !starts_a_conversation(words))
                    .ok_or_else(invalid)?,
            ),
            None => Happening::Seen(what.to_owned()),
        };
        events.push(SceneEvent { turn, happens });
    }
    events.sort_by_key(|event| event.turn);
    Ok(events)
}

/// Whether `words` is a `persona converse` command: an events file may switch stances and set
/// state, but never start a conversation of its own, which could run the same file again.
fn starts_a_conversation(words: &[String]) -> bool {
    matches!(words, [verb, sub, ..] if verb == "persona" && sub == "converse")
}

/// The speakers of the shown scene: its named characters that have a stored persona.
pub(super) fn speakers_of(cast: CastList) -> Vec<Speaker> {
    cast.members
        .iter()
        .filter_map(|member| {
            cast.personas
                .iter()
                .find(|summary| summary.id == member.name)
                .map(speaker_of)
        })
        .collect()
}

fn speaker_of(summary: &PersonaSummary) -> Speaker {
    Speaker {
        id: summary.id.clone(),
        display_name: summary.display_name.clone(),
        job: summary.job.clone(),
    }
}

/// One `persona say` against the host on `port`.
async fn say_once(port: u16, say: &SayRequest) -> Result<Spoken, DevError> {
    super::cli::reply_value(post_json(port, path::PERSONA_SAY, say).await?)
}

/// Asks Jev (`POST /dev/persona/addressed`) who `line`, said by `speaker`, is directed at: the id
/// of
/// a character at the table, or none. A refusal is an unexpected answer carrying the route's key.
async fn addressed_once(
    port: u16,
    table: &Table,
    speaker: &Speaker,
    line: &str,
    history: &[Turn],
    model: Option<&str>,
) -> Result<Option<String>, DevError> {
    let request = AddressRequest {
        line: line.to_owned(),
        speaker: speaker.id.clone(),
        speakers: table.speakers().to_vec(),
        recent: table.recent_lines(history),
        model: model.map(str::to_owned),
    };
    let reply: AddressReply = post_json(port, PERSONA_ADDRESSED, &request).await?;
    Ok(crate::dev_client::reply_value(reply)?.addressed)
}

/// `persona addressed`: whom `line`, said by `speaker` in the shown scene after `recent`, is
/// directed at, as the addressee judge reads it (a model call against the closed list of the
/// scene's speakers): their id, or `nobody`. The market runner uses it so the one a line is put
/// to answers it.
pub(super) async fn who_is_addressed(
    port: u16,
    speaker: String,
    line: String,
    recent: Vec<String>,
    model: Option<String>,
) -> Result<String, DevError> {
    let cast: CastList = super::cli::reply_value(get_json(port, cast_path::CAST).await?)?;
    let request = AddressRequest {
        line,
        speaker,
        speakers: speakers_of(cast),
        recent,
        model,
    };
    let reply: AddressReply = post_json(port, PERSONA_ADDRESSED, &request).await?;
    Ok(crate::dev_client::reply_value(reply)?
        .addressed
        .unwrap_or_else(|| "nobody".to_owned()))
}

/// Runs the conversation against the host on `port` and gives the transcript. A turn that fails
/// stops it with the lines said so far (`DevError::ConverseStopped`).
pub async fn converse(port: u16, request: ConverseRequest) -> Result<String, DevError> {
    let cast: CastList = super::cli::reply_value(get_json(port, cast_path::CAST).await?)?;
    let picture = cast.picture.clone();
    let mut speakers = speakers_of(cast);
    if !request.with.is_empty() {
        if let Some(missing) = request
            .with
            .iter()
            .find(|id| !speakers.iter().any(|speaker| speaker.id == **id))
        {
            return Err(DevError::Converse(ConverseError::UnknownSpeaker {
                id: missing.clone(),
            }));
        }
        speakers.retain(|speaker| request.with.contains(&speaker.id));
    }
    let table = Table::new(speakers).map_err(DevError::Converse)?;
    let events = match &request.events {
        Some(path) => {
            parse_events(&crate::dev::read_text_file(path).await?).map_err(DevError::Converse)?
        }
        None => Vec::new(),
    };
    let mut frames = request
        .frames
        .clone()
        .map(|options| FrameWatch::new(options, table.speakers(), &picture));
    let mut speaker = table
        .opener(request.opener.as_deref())
        .map_err(DevError::Converse)?
        .clone();
    let mut history: Vec<Turn> = Vec::new();
    let mut transcript = Vec::new();
    for turn in 0..request.turns {
        // An event is happening on its turn and the one after it, so the answer reacts to it too.
        for event in events.iter().filter(|event| event.turn == turn) {
            if let Happening::Command(words) = &event.happens {
                // The command line is the dev CLI's own, so this is a call back into it.
                let ran = Box::pin(crate::dev::run_dev(port, words)).await;
                transcript.push(match ran {
                    Ok(text) => format!(
                        "  ! {}: {}",
                        words.join(" "),
                        text.lines().next().unwrap_or("ok")
                    ),
                    Err(error) => format!("  ! {} failed: {error}", words.join(" ")),
                });
            }
        }
        let happening = events
            .iter()
            .rev()
            .filter(|event| event.seen().is_some())
            .find(|event| event.turn <= turn && turn <= event.turn + 1);
        if let (Some(watch), Some(event)) = (frames.as_mut(), happening)
            && event.turn == turn
            && let Some(seen) = event.seen()
        {
            watch.note_event(seen);
        }
        let say = SayRequest {
            character: speaker.id.clone(),
            situation: Table::situation(
                request.topic.as_deref(),
                happening.and_then(SceneEvent::seen),
                &history,
            ),
            model: request.model.clone(),
            speak: request.speak,
            recent: table.recent_lines(&history),
            asked_by: None,
            intent: None,
        };
        let spoken = match say_once(port, &say).await {
            Ok(spoken) => spoken,
            Err(error) => {
                return Err(DevError::ConverseStopped {
                    done: transcript,
                    turn,
                    error: Box::new(error),
                });
            }
        };
        transcript.push(format!(
            "[{turn}] {}: {}",
            speaker.display_name, spoken.said.line
        ));
        if let Some(Voiced::Failed { error }) = &spoken.voice {
            transcript.push(format!("  voice failed: {}", error.key()));
        }
        // Who the line is directed at is asked while it is read or heard, so the pace is the
        // longer of the two and not their sum. Nobody follows the last line.
        let pause = request
            .pace
            .then(|| pause_after(&spoken.said.line, spoken.voice.as_ref()));
        let line = spoken.said.line;
        let following = turn + 1 < request.turns;
        // The frame judge reads the line while it is read too, next to who it is directed at.
        let (addressed, (), judged) = tokio::join!(
            async {
                if following {
                    addressed_once(
                        port,
                        &table,
                        &speaker,
                        &line,
                        &history,
                        request.model.as_deref(),
                    )
                    .await
                } else {
                    Ok(None)
                }
            },
            async {
                if let Some(pause) = pause {
                    tokio::time::sleep(pause).await;
                }
            },
            async {
                match frames.as_mut() {
                    Some(watch) => {
                        watch
                            .after_line(port, &speaker, &line, &mut transcript)
                            .await
                    }
                    None => None,
                }
            }
        );
        if let (Some(watch), Some(judged)) = (frames.as_mut(), judged) {
            watch.consider(port, turn, judged, &mut transcript).await;
        }
        // Who the line was directed at only chooses the next speaker: a reading that failed (a
        // refused answer included) falls back to taking turns, and the talk goes on.
        let addressed = match addressed {
            Ok(addressed) => addressed,
            Err(error) => {
                transcript.push(format!(
                    "  addressee not read after line {turn} ({error}); next speaker by turn"
                ));
                None
            }
        };
        history.push(Turn {
            speaker: speaker.id.clone(),
            line,
        });
        speaker = table.next_speaker(&history, addressed.as_deref()).clone();
    }
    if let Some(watch) = frames.as_mut() {
        watch.finish(port, &mut transcript).await;
    }
    if let Some(name) = &request.plate {
        match make_plate(port, name, &table, &history).await {
            Ok(summary) => transcript.push(summary),
            Err((step, error)) => {
                return Err(DevError::PlateStopped {
                    done: transcript,
                    step,
                    error,
                });
            }
        }
    }
    Ok(transcript.join("\n"))
}
