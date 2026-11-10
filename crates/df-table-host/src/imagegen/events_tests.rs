//! The lines of the audit log: ids, keys, sizes and milliseconds, never the prompt.

use std::path::PathBuf;
use std::time::Duration;

use super::backend::{BackendError, BackendTuning};
use super::events::ImageGenEvent;
use super::model::{GenerateRequestError, ImageRequest, InferenceSteps, PictureSize};
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
        size: (1672, 941),
        prompt_chars: request.prompt().chars().count(),
        deadline: Duration::from_secs(900),
        backend: "fake",
        running: 2,
    }
    .line();
    assert_eq!(
        line,
        format!(
            concat!(
                "IMAGEGEN requested job=1 name=hall purpose=background aspect=16:9 size=1672x941 ",
                "prompt_chars={} deadline_ms=900000 backend=fake running=2"
            ),
            secret.chars().count()
        )
    );
    assert!(!line.contains("dragon"));
}

#[test]
fn the_pictures_a_job_follows_are_one_line_of_names() {
    let line = ImageGenEvent::References {
        job: 3,
        name: "roam-01".to_owned(),
        base: None,
        attached: vec![
            "ref-danny.png".to_owned(),
            "item-danny-easel.png".to_owned(),
        ],
        maximum: 8,
        left_out: Vec::new(),
    }
    .line();
    assert_eq!(
        line,
        concat!(
            "IMAGEGEN references job=3 name=roam-01 base=- ",
            "attached=ref-danny.png,item-danny-easel.png count=2 maximum=8 left_out=-"
        )
    );
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
        tokens_in: Some(10),
        tokens_out: None,
    };
    assert_eq!(
        stored.line(),
        concat!(
            "IMAGEGEN stored job=2 name=hall file=hall.png bytes=99 size=1672x941 ",
            "duration_ms=1500 backend=fake sha256=0123456789abcdef tokens_in=10 tokens_out=-"
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
fn a_forge_refusal_line_quotes_forge_s_reason() {
    let failed = ImageGenEvent::Failed {
        job: 77,
        name: "hall".to_owned(),
        error: GenerateError::Backend {
            job: Some(77),
            backend: BackendError::ForgeFailed {
                detail: "rejected: Image description must contain 1–4000 characters.".to_owned(),
            },
        },
        duration: Duration::from_millis(198),
        backend: "forge",
    };
    assert_eq!(
        failed.line(),
        concat!(
            "IMAGEGEN failed job=77 name=hall error=backend cause=forge_failed ",
            "detail=\"rejected: Image description must contain 1–4000 characters.\" ",
            "duration_ms=198 backend=forge"
        )
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

#[test]
fn a_check_line_names_the_backend_its_time_and_the_failed_precondition_by_key() {
    let held = ImageGenEvent::Check {
        backend: "codex-cli",
        duration: Duration::from_millis(12),
        failed: None,
    };
    assert_eq!(
        held.line(),
        "IMAGEGEN check backend=codex-cli duration_ms=12 result=ok"
    );
    let failed = ImageGenEvent::Check {
        backend: "codex-cli",
        duration: Duration::from_millis(3),
        failed: Some(BackendError::NoLogin {
            home: PathBuf::from("C:/secret/codex-home"),
        }),
    };
    assert_eq!(
        failed.line(),
        "IMAGEGEN check backend=codex-cli duration_ms=3 result=no_login"
    );
}

#[test]
fn a_started_line_carries_the_job_and_its_wait_in_milliseconds() {
    let line = ImageGenEvent::Started {
        job: 4,
        wait: Duration::from_millis(250),
    }
    .line();
    assert_eq!(line, "IMAGEGEN started job=4 wait_ms=250");
}

#[test]
fn a_scratch_folder_left_behind_is_named_by_its_cleanup_key() {
    let line = ImageGenEvent::CleanupFailed {
        job: 3,
        cause: df_imagery::oneshot::CleanupCause::PermissionDenied,
    }
    .line();
    assert_eq!(line, "IMAGEGEN cleanup_failed job=3 kind=permission_denied");
}

#[test]
fn a_switch_line_shows_the_size_and_steps_set_and_nothing_when_none_is() {
    let switched = |tuning| {
        ImageGenEvent::Switched {
            from: "codex-cli",
            to: "forge",
            tuning,
            running: 1,
        }
        .line()
    };
    assert_eq!(
        switched(BackendTuning::default()),
        "IMAGEGEN switched from=codex-cli to=forge running=1"
    );
    let tuning = BackendTuning {
        size: Some("512x512".parse::<PictureSize>().expect("a size")),
        steps: Some(InferenceSteps::new(20).expect("steps")),
    };
    assert_eq!(
        switched(tuning),
        "IMAGEGEN switched from=codex-cli to=forge size=512x512 steps=20 running=1"
    );
}

#[test]
fn a_picture_of_another_shape_than_asked_is_one_line_of_both_sizes() {
    let line = ImageGenEvent::Reshaped {
        job: 12,
        name: "roam-06-look-1".to_owned(),
        asked: (1672, 941),
        delivered: (512, 512),
        backend: "forge",
    }
    .line();
    assert_eq!(
        line,
        concat!(
            "IMAGEGEN reshaped job=12 name=roam-06-look-1 asked=1672x941 delivered=512x512 ",
            "backend=forge"
        )
    );
}
