//! The grammar of `media generate`: a prompt in words, then `key=value` options, parsed into one
//! typed command. It owns the words and the usage line and nothing else: the sending is in
//! `imagegen::cli`.
//!
//! `media generate <prompt...> as=NAME [purpose=plate|background|portrait] [aspect=W:H]
//! [wait=true]`. Any word that is not one of the four keys is part of the prompt, in order, so a
//! prompt with commas and colons needs no quoting (a prompt with `as=` or `wait=` inside it does).
//! `media generate status` lists the generations so far.

use std::collections::BTreeSet;

use super::{Command, CommandError, bad_choice, truth};
use crate::imagegen::{ImageRequest, RequestProblem};

pub(super) const MEDIA_GENERATE_USAGE: &str = "media generate <prompt...> as=NAME [purpose=plate|background|portrait] [aspect=W:H] [wait=true] | media generate status";

const KEYS: [&str; 4] = ["as", "purpose", "aspect", "wait"];

/// A `media generate` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaGenerate {
    /// Make one picture. `purpose` and `aspect` are the words given, checked here and again by
    /// the host.
    Generate {
        prompt: String,
        name: String,
        purpose: Option<String>,
        aspect: Option<String>,
        wait: bool,
    },
    /// List the generations so far.
    Status,
}

fn option_key(word: &str) -> Option<(&str, &str)> {
    let (key, value) = word.split_once('=')?;
    KEYS.contains(&key).then_some((key, value))
}

pub(super) fn parse(words: &[String]) -> Result<Command, CommandError> {
    if matches!(words, [only] if only == "status") {
        return Ok(Command::MediaGenerate(MediaGenerate::Status));
    }
    let mut prompt_words = Vec::new();
    let mut seen = BTreeSet::new();
    let (mut name, mut purpose, mut aspect, mut wait) = (None, None, None, false);
    for word in words {
        let Some((key, value)) = option_key(word) else {
            prompt_words.push(word.as_str());
            continue;
        };
        if !seen.insert(key) {
            return Err(CommandError::DuplicateKey(key.to_owned()));
        }
        match key {
            "as" => name = Some(value.to_owned()),
            "purpose" => purpose = Some(value.to_owned()),
            "aspect" => aspect = Some(value.to_owned()),
            _ => wait = truth("wait", value)?,
        }
    }
    let name = name.ok_or(CommandError::Usage(MEDIA_GENERATE_USAGE))?;
    let prompt = prompt_words.join(" ");
    if let Err(problem) = ImageRequest::new(&prompt, &name, purpose.as_deref(), aspect.as_deref()) {
        return Err(match problem {
            RequestProblem::EmptyPrompt => CommandError::Usage(MEDIA_GENERATE_USAGE),
            RequestProblem::PromptTooLong { chars } => bad_choice(
                "prompt",
                &format!("{chars} characters"),
                "at most 16000 characters",
            ),
            RequestProblem::BadName { name } => bad_choice(
                "as",
                &name,
                "1 to 64 letters, digits, '-' or '_', with no extension",
            ),
            RequestProblem::BadPurpose { text } => {
                bad_choice("purpose", &text, "plate, background or portrait")
            }
            RequestProblem::BadAspect { text } => bad_choice(
                "aspect",
                &text,
                "W:H such as 16:9, with neither side more than four times the other",
            ),
        });
    }
    Ok(Command::MediaGenerate(MediaGenerate::Generate {
        prompt,
        name,
        purpose,
        aspect,
        wait,
    }))
}
