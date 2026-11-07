//! The text `services` prints: one line per service, of keys and values only.

use super::request::ServicesReport;

/// A value that may be absent, as a line writes it.
fn or_dash(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}

/// A count that may be unknown, as a line writes it.
fn count_or_dash(value: Option<u64>) -> String {
    value.map_or_else(|| "-".to_owned(), |value| value.to_string())
}

/// `set` or `unset`.
fn set(present: bool) -> &'static str {
    if present { "set" } else { "unset" }
}

/// One line per service: text, imagegen, locate, speech.
pub fn format_services(report: &ServicesReport) -> String {
    let text = &report.text;
    let image = &report.imagegen;
    let locate = &report.locate;
    let speech = &report.speech;
    [
        format!(
            concat!(
                "text: backend={} profile={} model={} persona_model={} persona_profile={} ",
                "configured={} missing={} attempts_kept={} in_flight={}"
            ),
            text.backend.key(),
            text.profile,
            or_dash(text.identity_model.as_deref()),
            or_dash(text.persona_model.as_deref()),
            or_dash(text.persona_profile.as_deref()),
            text.configured,
            or_dash(text.missing.as_deref()),
            text.attempts_kept,
            text.in_flight
        ),
        format!(
            concat!(
                "imagegen: backend={} mode={} deadline_ms={} provenance_dir={} codex_home={} ",
                "running={} last_outcome={}"
            ),
            image.backend,
            image.mode.key(),
            image.deadline_ms,
            set(image.provenance_dir_set),
            set(image.codex_home_set),
            image.running,
            or_dash(image.last_outcome.as_deref())
        ),
        format!(
            concat!(
                "locate: model={} endpoint_host={} timeout_ms={} key={} ",
                "min_confidence_permille={}"
            ),
            locate.model,
            locate.endpoint_host,
            locate.timeout_ms,
            if locate.key_present {
                "present"
            } else {
                "missing"
            },
            locate.min_confidence_permille
        ),
        format!(
            concat!(
                "speech: tts_model={} stt_model={} configured={} missing={} voices={} ",
                "clips_kept={}"
            ),
            speech.tts_model,
            speech.stt_model,
            speech.configured,
            or_dash(speech.missing.as_deref()),
            count_or_dash(speech.voices),
            count_or_dash(speech.clips_kept)
        ),
    ]
    .join("\n")
}
