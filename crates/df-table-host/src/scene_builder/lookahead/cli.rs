//! The dev verbs of the look-ahead worker: `scene lookahead show|pause|resume|clear|set|target`.
//!
//! They have the shape of the other scene verbs: words in, one typed `LookaheadCommand` out, a
//! usage line on a mistake, a numbered answer. `command::scene` hands every `scene lookahead ...`
//! line to `parse`, and `run` runs one against a host on a port.

use df_table::lookahead::Expectation;

use super::format::format_reply;
use super::request::{LookaheadReply, LookaheadRequest, LookaheadSettingName, is_built_ahead};
use crate::command::CommandError;
use crate::dev::DevError;
use crate::dev_client::{get_json, post_json};
use crate::scene_builder::cli::{dev_stage, place, reply_value, slow_stage, whole_number};
use crate::scene_builder::routes::path;

pub const LOOKAHEAD_USAGE: &str = concat!(
    "scene lookahead show | pause | resume | clear",
    " | set <enabled|depth|start_s|deadline_s|book_age_s> <value>",
    " | target <place> [kind=authored_beat|travel_leg|exit_hint] [likelihood=<thousandths>]",
    " [fail=<describe|background>] [slow=<describe|background>:<seconds>]",
);

/// The stages a look-ahead build makes, for a refusal of any other.
const BUILT_AHEAD: &str = "describe or background";

/// A look-ahead verb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookaheadCommand {
    Show,
    Change(LookaheadRequest),
}

/// The help text of these verbs, to follow the scene builder's in `help`.
pub fn help_text() -> String {
    format!(
        concat!(
            "\n  {usage}\n",
            "      the look-ahead worker builds the next scene's description and background\n",
            "      on the lowest broker class while the current scene plays; target says\n",
            "      which scene the story expects next (the one the next scene build makes),\n",
            "      pause stops new builds, clear cancels them; set changes a lookahead_*\n",
            "      setting (enabled takes on or off, the others a whole number); show prints\n",
            "      the settings, the expected scene, the book of built scenes and the hit,\n",
            "      miss and stale counts\n",
        ),
        usage = LOOKAHEAD_USAGE,
    )
}

fn target(words: &[String]) -> Result<LookaheadCommand, CommandError> {
    let [name, options @ ..] = words else {
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
                kind = Expectation::from_key(value).ok_or_else(|| CommandError::BadChoice {
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
                let stage = dev_stage("fail", value, is_built_ahead, BUILT_AHEAD)?;
                if fail.replace(stage).is_some() {
                    return Err(CommandError::DuplicateKey("fail".to_owned()));
                }
            }
            "slow" => {
                let chosen = slow_stage(option, value, is_built_ahead, BUILT_AHEAD)?;
                if slow.replace(chosen).is_some() {
                    return Err(CommandError::DuplicateKey("slow".to_owned()));
                }
            }
            _ => return Err(CommandError::Usage(LOOKAHEAD_USAGE)),
        }
    }
    Ok(LookaheadCommand::Change(LookaheadRequest::Target {
        place: place(name)?,
        kind,
        likelihood_permille,
        fail,
        slow,
    }))
}

fn set(words: &[String]) -> Result<LookaheadCommand, CommandError> {
    let [name, value] = words else {
        return Err(CommandError::Usage(LOOKAHEAD_USAGE));
    };
    let setting = LookaheadSettingName::from_key(name).ok_or_else(|| CommandError::BadChoice {
        key: "setting".to_owned(),
        text: name.clone(),
        allowed: "enabled, depth, start_s, deadline_s or book_age_s".to_owned(),
    })?;
    let value = if setting == LookaheadSettingName::Enabled {
        match value.as_str() {
            "on" => 1,
            "off" => 0,
            _ => {
                return Err(CommandError::BadChoice {
                    key: "enabled".to_owned(),
                    text: value.clone(),
                    allowed: "on or off".to_owned(),
                });
            }
        }
    } else {
        whole_number("the value", value)?
    };
    Ok(LookaheadCommand::Change(LookaheadRequest::Set {
        setting,
        value,
    }))
}

/// Parses the words after `scene lookahead`.
pub fn parse(words: &[String]) -> Result<LookaheadCommand, CommandError> {
    match words {
        [word] if word == "show" => Ok(LookaheadCommand::Show),
        [word] if word == "pause" => Ok(LookaheadCommand::Change(LookaheadRequest::Pause)),
        [word] if word == "resume" => Ok(LookaheadCommand::Change(LookaheadRequest::Resume)),
        [word] if word == "clear" => Ok(LookaheadCommand::Change(LookaheadRequest::Clear)),
        [word, rest @ ..] if word == "set" => set(rest),
        [word, rest @ ..] if word == "target" => target(rest),
        _ => Err(CommandError::Usage(LOOKAHEAD_USAGE)),
    }
}

/// Runs one look-ahead verb against the host on `port` and returns the text to print.
pub async fn run(port: u16, command: LookaheadCommand) -> Result<String, DevError> {
    let reply: LookaheadReply = match command {
        LookaheadCommand::Show => reply_value(get_json(port, path::LOOKAHEAD).await?)?,
        LookaheadCommand::Change(request) => {
            reply_value(post_json(port, path::LOOKAHEAD, &request).await?)?
        }
    };
    Ok(format_reply(&reply))
}
