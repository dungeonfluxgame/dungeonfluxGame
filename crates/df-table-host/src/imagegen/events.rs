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

use super::backend::{BackendError, BackendTuning};
use super::limits::MAXIMUM_NAME_CHARS;
use super::model::{Aspect, Priority, Purpose};
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
        /// The width and height asked of the backend: the shape's own, or the size the backend
        /// was switched to.
        size: (u32, u32),
        prompt_chars: usize,
        deadline: Duration,
        backend: &'static str,
        running: usize,
    },
    /// A job that is not of normal urgency, the line after its request: an emergency picture is
    /// put first by a backend that has a queue of its own.
    Prioritised {
        job: u64,
        name: String,
        priority: Priority,
        backend: &'static str,
    },
    /// A person's sheet showed several views side by side and the picture was given one view of
    /// it (the front view for an edit), so no view is painted as another person.
    SheetView {
        job: u64,
        name: String,
        picture: String,
        view: &'static str,
    },
    /// The pictures a job was given to follow, the line after its request: the picture it edits,
    /// the references attached in order, the `maximum` the backend takes, and what its caller left
    /// out for want of room. Only written for a job that has any of them.
    References {
        job: u64,
        name: String,
        base: Option<String>,
        attached: Vec<String>,
        maximum: usize,
        left_out: Vec<String>,
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
    /// The backend delivered a picture of another shape than the one asked of it, the line before
    /// its `stored` line: whatever shows the picture shows it in the wrong shape or pads it.
    Reshaped {
        job: u64,
        name: String,
        asked: (u32, u32),
        delivered: (u32, u32),
        backend: &'static str,
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
    /// New jobs start on `to` from now, with the `tuning` set on it; the `running` jobs go on with
    /// the backend they started on.
    Switched {
        from: &'static str,
        to: &'static str,
        tuning: BackendTuning,
        running: usize,
    },
    /// A switch of the backend was refused and nothing changed: `to` is the backend asked for,
    /// when the body named one.
    SwitchRefused {
        to: Option<&'static str>,
        error: GenerateError,
    },
}

/// Picture names as a line writes them: each one plain word, comma-separated, or `-` for none.
fn names_or_dash(names: &[String]) -> String {
    if names.is_empty() {
        return "-".to_owned();
    }
    names
        .iter()
        .map(|name| plain_word(name))
        .collect::<Vec<_>>()
        .join(",")
}

/// A count as a line writes it: the number, or `-` when it is not known.
fn count_or_dash(count: Option<u64>) -> String {
    count.map_or_else(|| "-".to_owned(), |count| count.to_string())
}

/// A name as one plain word of a line: anything that is not a letter, digit, '-', '_' or '.' (a
/// picture's file name keeps its extension) becomes '_', and a long name is cut, so a refused
/// request cannot break the line's shape.
fn plain_word(name: &str) -> String {
    let word: String = name
        .chars()
        .take(MAXIMUM_NAME_CHARS)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
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
            Self::Prioritised { .. } => "priority",
            Self::SheetView { .. } => "sheet_view",
            Self::References { .. } => "references",
            Self::Started { .. } => "started",
            Self::Stored { .. } => "stored",
            Self::Reshaped { .. } => "reshaped",
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
                size,
                prompt_chars,
                deadline,
                backend,
                running,
            } => format!(
                concat!(
                    "IMAGEGEN {} job={} name={} purpose={} aspect={} size={}x{} prompt_chars={} ",
                    "deadline_ms={} backend={} running={}"
                ),
                verb,
                job,
                name,
                purpose.key(),
                aspect,
                size.0,
                size.1,
                prompt_chars,
                deadline.as_millis(),
                backend,
                running
            ),
            Self::SheetView {
                job,
                name,
                picture,
                view,
            } => format!(
                "IMAGEGEN {verb} job={job} name={} picture={} view={view}",
                plain_word(name),
                plain_word(picture)
            ),
            Self::Prioritised {
                job,
                name,
                priority,
                backend,
            } => format!(
                "IMAGEGEN {verb} job={job} name={} level={} backend={backend}",
                plain_word(name),
                priority.key()
            ),
            Self::References {
                job,
                name,
                base,
                attached,
                maximum,
                left_out,
            } => format!(
                concat!(
                    "IMAGEGEN {} job={} name={} base={} attached={} count={} maximum={} ",
                    "left_out={}"
                ),
                verb,
                job,
                plain_word(name),
                base.as_deref().map_or_else(|| "-".to_owned(), plain_word),
                names_or_dash(attached),
                attached.len(),
                maximum,
                names_or_dash(left_out)
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
            Self::Reshaped {
                job,
                name,
                asked,
                delivered,
                backend,
            } => format!(
                "IMAGEGEN {verb} job={job} name={} asked={}x{} delivered={}x{} backend={backend}",
                plain_word(name),
                asked.0,
                asked.1,
                delivered.0,
                delivered.1
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
            Self::Switched {
                from,
                to,
                tuning,
                running,
            } => format!("IMAGEGEN {verb} from={from} to={to}{tuning} running={running}"),
            Self::SwitchRefused { to, error } => format!(
                "IMAGEGEN {verb} verb=backend to={} {}",
                to.unwrap_or("_"),
                error_fields(error)
            ),
        }
    }
}
