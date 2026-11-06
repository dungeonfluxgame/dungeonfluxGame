//! The lines of the audit log: ids, keys, sizes and milliseconds, never the prompt.

use std::path::PathBuf;
use std::time::Duration;

use super::backend::BackendError;
use super::events::ImageGenEvent;
use super::model::{GenerateRequestError, ImageRequest};
use super::request::GenerateError;

#[test]
fn an_audit_line_never_holds_the_prompt() {
    let secret = "the dragon's private secret plan";
    let request = ImageRequest::new(secret, "hall", None, None).expect("ok");
    let line = ImageGenEvent::Requested {
        job: 1,
        name: request.name().to_owned(),
        purpose: request.purpose(),
        aspect: request.aspect(),
        prompt_chars: request.prompt().chars().count(),
        deadline: Duration::from_secs(900),
        backend: "fake",
    }
    .line();
    assert_eq!(
        line,
        format!(
            concat!(
                "IMAGEGEN requested job=1 name=hall purpose=background aspect=16:9 ",
                "prompt_chars={} deadline_ms=900000 backend=fake"
            ),
            secret.chars().count()
        )
    );
    assert!(!line.contains("dragon"));
}

#[test]
fn a_refused_request_cannot_break_the_shape_of_its_line() {
    let name = "bad name\nIMAGEGEN stored".to_owned();
    let line = ImageGenEvent::Refused {
        name: Some(name.clone()),
        error: GenerateError::InvalidRequest(GenerateRequestError::InvalidName { name }),
        backend: "fake",
    }
    .line();
    assert_eq!(line.lines().count(), 1);
    assert_eq!(
        line,
        concat!(
            "IMAGEGEN refused verb=generate name=bad_name_IMAGEGEN_stored error=invalid_request ",
            "cause=invalid_name backend=fake"
        )
    );
    assert_eq!(
        ImageGenEvent::Refused {
            name: None,
            error: GenerateError::Backend {
                job: None,
                backend: BackendError::NotSwitchedOn,
            },
            backend: "none",
        }
        .line(),
        "IMAGEGEN refused verb=generate name=_ error=backend cause=not_switched_on backend=none"
    );
}

#[test]
fn a_refused_line_names_the_error_key_and_no_private_path() {
    let line = ImageGenEvent::Refused {
        name: Some("hall".to_owned()),
        error: GenerateError::Backend {
            job: None,
            backend: BackendError::NoLogin {
                home: PathBuf::from("C:/secret/codex-home"),
            },
        },
        backend: "codex-cli",
    }
    .line();
    assert_eq!(
        line,
        "IMAGEGEN refused verb=generate name=hall error=backend cause=no_login backend=codex-cli"
    );
}

#[test]
fn the_stored_and_failed_lines_carry_ids_sizes_and_milliseconds() {
    let stored = ImageGenEvent::Stored {
        job: 2,
        name: "hall".to_owned(),
        file: "hall.png".to_owned(),
        bytes: 99,
        width_px: 1672,
        height_px: 941,
        duration: Duration::from_millis(1500),
        backend: "fake",
        sha256_prefix: "0123456789abcdef".to_owned(),
    };
    assert_eq!(
        stored.line(),
        concat!(
            "IMAGEGEN stored job=2 name=hall file=hall.png bytes=99 size=1672x941 ",
            "duration_ms=1500 backend=fake sha256=0123456789abcdef"
        )
    );
    let failed = ImageGenEvent::Failed {
        job: 2,
        name: "hall".to_owned(),
        error: GenerateError::Backend {
            job: Some(2),
            backend: BackendError::Timeout,
        },
        duration: Duration::from_secs(2),
        backend: "fake",
    };
    assert_eq!(
        failed.line(),
        "IMAGEGEN failed job=2 name=hall error=backend cause=timeout duration_ms=2000 backend=fake"
    );
}

#[test]
fn a_setting_that_does_not_read_names_its_variable_and_the_default_in_milliseconds() {
    let line = ImageGenEvent::InvalidSetting {
        variable: "DF_IMAGEGEN_DEADLINE_SECS",
        default: Duration::from_secs(900),
    }
    .line();
    assert_eq!(
        line,
        "IMAGEGEN setting variable=DF_IMAGEGEN_DEADLINE_SECS status=invalid default_ms=900000"
    );
}
