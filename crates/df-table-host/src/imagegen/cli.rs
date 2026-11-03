//! Running `media generate` against a host: send the request, read the typed answer, print it as
//! a numbered line with the facts under it. It never prints and never starts a server.

use crate::client::{self, Response};
use crate::command::MediaGenerate;
use crate::dev::DevError;
use crate::limits::MAXIMUM_ECHOED_ANSWER;

use super::model::{GenerateBody, GenerateReply, JobState, JobView};
use super::routes::path;

fn read(response: &Response) -> Result<GenerateReply, DevError> {
    serde_json::from_str(&response.body).map_err(|error| DevError::UnexpectedAnswer {
        status: response.status,
        body: response.body.chars().take(MAXIMUM_ECHOED_ANSWER).collect(),
        reason: Some(error.to_string()),
    })
}

fn seconds(milliseconds: u64) -> String {
    format!("{:.1} s", milliseconds as f64 / 1000.0)
}

/// The numbered line and the facts of a job that ended, or the refusal that ended it.
fn format_finished(job: &JobView) -> Result<String, String> {
    match (job.state, &job.provenance) {
        (JobState::Done, Some(provenance)) => {
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
            if let Some(file) = &job.provenance_file {
                text.push_str(&format!("\n  provenance: {file}"));
            }
            Ok(text)
        }
        _ => {
            let cause = job.cause.map_or("failed", |cause| cause.key());
            let message = job.message.clone().unwrap_or_default();
            Err(format!(
                "generation {} of {} ended without a picture ({cause}) after {}\n  {message}",
                job.job,
                job.name,
                seconds(job.elapsed_ms)
            ))
        }
    }
}

fn format_jobs(jobs: &[JobView]) -> String {
    if jobs.is_empty() {
        return "no generations yet (start one with: media generate <prompt> as=NAME)".to_owned();
    }
    jobs.iter()
        .map(|job| {
            let state = match (job.state, job.cause) {
                (JobState::Running, _) => "running".to_owned(),
                (JobState::Done, _) => "done".to_owned(),
                (JobState::Refused, cause) => {
                    format!("refused ({})", cause.map_or("failed", |cause| cause.key()))
                }
            };
            let file = job.provenance.as_ref().map_or(String::new(), |provenance| {
                format!(
                    " -> {} {}x{}",
                    provenance.media_file, provenance.width_px, provenance.height_px
                )
            });
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

/// Runs one `media generate` command against the host on `port`.
pub async fn run(port: u16, command: MediaGenerate) -> Result<String, DevError> {
    let reply = match command {
        MediaGenerate::Status => {
            let response = client::get(port, path::GENERATE)
                .await
                .map_err(DevError::Client)?;
            read(&response)?
        }
        MediaGenerate::Generate {
            prompt,
            name,
            purpose,
            aspect,
            wait,
        } => {
            let body = serde_json::to_string(&GenerateBody {
                prompt,
                name,
                purpose,
                aspect,
                wait,
            })
            .map_err(|error| DevError::Unwritable {
                what: "request",
                reason: error.to_string(),
            })?;
            let response = client::post_json(port, path::GENERATE, &body)
                .await
                .map_err(DevError::Client)?;
            read(&response)?
        }
    };
    match reply {
        GenerateReply::Started {
            job,
            name,
            deadline_secs,
        } => Ok(format!(
            "ok (generation {job}) started {name}; it has {deadline_secs} s; follow it with: media generate status"
        )),
        GenerateReply::Finished { job } => format_finished(&job).map_err(DevError::Rejected),
        GenerateReply::Refused {
            job,
            cause,
            message,
        } => {
            let number = job.map_or(String::new(), |job| format!("generation {job} "));
            Err(DevError::Rejected(format!(
                "{number}refused ({}): {message}",
                cause.key()
            )))
        }
        GenerateReply::Jobs { jobs } => Ok(format_jobs(&jobs)),
    }
}
