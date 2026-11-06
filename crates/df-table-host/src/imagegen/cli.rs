//! `media generate` on the command line: its grammar, and running it against a host (send the
//! request, read the typed answer, print it). It never prints and never starts a server.
//!
//! `media generate <prompt...> as=NAME [purpose=plate|background|portrait] [aspect=W:H]
//! [wait=true]`. Any word that is not one of the four keys is part of the prompt, in order, so a
//! prompt with commas and colons needs no quoting (a prompt with `as=` or `wait=` inside it does).
//! `media generate status` lists the generations so far.

use std::collections::BTreeSet;

use super::format::{format_job, format_jobs};
use super::limits::{MAXIMUM_ASPECT_PART, MAXIMUM_ASPECT_RATIO, MAXIMUM_NAME_CHARS};
use super::model::{Aspect, GenerateRequestError, ImageRequest, Purpose};
use super::request::{GenerateError, GenerateReply, GenerateRequest, JobState, JobView};
use super::routes::path;
use crate::command::{CommandError, bad_choice};
use crate::dev::DevError;
use crate::dev_client;
use crate::dev_http::DevReply;

pub const MEDIA_GENERATE_USAGE: &str = concat!(
    "media generate <prompt...> as=NAME [purpose=plate|background|portrait] [aspect=W:H] ",
    "[wait=true] | media generate status"
);

const KEYS: [&str; 4] = ["as", "purpose", "aspect", "wait"];

/// A `media generate` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaGenerate {
    /// Make one picture: the body sent, checked here and again by the host.
    Generate(GenerateRequest),
    /// List the generations so far.
    Status,
}

fn option_key(word: &str) -> Option<(&str, &str)> {
    let (key, value) = word.split_once('=')?;
    KEYS.contains(&key).then_some((key, value))
}

fn purpose(text: &str) -> Result<Purpose, CommandError> {
    Purpose::from_key(text)
        .ok_or_else(|| bad_choice("purpose", text, "plate, background or portrait".to_owned()))
}

fn aspect(text: &str) -> Result<Aspect, CommandError> {
    text.parse().map_err(|_| {
        let allowed = format!(
            concat!(
                "W:H such as 16:9, each number from 1 to {}, with neither side more than {} ",
                "times the other"
            ),
            MAXIMUM_ASPECT_PART, MAXIMUM_ASPECT_RATIO
        );
        bad_choice("aspect", text, allowed)
    })
}

/// `true`/`on` or `false`/`off`, as every other switch of the command line reads.
fn wait(text: &str) -> Result<bool, CommandError> {
    match text {
        "true" | "on" => Ok(true),
        "false" | "off" => Ok(false),
        _ => Err(bad_choice("wait", text, "true or false".to_owned())),
    }
}

fn request_error(error: GenerateRequestError) -> CommandError {
    match error {
        GenerateRequestError::EmptyPrompt => CommandError::Usage(MEDIA_GENERATE_USAGE),
        GenerateRequestError::PromptTooLong {
            chars,
            maximum_chars,
        } => bad_choice(
            "prompt",
            &format!("{chars} characters"),
            format!("at most {maximum_chars} characters"),
        ),
        GenerateRequestError::InvalidName { name } => bad_choice(
            "as",
            &name,
            format!("1 to {MAXIMUM_NAME_CHARS} letters, digits, '-' or '_', with no extension"),
        ),
    }
}

/// Reads the words after `media generate`.
pub fn parse(words: &[String]) -> Result<MediaGenerate, CommandError> {
    if matches!(words, [only] if only == "status") {
        return Ok(MediaGenerate::Status);
    }
    let mut prompt_words = Vec::new();
    let mut seen = BTreeSet::new();
    let (mut name, mut purpose_key, mut aspect_text, mut wait_text) = (None, None, None, None);
    for word in words {
        let Some((key, value)) = option_key(word) else {
            prompt_words.push(word.as_str());
            continue;
        };
        if !seen.insert(key) {
            return Err(CommandError::DuplicateKey(key.to_owned()));
        }
        let slot = match key {
            "as" => &mut name,
            "purpose" => &mut purpose_key,
            "aspect" => &mut aspect_text,
            _ => &mut wait_text,
        };
        *slot = Some(value);
    }
    let request = GenerateRequest {
        prompt: prompt_words.join(" "),
        name: name
            .ok_or(CommandError::Usage(MEDIA_GENERATE_USAGE))?
            .to_owned(),
        purpose: purpose_key.map(purpose).transpose()?,
        aspect: aspect_text.map(aspect).transpose()?,
        wait: wait_text.map(wait).transpose()?.unwrap_or(false),
    };
    ImageRequest::new(
        &request.prompt,
        &request.name,
        request.purpose,
        request.aspect,
    )
    .map_err(request_error)?;
    Ok(MediaGenerate::Generate(request))
}

/// Runs one `media generate` command against the host on `port`.
pub async fn run(port: u16, command: MediaGenerate) -> Result<String, DevError> {
    match command {
        MediaGenerate::Status => {
            let jobs: GenerateReply<Vec<JobView>> =
                dev_client::get_json(port, path::GENERATE).await?;
            Ok(format_jobs(&dev_client::reply_value(jobs)?))
        }
        MediaGenerate::Generate(request) => {
            let job: GenerateReply<JobView> =
                dev_client::post_json(port, path::GENERATE, &request).await?;
            match job {
                DevReply::Done { value } => Ok(format_job(&value)),
                DevReply::Rejected { error, message } => {
                    Err(refused_generation(port, error, message).await)
                }
            }
        }
    }
}

/// The failure of a refused generation. A job that ran and ended without a picture is
/// `GenerationEnded`, with its name and how long it ran read from the status; when the status
/// cannot say (it does not answer, or no longer holds the job), it is the refusal as it came.
async fn refused_generation(port: u16, error: GenerateError, message: String) -> DevError {
    let ended = match error.job() {
        Some(number) => {
            let jobs: Result<GenerateReply<Vec<JobView>>, DevError> =
                dev_client::get_json(port, path::GENERATE).await;
            match jobs {
                Ok(DevReply::Done { value }) => value.into_iter().find(|job| job.job == number),
                Ok(DevReply::Rejected { .. }) | Err(_) => None,
            }
        }
        None => None,
    };
    match ended {
        Some(job) if matches!(job.state, JobState::Failed { .. }) => DevError::GenerationEnded {
            job: Box::new(job),
            error,
            message,
        },
        _ => DevError::Rejected {
            error: error.into(),
            message,
        },
    }
}
