//! What the picture generator records: one typed event for each request, each stored picture and
//! each refusal, rendered as one `IMAGEGEN <verb> key=value ...` line for the audit log.
//!
//! The request and its outcome are only recorded: no budget, price or count of pictures ever
//! blocks a request (owner decision: no spend caps). A line holds ids, closed keys, sizes and
//! milliseconds, and the prompt's length; it never holds the prompt, which is private text.

use std::time::Duration;

use super::model::{Aspect, Purpose, RefusalCause};

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
        cause: RefusalCause,
        duration: Duration,
        backend: &'static str,
    },
    /// The request never became a job.
    Refused {
        name: String,
        cause: RefusalCause,
        backend: &'static str,
    },
}

/// A name as one plain word of a line: anything that is not a letter, digit, '-' or '_' becomes
/// '_', and a long name is cut, so a refused request cannot break the line's shape.
fn plain_word(name: &str) -> String {
    let word: String = name
        .chars()
        .take(super::model::MAXIMUM_NAME_CHARS)
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

impl ImageGenEvent {
    /// The event of a request that never became a job; the name is shown as a plain word.
    pub fn refused(name: &str, cause: RefusalCause, backend: &'static str) -> Self {
        Self::Refused {
            name: plain_word(name),
            cause,
            backend,
        }
    }

    pub fn verb(&self) -> &'static str {
        match self {
            Self::Requested { .. } => "requested",
            Self::Stored { .. } => "stored",
            Self::Failed { .. } => "failed",
            Self::Refused { .. } => "refused",
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
                "IMAGEGEN {verb} job={job} name={name} purpose={} aspect={aspect} prompt_chars={prompt_chars} deadline_s={} backend={backend}",
                purpose.key(),
                deadline.as_secs()
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
                "IMAGEGEN {verb} job={job} name={name} file={file} bytes={bytes} size={width_px}x{height_px} duration_ms={} backend={backend} sha256={sha256_prefix}",
                duration.as_millis()
            ),
            Self::Failed {
                job,
                name,
                cause,
                duration,
                backend,
            } => format!(
                "IMAGEGEN {verb} job={job} name={name} cause={} duration_ms={} backend={backend}",
                cause.key(),
                duration.as_millis()
            ),
            Self::Refused {
                name,
                cause,
                backend,
            } => format!(
                "IMAGEGEN {verb} name={name} cause={} backend={backend}",
                cause.key()
            ),
        }
    }
}
