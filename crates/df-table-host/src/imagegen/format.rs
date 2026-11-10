//! The text `media generate` prints: a numbered line with the facts under it for a job, one line
//! per job for the status, the end of a job that has no picture, and the refusal line.

use super::request::{BackendView, GenerateError, JobState, JobView, Sidecar};

fn seconds(milliseconds: u64) -> String {
    format!("{:.1} s", milliseconds as f64 / 1000.0)
}

/// The backend new jobs start on and what it takes.
pub(super) fn format_backend(view: &BackendView) -> String {
    let limit = |maximum: Option<usize>, none: &str| {
        maximum.map_or_else(|| none.to_owned(), |maximum| format!("at most {maximum}"))
    };
    let tuned = [
        view.size.map(|size| format!("size {size}")),
        view.steps.map(|steps| format!("steps {steps}")),
    ]
    .into_iter()
    .flatten()
    .map(|setting| format!(", {setting}"))
    .collect::<String>();
    format!(
        concat!(
            "image backend: {} (text {} characters, negative prompt {}, {} pictures{}, soft ",
            "cap {}); {} running on the backend each started on"
        ),
        view.backend,
        limit(
            view.limits.maximum_text_characters,
            "the request's bound of"
        ),
        limit(
            view.limits.maximum_negative_characters,
            "none: bans go in the text,"
        ),
        view.limits.maximum_pictures,
        tuned,
        view.reference_cap,
        view.running
    )
}

/// The answer to a generation: the job that runs on, or the picture it stored.
pub(super) fn format_job(job: &JobView) -> String {
    match &job.state {
        JobState::Stored {
            provenance,
            sidecar,
        } => {
            let mut text = format!(
                "ok (generation {}) stored {} in the media library ({} bytes, {}x{}, {})",
                job.job,
                provenance.media_file,
                provenance.bytes,
                provenance.width_px,
                provenance.height_px,
                provenance.mime
            );
            text.push_str(&format!(
                "\n  made by {} in {} (asked for {}x{}, {} {})",
                provenance.backend,
                seconds(provenance.duration_ms),
                provenance.requested_width_px,
                provenance.requested_height_px,
                provenance.purpose.key(),
                provenance.aspect
            ));
            text.push_str(&format!("\n  sha256 {}", provenance.sha256));
            text.push_str(&format!("\n  note: {}", provenance.note));
            match sidecar {
                Some(Sidecar::Written { path }) => {
                    text.push_str(&format!("\n  provenance: {}", path.display()));
                }
                Some(Sidecar::NotWritten { path, cause }) => {
                    text.push_str(&format!(
                        "\n  provenance: {} (not written: {cause})",
                        path.display()
                    ));
                }
                None => {}
            }
            text
        }
        JobState::Running => format!(
            concat!(
                "ok (generation {}) started {}; it has {}; follow it with: ",
                "media generate status"
            ),
            job.job,
            job.name,
            seconds(job.deadline_ms)
        ),
        JobState::Failed { error, message } => format_ended(job, error, message),
    }
}

/// A job that ended without a picture: its number, name, keys, how long it ran and the host's
/// sentence.
pub(crate) fn format_ended(job: &JobView, error: &GenerateError, message: &str) -> String {
    format!(
        "generation {} of {} ended without a picture ({}) after {}\n  {message}",
        job.job,
        job.name,
        error_keys(error),
        seconds(job.elapsed_ms)
    )
}

/// One line per job, oldest first.
pub(super) fn format_jobs(jobs: &[JobView]) -> String {
    if jobs.is_empty() {
        return "no generations yet (start one with: media generate <prompt> as=NAME)".to_owned();
    }
    jobs.iter()
        .map(|job| {
            let (state, file) = match &job.state {
                JobState::Running => ("running".to_owned(), String::new()),
                JobState::Stored { provenance, .. } => (
                    "done".to_owned(),
                    format!(
                        " -> {} {}x{}",
                        provenance.media_file, provenance.width_px, provenance.height_px
                    ),
                ),
                JobState::Failed { error, .. } => {
                    (format!("refused ({})", error_keys(error)), String::new())
                }
            };
            format!(
                "generation {}: {} {} {}{file}",
                job.job,
                job.name,
                state,
                seconds(job.elapsed_ms)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `<error>` or `<error>: <cause>`.
fn error_keys(error: &GenerateError) -> String {
    match error.cause_key() {
        Some(cause) => format!("{}: {cause}", error.key()),
        None => error.key().to_owned(),
    }
}

/// The refusal of a generation: its job when one started, its keys and the host's sentence.
pub(crate) fn format_refusal(error: &GenerateError, message: &str) -> String {
    let number = error
        .job()
        .map_or(String::new(), |job| format!("generation {job} "));
    format!("{number}refused ({}): {message}", error_keys(error))
}
