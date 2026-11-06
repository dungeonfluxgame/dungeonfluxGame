//! What the picture generator records: one typed event for each request, each stored picture,
//! each job that ended without one, each refusal and each setting that does not read, rendered as
//! one `IMAGEGEN <verb> key=value` line for the audit log.
//!
//! The request and its outcome are only recorded: no budget, price or count of pictures ever
//! blocks a request (owner decision: no spend caps). A line holds ids, closed keys, sizes and
//! milliseconds, and the prompt's length; it never holds the prompt, which is private text.

use std::time::Duration;

use super::limits::MAXIMUM_NAME_CHARS;
use super::model::{Aspect, Purpose};
use super::request::GenerateError;

/// One thing the generator did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageGenEvent {
    /// A job started: the request was valid and every precondition held.
    Requested {
        job: u64,
        name: String,
        purpose: Purpose,
        aspect: Aspect,
        prompt_chars: usize,
        deadline: Duration,
        backend: &'static str,
    },
    /// The picture was made and stored in the media library.
    Stored {
        job: u64,
        name: String,
        file: String,
        bytes: u64,
        width_px: u32,
        height_px: u32,
        duration: Duration,
        backend: &'static str,
        sha256_prefix: String,
    },
    /// The job ended without a stored picture.
    Failed {
        job: u64,
        name: String,
        error: GenerateError,
        duration: Duration,
        backend: &'static str,
    },
    /// The request never became a job. `name` is the name asked for, when the body had one; it
    /// is written as a plain word, so a refused name cannot break the line's shape.
    Refused {
        name: Option<String>,
        error: GenerateError,
        backend: &'static str,
    },
    /// The setting in `variable` does not read, so `default` is used. The text of the setting is
    /// not written: it was not checked to be one plain word.
    InvalidSetting {
        variable: &'static str,
        default: Duration,
    },
}

/// A name as one plain word of a line: anything that is not a letter, digit, '-' or '_' becomes
/// '_', and a long name is cut, so a refused request cannot break the line's shape.
fn plain_word(name: &str) -> String {
    let word: String = name
        .chars()
        .take(MAXIMUM_NAME_CHARS)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if word.is_empty() {
        "_".to_owned()
    } else {
        word
    }
}

/// `error=<key>`, then ` cause=<key>` when the error has a typed cause.
fn error_fields(error: &GenerateError) -> String {
    match error.cause_key() {
        Some(cause) => format!("error={} cause={cause}", error.key()),
        None => format!("error={}", error.key()),
    }
}

impl ImageGenEvent {
    pub fn verb(&self) -> &'static str {
        match self {
            Self::Requested { .. } => "requested",
            Self::Stored { .. } => "stored",
            Self::Failed { .. } => "failed",
            Self::Refused { .. } => "refused",
            Self::InvalidSetting { .. } => "setting",
        }
    }

    /// The audit line, `IMAGEGEN <verb> key=value ...`.
    pub fn line(&self) -> String {
        let verb = self.verb();
        match self {
            Self::Requested {
                job,
                name,
                purpose,
                aspect,
                prompt_chars,
                deadline,
                backend,
            } => format!(
                concat!(
                    "IMAGEGEN {} job={} name={} purpose={} aspect={} prompt_chars={} ",
                    "deadline_ms={} backend={}"
                ),
                verb,
                job,
                name,
                purpose.key(),
                aspect,
                prompt_chars,
                deadline.as_millis(),
                backend
            ),
            Self::Stored {
                job,
                name,
                file,
                bytes,
                width_px,
                height_px,
                duration,
                backend,
                sha256_prefix,
            } => format!(
                concat!(
                    "IMAGEGEN {} job={} name={} file={} bytes={} size={}x{} duration_ms={} ",
                    "backend={} sha256={}"
                ),
                verb,
                job,
                name,
                file,
                bytes,
                width_px,
                height_px,
                duration.as_millis(),
                backend,
                sha256_prefix
            ),
            Self::Failed {
                job,
                name,
                error,
                duration,
                backend,
            } => format!(
                "IMAGEGEN {verb} job={job} name={name} {} duration_ms={} backend={backend}",
                error_fields(error),
                duration.as_millis()
            ),
            Self::Refused {
                name,
                error,
                backend,
            } => format!(
                "IMAGEGEN {verb} verb=generate name={} {} backend={backend}",
                name.as_deref().map_or_else(|| "_".to_owned(), plain_word),
                error_fields(error)
            ),
            Self::InvalidSetting { variable, default } => format!(
                "IMAGEGEN {verb} variable={variable} status=invalid default_ms={}",
                default.as_millis()
            ),
        }
    }
}
