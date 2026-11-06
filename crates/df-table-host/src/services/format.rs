//! The text `services` prints: one line per service, of keys and values only.

use super::request::ServicesReport;

/// A value that may be absent, as a line writes it.
fn or_dash(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}

/// `set` or `unset`.
fn set(present: bool) -> &'static str {
    if present { "set" } else { "unset" }
}

/// One line per service: text, imagegen, locate.
pub fn format_services(report: &ServicesReport) -> String {
    let text = &report.text;
    let image = &report.imagegen;
    let locate = &report.locate;
    [
        format!(
            concat!(
                "text: backend={} profile={} model={} configured={} missing={} ",
                "attempts_kept={} in_flight={}"
            ),
            text.backend.key(),
            text.profile,
            or_dash(text.identity_model.as_deref()),
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
    ]
    .join("\n")
}
