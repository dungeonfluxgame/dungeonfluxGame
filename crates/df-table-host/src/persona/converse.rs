//! `persona converse`: the characters of the shown scene talk to each other.
//!
//! A dev tool over the routes that already exist. Each turn is one `persona say` (so the line is
//! worded from the persona, voiced when asked, and cued with the thinking indicator and the bubble
//! exactly as every other line is); this module only chooses who speaks, tells them what was said
//! and waits for the line to be read or heard. It holds no prompt text beyond the scene's setting
//! and the instruction of the turn, never reads a player's words, and decides nothing about the
//! story: the narrative engine will make the same choices for the production table.
//!
//! The next speaker is whoever the last line addressed by name, unless they spoke in the last
//! [`TURNS_BEFORE_AGAIN`] turns, and then the character who has been quiet longest.

use std::time::Duration;

use df_playground::copy::{ReadingPace, caption_dwell_ms, word_count};

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
const HISTORY_LINES: usize = 6;
/// A character who spoke in the last this many turns lets the quietest one answer instead.
const TURNS_BEFORE_AGAIN: usize = 3;
/// Bytes of one second of the speech adapter's only output format (`mp3_22050_32`).
const VOICE_BYTES_PER_SECOND: u64 = 4_000;
/// The fewest characters at the table.
const MINIMUM_SPEAKERS: usize = 2;
/// Words that start a display name without being the character's own name.
const TITLES: [&str; 13] = [
    "the", "a", "an", "old", "young", "captain", "mother", "father", "sir", "lady", "lord",
    "master", "doctor",
];

/// A character at the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speaker {
    /// The persona id, which is the character's name in the scene.
    pub id: String,
    pub display_name: String,
    pub job: String,
}

impl Speaker {
    /// The names the others call this character by, as lowercase words split the way a line is
    /// split: the id's words ("braided woman"), the display name without a leading article or
    /// title ("braided woman"), and its first word when that is a given name ("aldric" for "Aldric
    /// Venn"; a title such as "Captain" or "Old" is not one).
    fn names(&self) -> Vec<Vec<String>> {
        let tokens = |text: &str| -> Vec<String> {
            text.to_lowercase()
                .split(|character: char| !character.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .map(str::to_owned)
                .collect()
        };
        let plain = |text: &str| {
            let mut words = tokens(text);
            if words.len() > 1 && TITLES.contains(&words[0].as_str()) {
                words.remove(0);
            }
            words
        };
        let mut names = vec![plain(&self.id), plain(&self.display_name)];
        let all = tokens(&self.display_name);
        if all.len() > 1 && !TITLES.contains(&all[0].as_str()) {
            names.push(vec![all[0].clone()]);
        }
        names.retain(|name| !name.is_empty());
        names.sort();
        names.dedup();
        names
    }
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

    /// Who speaks after `history` (which must not be empty): the character the last line named,
    /// unless they spoke in the last [`TURNS_BEFORE_AGAIN`] turns, and then whoever has been quiet
    /// longest. It is never the last speaker.
    pub fn next_speaker(&self, history: &[Turn]) -> &Speaker {
        let last = history.last().map_or("", |turn| turn.speaker.as_str());
        let silent_for = |speaker: &Speaker| {
            history
                .iter()
                .rev()
                .position(|turn| turn.speaker == speaker.id)
                .unwrap_or(usize::MAX)
        };
        let named = self
            .addressed(history.last().map_or("", |turn| turn.line.as_str()), last)
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

    /// The character `line` names by one of their names as a whole word, other than `by`; the
    /// first one named in the line. The line is a character's own, never a player's words.
    fn addressed(&self, line: &str, by: &str) -> Option<&Speaker> {
        let lower = line.to_lowercase();
        let words: Vec<&str> = lower
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect();
        // Where, and by how long a name, the speaker is named: the earliest mention wins, and of
        // two at the same place the longer name ("Aldric Stone" over "Aldric").
        let mentioned = |speaker: &Speaker| {
            speaker
                .names()
                .iter()
                .filter_map(|name| {
                    words
                        .windows(name.len())
                        .position(|window| {
                            window
                                .iter()
                                .zip(name)
                                .all(|(word, part)| *word == part.as_str())
                        })
                        .map(|at| (at, std::cmp::Reverse(name.len())))
                })
                .min()
        };
        self.speakers
            .iter()
            .enumerate()
            .filter(|(_, speaker)| speaker.id != by)
            .filter_map(|(seat, speaker)| mentioned(speaker).map(|key| (key, seat, speaker)))
            .min_by_key(|(key, seat, _)| (*key, *seat))
            .map(|(_, _, speaker)| speaker)
    }

    /// What `speaker` is told for their turn: the setting, the topic when there is one, the latest
    /// lines, and the instruction.
    pub fn situation(&self, speaker: &Speaker, topic: Option<&str>, history: &[Turn]) -> String {
        let people: Vec<String> = self
            .speakers
            .iter()
            .map(|other| format!("{} ({})", other.display_name, other.job))
            .collect();
        let mut text = format!(
            "A table at the inn, everyone can hear everyone. Seated here: {}. You are {}.",
            people.join("; "),
            speaker.display_name
        );
        if let Some(topic) = topic {
            text.push_str(&format!(" What is on the table: {topic}"));
        }
        if history.is_empty() {
            text.push_str(" You speak first: open the conversation in your own voice.");
            return text;
        }
        text.push_str(" What was said, newest last:");
        let skip = history.len().saturating_sub(HISTORY_LINES);
        for turn in &history[skip..] {
            let name = self
                .speakers
                .iter()
                .find(|other| other.id == turn.speaker)
                .map_or(turn.speaker.as_str(), |other| other.display_name.as_str());
            text.push_str(&format!("\n{name}: {}", turn.line));
        }
        text.push_str(
            "\nReact to the last thing said, in your own voice; you may address someone by name. \
             One or two short sentences. Do not repeat what was already said.",
        );
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
}

/// The speakers of the shown scene: its named characters that have a stored persona.
fn speakers_of(cast: CastList) -> Vec<Speaker> {
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

/// Runs the conversation against the host on `port` and gives the transcript. A turn that fails
/// stops it with the lines said so far (`DevError::ConverseStopped`).
pub async fn converse(port: u16, request: ConverseRequest) -> Result<String, DevError> {
    let cast: CastList = super::cli::reply_value(get_json(port, cast_path::CAST).await?)?;
    let table = Table::new(speakers_of(cast)).map_err(DevError::Converse)?;
    let mut speaker = table
        .opener(request.opener.as_deref())
        .map_err(DevError::Converse)?
        .clone();
    let mut history: Vec<Turn> = Vec::new();
    let mut transcript = Vec::new();
    for turn in 0..request.turns {
        let say = SayRequest {
            character: speaker.id.clone(),
            situation: table.situation(&speaker, request.topic.as_deref(), &history),
            model: request.model.clone(),
            speak: request.speak,
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
        if request.pace {
            tokio::time::sleep(pause_after(&spoken.said.line, spoken.voice.as_ref())).await;
        }
        history.push(Turn {
            speaker: speaker.id.clone(),
            line: spoken.said.line,
        });
        speaker = table.next_speaker(&history).clone();
    }
    Ok(transcript.join("\n"))
}
