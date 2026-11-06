//! The grammar of `rules check <skill> <seat> [dc=N]` and its call to the host.

use crate::adapters::action_book::DifficultyClass;
use crate::adapters::hero_profile::Skill;
use crate::command::CommandError;
use crate::dev::DevError;
use crate::dev_client;
use crate::engine::routes::path;

use super::format::format_rules_check;
use super::request::{RulesCheckReply, RulesCheckRequest};

pub const RULES_CHECK_USAGE: &str = "rules check <skill> <seat> [dc=N]";

/// Parses the words after `rules`.
pub fn parse(words: &[String]) -> Result<RulesCheckRequest, CommandError> {
    let usage = || CommandError::Usage(RULES_CHECK_USAGE);
    let [verb, skill, actor, rest @ ..] = words else {
        return Err(usage());
    };
    if verb != "check" {
        return Err(usage());
    }
    let skill = Skill::from_key(skill).ok_or_else(|| CommandError::BadChoice {
        key: "the skill".to_owned(),
        text: skill.clone(),
        allowed: Skill::ALL.map(Skill::key).join(", "),
    })?;
    let actor = actor.parse::<u64>().map_err(|_| CommandError::BadCount {
        what: "the seat",
        text: actor.clone(),
    })?;
    let dc = match rest {
        [] => None,
        [pair] => {
            let text = pair.strip_prefix("dc=").ok_or_else(usage)?;
            let invalid = || CommandError::BadCount {
                what: "the difficulty class",
                text: text.to_owned(),
            };
            let value = text.parse::<u8>().map_err(|_| invalid())?;
            Some(DifficultyClass::new(value).map_err(|_| invalid())?)
        }
        _ => return Err(usage()),
    };
    Ok(RulesCheckRequest { skill, actor, dc })
}

/// Sends `request` to the host on `port` and reads its typed reply, a refusal included.
pub async fn ask(port: u16, request: &RulesCheckRequest) -> Result<RulesCheckReply, DevError> {
    dev_client::post_json(port, path::RULES_CHECK, request).await
}

/// Runs one check against the host on `port` and returns the text to print.
pub async fn run(port: u16, request: &RulesCheckRequest) -> Result<String, DevError> {
    dev_client::reply_value(ask(port, request).await?).map(|value| format_rules_check(&value))
}
