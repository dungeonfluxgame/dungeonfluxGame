//! The dev verbs of the look-ahead worker: `scene lookahead show|pause|resume|clear|target`.
//!
//! They have the shape of the other scene verbs: words in, one typed `LookaheadCommand` out, a usage
//! line on a mistake, a numbered answer. `command::scene` hands every `scene lookahead ...` line to
//! `parse`, and `execute` runs one against a host on a port.

use df_table::lookahead::Expectation;

use super::format::format_reply;
use super::request::{LookaheadReply, LookaheadRequest};
use crate::client;
use crate::command::CommandError;
use crate::dev::DevError;
use crate::limits::MAXIMUM_ECHOED_ANSWER;
use crate::scene_builder::request::{DevStage, SceneReply, SlowStage};
use crate::scene_builder::routes::path;

pub const LOOKAHEAD_USAGE: &str = "scene lookahead show | pause | resume | clear | target <place> [kind=authored_beat|travel_leg|exit_hint] [likelihood=<thousandths>] [fail=<describe|background>] [slow=<describe|background>:<seconds>]";

/// A look-ahead verb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookaheadCommand {
    Show,
    Change(LookaheadRequest),
}

/// The help text of these verbs, to follow the scene builder's in `help`.
pub fn help_text() -> String {
    format!(
        "\n  {LOOKAHEAD_USAGE}\n      the look-ahead worker builds the next scene's description and background on the lowest\n      broker class while the current scene plays; target says which scene the story expects next\n      (the one the next scene build makes), pause stops new builds, clear cancels them; show prints\n      the settings, the expected scene, the book of built scenes and the hit, miss and stale counts\n"
    )
}

fn whole_number(what: &'static str, text: &str) -> Result<u64, CommandError> {
    text.parse::<u64>().map_err(|_| CommandError::BadCount {
        what,
        text: text.to_owned(),
    })
}

fn stage(key: &str, text: &str) -> Result<DevStage, CommandError> {
    DevStage::parse(text)
        .filter(|stage| super::request::is_built_ahead(*stage))
        .ok_or_else(|| CommandError::BadChoice {
            key: key.to_owned(),
            text: text.to_owned(),
            allowed: "describe or background".to_owned(),
        })
}

fn target(words: &[String]) -> Result<LookaheadCommand, CommandError> {
    let [place, options @ ..] = words else {
        return Err(CommandError::Usage(LOOKAHEAD_USAGE));
    };
    let mut kind = Expectation::AuthoredBeat;
    let mut likelihood_permille = None;
    let mut fail = None;
    let mut slow = None;
    for option in options {
        let (key, value) = option
            .split_once('=')
            .ok_or_else(|| CommandError::BadPair(option.clone()))?;
        match key {
            "kind" => {
                kind = Expectation::parse(value).ok_or_else(|| CommandError::BadChoice {
                    key: "kind".to_owned(),
                    text: value.to_owned(),
                    allowed: "authored_beat, travel_leg or exit_hint".to_owned(),
                })?;
            }
            "likelihood" => {
                let permille =
                    u16::try_from(whole_number("the likelihood", value)?).map_err(|_| {
                        CommandError::BadCount {
                            what: "the likelihood",
                            text: value.to_owned(),
                        }
                    })?;
                if likelihood_permille.replace(permille).is_some() {
                    return Err(CommandError::DuplicateKey("likelihood".to_owned()));
                }
            }
            "fail" => {
                if fail.replace(stage("fail", value)?).is_some() {
                    return Err(CommandError::DuplicateKey("fail".to_owned()));
                }
            }
            "slow" => {
                let (name, duration) = value
                    .split_once(':')
                    .ok_or_else(|| CommandError::BadPair(option.clone()))?;
                let chosen = SlowStage {
                    stage: stage("slow", name)?,
                    seconds: whole_number("the seconds", duration)?,
                };
                if slow.replace(chosen).is_some() {
                    return Err(CommandError::DuplicateKey("slow".to_owned()));
                }
            }
            _ => return Err(CommandError::Usage(LOOKAHEAD_USAGE)),
        }
    }
    Ok(LookaheadCommand::Change(LookaheadRequest::Target {
        place: place.clone(),
        kind,
        likelihood_permille,
        fail,
        slow,
    }))
}

/// Parses the words after `scene lookahead`.
pub fn parse(words: &[String]) -> Result<LookaheadCommand, CommandError> {
    match words {
        [word] if word == "show" => Ok(LookaheadCommand::Show),
        [word] if word == "pause" => Ok(LookaheadCommand::Change(LookaheadRequest::Pause)),
        [word] if word == "resume" => Ok(LookaheadCommand::Change(LookaheadRequest::Resume)),
        [word] if word == "clear" => Ok(LookaheadCommand::Change(LookaheadRequest::Clear)),
        [word, rest @ ..] if word == "target" => target(rest),
        _ => Err(CommandError::Usage(LOOKAHEAD_USAGE)),
    }
}

fn unreadable(response: &client::Response, reason: String) -> DevError {
    DevError::UnexpectedAnswer {
        status: response.status,
        body: response.body.chars().take(MAXIMUM_ECHOED_ANSWER).collect(),
        reason: Some(reason),
    }
}

fn read(response: &client::Response) -> Result<LookaheadReply, DevError> {
    let reply: SceneReply<LookaheadReply> = serde_json::from_str(&response.body)
        .map_err(|error| unreadable(response, error.to_string()))?;
    match reply {
        SceneReply::Done { value } => Ok(value),
        SceneReply::Rejected { message, .. } => Err(DevError::Rejected(message)),
    }
}

/// Runs one look-ahead verb against the host on `port` and returns the text to print.
pub async fn execute(port: u16, command: LookaheadCommand) -> Result<String, DevError> {
    let response = match command {
        LookaheadCommand::Show => client::get(port, path::LOOKAHEAD).await,
        LookaheadCommand::Change(request) => {
            let body = serde_json::to_string(&request).map_err(|error| DevError::Unwritable {
                what: "request",
                reason: error.to_string(),
            })?;
            client::post_json(port, path::LOOKAHEAD, &body).await
        }
    }
    .map_err(DevError::Client)?;
    Ok(format_reply(&read(&response)?))
}
