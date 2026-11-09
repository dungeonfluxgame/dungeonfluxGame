//! What the picture generator records: one typed event for each precondition check, each request,
//! each job's start, each stored picture, each job that ended without one, each refusal, each
//! scratch folder left behind and each setting that does not read, rendered as one
//! `IMAGEGEN <verb> key=value` line for the audit log.
//!
//! The request and its outcome are only recorded: no budget, price or count of pictures ever
//! blocks a request (owner decision: no spend caps). A line holds ids, closed keys, sizes and
//! milliseconds, the prompt's length and the Forge studio's own sentence for a refusal; it never
//! holds the prompt, which is private text.

use std::time::Duration;

use super::backend::BackendError;
use super::limits::MAXIMUM_NAME_CHARS;
use super::model::{Aspect, Purpose};
use super::request::GenerateError;
use df_imagery::oneshot::CleanupCause;

/// One thing the generator did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageGenEvent {
    /// The backend's preconditions were looked at before a job: how long that took, and the
    /// precondition that failed (`None`: every one held).
    Check {
        backend: &'static str,
        duration: Duration,
        failed: Option<BackendError>,
    },
    /// A job was opened: the request was valid and every precondition held. `running` counts the
    /// jobs running now, this one included.
    Requested {
        job: u64,
        name: String,
        purpose: Purpose,
        aspect: Aspect,
        prompt_chars: usize,
        deadline: Duration,
        backend: &'static str,
        running: usize,
    },
    /// The job's own task began its work, `wait` after the request arrived (the check included).
    Started { job: u64, wait: Duration },
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
        /// The token counts the backend reported, when it reported them.
        tokens_in: Option<u64>,
        tokens_out: Option<u64>,
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
    /// The job's scratch folder could not be removed after the attempt; the job's outcome stands.
    CleanupFailed { job: u64, cause: CleanupCause },
    /// New jobs start on `to` from now; the `running` jobs go on with the backend they started on.
    Switched {
        from: &'static str,
        to: &'static str,
        running: usize,
    },
    /// A switch of the backend was refused and nothing changed: `to` is the backend asked for,
    /// when the body named one.
    SwitchRefused {
        to: Option<&'static str>,
        error: GenerateError,
    },
}

/// A count as a line writes it: the number, or `-` when it is not known.
fn count_or_dash(count: Option<u64>) -> String {
    count.map_or_else(|| "-".to_owned(), |count| count.to_string())
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

/// `error=<key>`, then ` cause=<key>` when the error has a typed cause, then the Forge studio's
/// `detail` (its cause's key and its own reason for a refusal) quoted, so the line keeps its shape,
/// or the length and bound of a text the backend cannot take.
fn error_fields(error: &GenerateError) -> String {
    let mut fields = match error.cause_key() {
        Some(cause) => format!("error={} cause={cause}", error.key()),
        None => format!("error={}", error.key()),
    };
    match error {
        GenerateError::Backend {
            backend: BackendError::ForgeFailed { detail },
            ..
        } => fields.push_str(&format!(" detail={detail:?}")),
        GenerateError::Backend {
            backend:
                BackendError::TextTooLong {
                    characters,
                    maximum,
                },
            ..
        } => fields.push_str(&format!(" characters={characters} maximum={maximum}")),
        _ => {}
    }
    fields
}

impl ImageGenEvent {
    pub fn verb(&self) -> &'static str {
        match self {
            Self::Check { .. } => "check",
            Self::Requested { .. } => "requested",
            Self::Started { .. } => "started",
            Self::Stored { .. } => "stored",
            Self::Failed { .. } => "failed",
            Self::Refused { .. } => "refused",
            Self::InvalidSetting { .. } => "setting",
            Self::CleanupFailed { .. } => "cleanup_failed",
            Self::Switched { .. } => "switched",
            Self::SwitchRefused { .. } => "refused",
        }
    }

    /// The audit line, `IMAGEGEN <verb> key=value ...`.
    pub fn line(&self) -> String {
        let verb = self.verb();
        match self {
            Self::Check {
                backend,
                duration,
                failed,
            } => format!(
                "IMAGEGEN {verb} backend={backend} duration_ms={} result={}",
                duration.as_millis(),
                failed.as_ref().map_or("ok", BackendError::key)
            ),
            Self::Requested {
                job,
                name,
                purpose,
                aspect,
                prompt_chars,
                deadline,
                backend,
                running,
            } => format!(
                concat!(
                    "IMAGEGEN {} job={} name={} purpose={} aspect={} prompt_chars={} ",
                    "deadline_ms={} backend={} running={}"
                ),
                verb,
                job,
                name,
                purpose.key(),
                aspect,
                prompt_chars,
                deadline.as_millis(),
                backend,
                running
            ),
            Self::Started { job, wait } => {
                format!("IMAGEGEN {verb} job={job} wait_ms={}", wait.as_millis())
            }
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
                tokens_in,
                tokens_out,
            } => format!(
                concat!(
                    "IMAGEGEN {} job={} name={} file={} bytes={} size={}x{} duration_ms={} ",
                    "backend={} sha256={} tokens_in={} tokens_out={}"
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
                sha256_prefix,
                count_or_dash(*tokens_in),
                count_or_dash(*tokens_out)
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
            Self::CleanupFailed { job, cause } => {
                format!("IMAGEGEN {verb} job={job} kind={}", cause.key())
            }
            Self::Switched { from, to, running } => {
                format!("IMAGEGEN {verb} from={from} to={to} running={running}")
            }
            Self::SwitchRefused { to, error } => format!(
                "IMAGEGEN {verb} verb=backend to={} {}",
                to.unwrap_or("_"),
                error_fields(error)
            ),
        }
    }
}
