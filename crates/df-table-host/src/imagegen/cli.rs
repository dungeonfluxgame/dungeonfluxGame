//! `media generate` on the command line: its grammar, and running it against a host (send the
//! request, read the typed answer, print it). It never prints and never starts a server.
//!
//! `media generate <prompt...> as=NAME [purpose=plate|background|portrait] [aspect=W:H]
//! [wait=true] [base=<picture>] [refs=<picture>,<picture>...] [look=house|none]
//! [priority=normal|emergency]`. `priority=emergency` puts the picture first in a backend's own
//! queue (Forge's waiting line); `refs=` names
//! pictures of the media library (character reference sheets) the new picture follows for the
//! people's looks. Any word that is not one of the keys is
//! part of the prompt, in order, so a prompt with commas and colons needs no quoting (a prompt
//! with `as=` or `wait=` inside it does). `look=house` (the default) sends the prompt inside the
//! house look the scene frames use (`BriefStyle`), with what it must not show, so a plate made by
//! hand matches them; `look=none` sends it as written.
//! `media generate status` lists the generations so far.
//!
//! `media backend` shows the image backend new jobs start on and what it takes; `media backend
//! codex-cli` and `media backend forge [url=<origin>] [project=<id>] [size=<W>x<H>] [steps=<N>]`
//! switch it while the host runs (jobs already running finish on the backend they started on).
//! `size=` and `steps=` make every Forge picture that size (whatever shape was asked) and take
//! that many steps, until the next `media backend` switch; Codex takes neither.

use std::collections::BTreeSet;

use df_providers::FORGE_IMAGE_MAXIMUM_STEPS;

use super::format::{format_backend, format_job, format_jobs};
use super::limits::{MAXIMUM_ASPECT_PART, MAXIMUM_ASPECT_RATIO, MAXIMUM_NAME_CHARS};
use super::model::{
    Aspect, GenerateRequestError, InferenceSteps, MAXIMUM_SIZE_SIDE_PX, PictureSize, Priority,
    Purpose,
};
use super::request::{
    BackendSwitch, BackendView, GenerateError, GenerateReply, GenerateRequest, JobState, JobView,
    ReferenceCap,
};
use super::routes::path;
use crate::command::{CommandError, bad_choice};
use crate::dev::DevError;
use crate::dev_client;
use crate::dev_http::DevReply;
use crate::movement::brief::BriefStyle;

pub const MEDIA_GENERATE_USAGE: &str = concat!(
    "media generate <prompt...> as=NAME [purpose=plate|background|portrait] [aspect=W:H] ",
    "[wait=true] [base=<picture to edit, such as bar.png>] ",
    "[refs=<reference pictures, such as ref-warrior.png,ref-mara_adventurer.png>] ",
    "[look=house|none] [priority=normal|emergency] | ",
    "media generate status"
);

pub const MEDIA_BACKEND_USAGE: &str = concat!(
    "media backend [codex-cli | forge [url=<loopback origin>] [project=<forge project id>] ",
    "[size=<W>x<H>] [steps=<N>] | refs=<N>]"
);

const KEYS: [&str; 8] = [
    "as", "purpose", "aspect", "wait", "base", "refs", "look", "priority",
];

/// `prompt` as `look` asks, and what the picture must not show: inside the house look (the
/// default), or as written with nothing to avoid.
pub(crate) fn with_look(
    prompt: String,
    look: Option<&str>,
) -> Result<(String, Vec<String>), CommandError> {
    match look {
        None | Some("house") => {
            let style = BriefStyle::default();
            Ok((
                format!("{} this scene: {prompt} {}", style.direction, style.writing),
                style.avoid,
            ))
        }
        Some("none") => Ok((prompt, Vec::new())),
        Some(other) => Err(bad_choice("look", other, "house or none".to_owned())),
    }
}

/// A `media generate` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaGenerate {
    /// Make one picture: the body sent, checked here and again by the host.
    Generate(GenerateRequest),
    /// List the generations so far.
    Status,
    /// Show the backend new jobs start on.
    Backend,
    /// Switch the backend new jobs start on.
    SwitchBackend(BackendSwitch),
    /// Set the soft cap on the pictures a plate request the host builds itself carries.
    ReferenceCap(ReferenceCap),
}

fn option_key(word: &str) -> Option<(&str, &str)> {
    let (key, value) = word.split_once('=')?;
    KEYS.contains(&key).then_some((key, value))
}

fn purpose(text: &str) -> Result<Purpose, CommandError> {
    Purpose::from_key(text)
        .ok_or_else(|| bad_choice("purpose", text, "plate, background or portrait".to_owned()))
}

fn priority(text: &str) -> Result<Priority, CommandError> {
    Priority::from_key(text)
        .ok_or_else(|| bad_choice("priority", text, "normal or emergency".to_owned()))
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

fn picture_size(text: &str) -> Result<PictureSize, CommandError> {
    text.parse().map_err(|_| {
        let allowed =
            format!("W x H in pixels such as 512x512, each side from 1 to {MAXIMUM_SIZE_SIDE_PX}");
        bad_choice("size", text, allowed)
    })
}

fn inference_steps(text: &str) -> Result<InferenceSteps, CommandError> {
    text.parse().map_err(|_| {
        let allowed = format!("a whole number from 1 to {FORGE_IMAGE_MAXIMUM_STEPS}");
        bad_choice("steps", text, allowed)
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
        GenerateRequestError::InvalidBase { base } => bad_choice(
            "base",
            &base,
            "a picture file of the media library, such as bar.png".to_owned(),
        ),
        GenerateRequestError::InvalidReference { reference } => bad_choice(
            "refs",
            &reference,
            "picture files of the media library joined by ',', such as ref-warrior.png".to_owned(),
        ),
        GenerateRequestError::TooManyPictures { count, maximum } => bad_choice(
            "refs",
            &format!("{count} pictures with the base"),
            format!("at most {maximum} pictures, the base included"),
        ),
    }
}

/// Reads the words after `media backend`.
pub fn parse_backend(words: &[String]) -> Result<MediaGenerate, CommandError> {
    let Some((backend, options)) = words.split_first() else {
        return Ok(MediaGenerate::Backend);
    };
    if let Some(maximum) = backend.strip_prefix("refs=") {
        let maximum = maximum
            .parse()
            .ok()
            .filter(|_| options.is_empty())
            .ok_or_else(|| bad_choice("refs", maximum, "a whole number from 1".to_owned()))?;
        return Ok(MediaGenerate::ReferenceCap(ReferenceCap { maximum }));
    }
    match backend.as_str() {
        "codex-cli" if options.is_empty() => {
            Ok(MediaGenerate::SwitchBackend(BackendSwitch::CodexCli {}))
        }
        "forge" => {
            let (mut url, mut project) = (None, None);
            let (mut size, mut steps) = (None, None);
            for word in options {
                let (key, value) = word
                    .split_once('=')
                    .ok_or(CommandError::Usage(MEDIA_BACKEND_USAGE))?;
                let slot = match key {
                    "url" => &mut url,
                    "project" => &mut project,
                    "size" => &mut size,
                    "steps" => &mut steps,
                    _ => return Err(CommandError::Usage(MEDIA_BACKEND_USAGE)),
                };
                if slot.replace(value).is_some() {
                    return Err(CommandError::DuplicateKey(key.to_owned()));
                }
            }
            Ok(MediaGenerate::SwitchBackend(BackendSwitch::Forge {
                url: url.map(str::to_owned),
                project: project.map(str::to_owned),
                size: size.map(picture_size).transpose()?,
                steps: steps.map(inference_steps).transpose()?,
            }))
        }
        "codex-cli" => Err(CommandError::Usage(MEDIA_BACKEND_USAGE)),
        other => Err(bad_choice(
            "backend",
            other,
            "codex-cli or forge".to_owned(),
        )),
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
    let (mut base, mut refs, mut look, mut urgency) = (None, None, None, None);
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
            "base" => &mut base,
            "refs" => &mut refs,
            "look" => &mut look,
            "priority" => &mut urgency,
            _ => &mut wait_text,
        };
        *slot = Some(value);
    }
    if prompt_words.is_empty() {
        return Err(CommandError::Usage(MEDIA_GENERATE_USAGE));
    }
    let (prompt, avoid) = with_look(prompt_words.join(" "), look)?;
    let request = GenerateRequest {
        prompt,
        name: name
            .ok_or(CommandError::Usage(MEDIA_GENERATE_USAGE))?
            .to_owned(),
        purpose: purpose_key.map(purpose).transpose()?,
        aspect: aspect_text.map(aspect).transpose()?,
        wait: wait_text.map(wait).transpose()?.unwrap_or(false),
        base: base.map(str::to_owned),
        references: refs
            .map(|refs| refs.split(',').map(str::to_owned).collect())
            .unwrap_or_default(),
        things: Vec::new(),
        avoid,
        left_out: Vec::new(),
        priority: urgency.map(priority).transpose()?.unwrap_or_default(),
        sheet: None,
    };
    request.checked().map_err(request_error)?;
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
        MediaGenerate::Backend => {
            let shown: GenerateReply<BackendView> =
                dev_client::get_json(port, path::BACKEND).await?;
            Ok(format_backend(&dev_client::reply_value(shown)?))
        }
        MediaGenerate::ReferenceCap(cap) => {
            let set: GenerateReply<BackendView> =
                dev_client::post_json(port, path::REFERENCES, &cap).await?;
            Ok(format_backend(&dev_client::reply_value(set)?))
        }
        MediaGenerate::SwitchBackend(switch) => {
            let switched: GenerateReply<BackendView> =
                dev_client::post_json(port, path::BACKEND, &switch).await?;
            Ok(format_backend(&dev_client::reply_value(switched)?))
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
