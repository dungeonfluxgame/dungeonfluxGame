//! The preconditions of the Codex backend: each one fails with its own typed error and says what
//! the owner must do. Nothing here starts Codex or opens a login file. Each backend's limits; the
//! Forge backend's bound on its text refuses a longer one before anything is sent, and its job
//! carries what a picture must not show as a negative prompt.

use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::http::StatusCode;
use df_providers::{
    FORGE_IMAGE_MAXIMUM_DESCRIPTION_CHARACTERS, FORGE_IMAGE_NEGATIVE_GUIDANCE, ForgeClient,
    ForgeConfig,
};
use tempfile::TempDir;

use super::backend::{
    BackendError, BackendLimits, BackendTuning, CodexBackend, CodexSettings, ForgeImageBackend,
    ImageBackend, SwitchedOff, backend_for, forge_image_job,
};
use super::limits::{CODEX_MAXIMUM_PICTURES, MAXIMUM_PICTURES};
use super::model::{BackendRequest, InferenceSteps, PictureSize, Priority};
use crate::io_cause::IoCause;

/// A signed-in-looking home: the login file is only looked for, so its content is irrelevant.
fn home_with_login(root: &Path) -> PathBuf {
    let home = root.join("codex-home");
    std::fs::create_dir_all(&home).expect("makes a directory");
    std::fs::write(home.join("auth.json"), b"{}").expect("writes");
    home
}

fn executable(root: &Path) -> PathBuf {
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("makes a directory");
    let name = if cfg!(windows) { "codex.exe" } else { "codex" };
    let path = bin.join(name);
    std::fs::write(&path, b"not really codex").expect("writes");
    path
}

fn settings(root: &Path) -> CodexSettings {
    CodexSettings {
        executable: Some(executable(root)),
        path_directories: Vec::new(),
        codex_home: Some(home_with_login(root)),
        work_root: root.join("work").join("jobs"),
        agent_model: None,
    }
}

fn check(settings: CodexSettings) -> Result<(), BackendError> {
    CodexBackend::new(settings).check()
}

fn refused(settings: CodexSettings) -> BackendError {
    check(settings).expect_err("refused")
}

#[test]
fn a_home_with_a_login_and_an_executable_passes() {
    let root = TempDir::new().expect("makes a directory");
    check(settings(root.path())).expect("every precondition holds");
    assert!(
        root.path().join("work").join("jobs").is_dir(),
        "the scratch folder is made"
    );
}

#[test]
fn no_codex_home_is_refused_and_says_how_to_start_the_host() {
    let root = TempDir::new().expect("makes a directory");
    let mut without = settings(root.path());
    without.codex_home = None;
    let error = refused(without);
    assert_eq!(error, BackendError::NoCodexHome);
    assert!(error.to_string().contains("CODEX_HOME="), "{error}");
}

#[test]
fn a_home_without_a_login_file_says_to_run_codex_login() {
    let root = TempDir::new().expect("makes a directory");
    let mut signed_out = settings(root.path());
    let empty = root.path().join("empty-home");
    std::fs::create_dir_all(&empty).expect("makes a directory");
    signed_out.codex_home = Some(empty.clone());
    let error = refused(signed_out);
    assert_eq!(error, BackendError::NoLogin { home: empty });
    assert!(error.to_string().contains("codex login"), "{error}");

    let mut missing = settings(root.path());
    let nowhere = root.path().join("no-such-home");
    missing.codex_home = Some(nowhere.clone());
    assert_eq!(refused(missing), BackendError::NoLogin { home: nowhere });
}

#[test]
fn a_home_with_a_config_file_is_not_the_dedicated_home() {
    let root = TempDir::new().expect("makes a directory");
    let with_config = settings(root.path());
    let home = with_config.codex_home.clone().expect("a home");
    std::fs::write(home.join("config.toml"), b"model = \"x\"").expect("writes");
    assert_eq!(refused(with_config), BackendError::HomeHasConfig { home });
}

#[test]
fn a_missing_executable_is_refused_by_where_it_was_looked_for() {
    let root = TempDir::new().expect("makes a directory");
    let mut explicit = settings(root.path());
    let nowhere = root.path().join("nowhere").join("codex.exe");
    explicit.executable = Some(nowhere.clone());
    assert_eq!(
        refused(explicit),
        BackendError::ExecutableNotAFile { path: nowhere }
    );

    let mut relative = settings(root.path());
    relative.executable = Some(PathBuf::from("codex"));
    assert_eq!(
        refused(relative),
        BackendError::ExecutableNotAFile {
            path: PathBuf::from("codex")
        }
    );

    let mut not_on_path = settings(root.path());
    not_on_path.executable = None;
    not_on_path.path_directories = vec![root.path().join("empty-bin")];
    assert_eq!(refused(not_on_path), BackendError::ExecutableNotOnPath);
}

#[test]
fn the_executable_is_found_on_path_when_none_is_named() {
    let root = TempDir::new().expect("makes a directory");
    let mut on_path = settings(root.path());
    on_path.executable = None;
    on_path.path_directories = vec![
        root.path().join("empty-bin"),
        executable(root.path())
            .parent()
            .expect("a folder")
            .to_path_buf(),
    ];
    check(on_path).expect("found on PATH");
}

#[test]
fn a_scratch_folder_inside_a_repository_or_an_agents_folder_is_refused() {
    let root = TempDir::new().expect("makes a directory");
    let repository = root.path().join("repository");
    std::fs::create_dir_all(repository.join(".git")).expect("makes a directory");
    let mut inside_repository = settings(root.path());
    inside_repository.work_root = repository.join("jobs");
    assert_eq!(
        refused(inside_repository),
        BackendError::WorkRootUnfit {
            path: repository.join("jobs"),
            folder: repository.clone(),
        }
    );

    let instructions = root.path().join("instructions");
    std::fs::create_dir_all(&instructions).expect("makes a directory");
    std::fs::write(instructions.join("AGENTS.md"), b"# rules").expect("writes");
    let mut inside_agents = settings(root.path());
    inside_agents.work_root = instructions.join("deeper").join("jobs");
    assert_eq!(
        refused(inside_agents),
        BackendError::WorkRootUnfit {
            path: instructions.join("deeper").join("jobs"),
            folder: instructions.clone(),
        }
    );

    let mut relative = settings(root.path());
    relative.work_root = PathBuf::from("jobs");
    assert_eq!(
        refused(relative),
        BackendError::WorkRootNotAbsolute {
            path: PathBuf::from("jobs")
        }
    );
}

#[test]
fn a_scratch_folder_that_cannot_be_made_carries_its_io_error_kind() {
    let root = TempDir::new().expect("makes a directory");
    let blocker = root.path().join("blocker");
    std::fs::write(&blocker, b"a file").expect("writes");
    let mut below_a_file = settings(root.path());
    below_a_file.work_root = blocker.join("jobs");
    let error = refused(below_a_file);
    let BackendError::WorkRootUnmakeable { path, io_cause } = &error else {
        panic!("expected an unmakeable folder: {error:?}");
    };
    assert_eq!(*path, blocker.join("jobs"));
    let made = std::fs::create_dir_all(blocker.join("jobs")).expect_err("cannot be made");
    assert_eq!(*io_cause, IoCause::from_kind(made.kind()));
}

#[test]
fn the_backend_is_off_unless_both_switches_say_codex() {
    let root = TempDir::new().expect("makes a directory");
    for (mode, backend) in [
        (None, None),
        (Some("live"), None),
        (None, Some("codex-cli")),
        (Some("fake"), Some("codex-cli")),
        (Some("live"), Some("openai")),
    ] {
        let chosen = backend_for(mode, backend, settings(root.path()));
        assert_eq!(chosen.name(), "none", "{mode:?} {backend:?}");
        let error = chosen.check().expect_err("not switched on");
        assert_eq!(error, BackendError::NotSwitchedOn);
        assert!(
            error.to_string().contains("DF_IMAGERY_MODE=live"),
            "{error}"
        );
    }
    let on = backend_for(Some("live"), Some("codex-cli"), settings(root.path()));
    assert_eq!(on.name(), "codex-cli");
    on.check().expect("switched on and ready");
}

#[test]
fn each_backend_error_answers_the_status_of_its_kind_of_failure() {
    let unavailable = StatusCode::SERVICE_UNAVAILABLE;
    for (error, status) in [
        (BackendError::NotSwitchedOn, unavailable),
        (
            BackendError::NoLogin {
                home: PathBuf::from("home"),
            },
            unavailable,
        ),
        (BackendError::ExecutableNotOnPath, unavailable),
        (BackendError::UsageLimited, unavailable),
        (
            BackendError::TextTooLong {
                characters: 4001,
                maximum: 4000,
            },
            StatusCode::BAD_REQUEST,
        ),
        (BackendError::Timeout, StatusCode::GATEWAY_TIMEOUT),
        (BackendError::Interrupted, StatusCode::BAD_GATEWAY),
        (
            BackendError::Refused {
                detail: "policy".to_owned(),
            },
            StatusCode::BAD_GATEWAY,
        ),
        (BackendError::Internal, StatusCode::INTERNAL_SERVER_ERROR),
    ] {
        assert_eq!(error.status(), status, "{error:?}");
    }
}

#[test]
fn a_backend_error_key_is_its_serde_name_and_it_reads_back() {
    let errors = [
        BackendError::NotSwitchedOn,
        BackendError::ExecutableNotAFile {
            path: PathBuf::from("codex"),
        },
        BackendError::ExecutableNotOnPath,
        BackendError::ExecutableNotStartable {
            kind: "not found".to_owned(),
        },
        BackendError::NoCodexHome,
        BackendError::NoLogin {
            home: PathBuf::from("home"),
        },
        BackendError::HomeHasConfig {
            home: PathBuf::from("home"),
        },
        BackendError::WorkRootNotAbsolute {
            path: PathBuf::from("jobs"),
        },
        BackendError::WorkRootUnmakeable {
            path: PathBuf::from("jobs"),
            io_cause: IoCause::PermissionDenied,
        },
        BackendError::WorkRootUnfit {
            path: PathBuf::from("jobs"),
            folder: PathBuf::from("repository"),
        },
        BackendError::Unaccepted {
            detail: "the prompt is too long".to_owned(),
        },
        BackendError::Timeout,
        BackendError::Interrupted,
        BackendError::UsageLimited,
        BackendError::Refused {
            detail: "policy".to_owned(),
        },
        BackendError::TextTooLong {
            characters: 4001,
            maximum: 4000,
        },
        BackendError::ForgeFailed {
            detail: "rejected: Image description must contain 1–4000 characters.".to_owned(),
        },
        BackendError::Unreadable {
            kind: "not found".to_owned(),
        },
        BackendError::Internal,
    ];
    for error in errors {
        let json = serde_json::to_value(&error).expect("serializes");
        assert_eq!(json["cause"], error.key(), "{json}");
        let read: BackendError = serde_json::from_value(json).expect("reads back");
        assert_eq!(read, error);
    }
}

/// A Forge backend whose studio is never reached: nothing listens on port 9.
fn unreachable_forge() -> ForgeImageBackend {
    let config = ForgeConfig::new("http://127.0.0.1:9", "41fbb3c9-ac91-4897-b890-7a056b4ae371")
        .expect("a loopback origin and a project id");
    ForgeImageBackend::new(Ok(ForgeClient::new(config).expect("builds")))
}

#[tokio::test(flavor = "current_thread")]
async fn a_text_longer_than_forge_takes_is_refused_by_its_length_and_never_sent() {
    let forge = unreachable_forge();
    let limits = forge.limits();
    let fits = "x".repeat(FORGE_IMAGE_MAXIMUM_DESCRIPTION_CHARACTERS);
    assert_eq!(limits.check_text(&fits), Ok(()));
    // Forge counts UTF-16 code units: a character outside the basic plane is two.
    let over = format!(
        "{}\u{1F56F}",
        "x".repeat(FORGE_IMAGE_MAXIMUM_DESCRIPTION_CHARACTERS - 1)
    );
    let too_long = BackendError::TextTooLong {
        characters: FORGE_IMAGE_MAXIMUM_DESCRIPTION_CHARACTERS + 1,
        maximum: FORGE_IMAGE_MAXIMUM_DESCRIPTION_CHARACTERS,
    };
    assert_eq!(limits.check_text(&over), Err(too_long.clone()));
    let request = BackendRequest {
        text: over,
        negative: None,
        width_px: 1664,
        height_px: 928,
        base: None,
        references: Vec::new(),
        priority: Priority::Normal,
    };
    // Nothing listens, so a request that was sent would end as a transport failure, not this.
    let refused = forge
        .generate(&request, Duration::from_secs(5))
        .await
        .expect_err("refused before sending");
    assert_eq!(refused, too_long);
    assert!(refused.to_string().contains("4001"), "{refused}");
}

#[test]
fn each_backend_takes_its_own_number_of_pictures_and_only_forge_a_negative_prompt() {
    let root = TempDir::new().expect("a temporary folder");
    let codex = CodexBackend::new(settings(root.path())).limits();
    assert_eq!(codex.maximum_pictures, CODEX_MAXIMUM_PICTURES);
    assert_eq!(codex.maximum_text_characters, None);
    assert_eq!(codex.maximum_negative_characters, None);
    let forge = unreachable_forge().limits();
    assert_eq!(forge.maximum_pictures, MAXIMUM_PICTURES);
    assert_eq!(
        forge.maximum_text_characters,
        Some(FORGE_IMAGE_MAXIMUM_DESCRIPTION_CHARACTERS)
    );
    assert!(forge.maximum_negative_characters.is_some());
    assert_eq!(SwitchedOff.limits(), BackendLimits::REQUEST);
}

#[test]
fn a_forge_job_carries_its_negative_prompt_with_the_guidance_that_applies_it() {
    let mut request = BackendRequest {
        text: "a scriptorium".to_owned(),
        negative: Some("painting, readable text".to_owned()),
        width_px: 1672,
        height_px: 941,
        base: None,
        references: Vec::new(),
        priority: Priority::Normal,
    };
    let staged = ["0f8e2a8e-7c1d-4b55-9d0e-2a3b4c5d6e7f.png".to_owned()];
    let job = forge_image_job(&request, &staged, BackendTuning::default());
    assert_eq!(job["prompt"], "a scriptorium");
    assert_eq!(job["negative_prompt"], "painting, readable text");
    assert_eq!(job["true_cfg_scale"], FORGE_IMAGE_NEGATIVE_GUIDANCE);
    assert_eq!(job["references"][0], staged[0]);
    assert_eq!(
        (job["width"].as_u64(), job["height"].as_u64()),
        (Some(1664), Some(928))
    );
    // Forge refuses guidance without a negative prompt, so neither goes alone.
    request.negative = None;
    let plain = forge_image_job(&request, &[], BackendTuning::default());
    assert!(plain.get("negative_prompt").is_none() && plain.get("true_cfg_scale").is_none());
}

#[test]
fn a_forge_job_takes_the_set_size_and_steps_and_without_them_the_request_s_own() {
    let request = BackendRequest {
        text: "a scriptorium".to_owned(),
        negative: None,
        width_px: 1672,
        height_px: 941,
        base: None,
        references: Vec::new(),
        priority: Priority::Normal,
    };
    let size = |text: &str| Some(text.parse::<PictureSize>().expect("a size"));
    let sides = |job: &serde_json::Value| (job["width"].as_u64(), job["height"].as_u64());
    // Unset: the request's own size, and no steps sent, so the studio's default applies.
    let default = forge_image_job(&request, &[], BackendTuning::default());
    assert_eq!(sides(&default), (Some(1664), Some(928)));
    assert!(default.get("num_inference_steps").is_none());
    // Set: the size replaces the shape's, whatever aspect the request had, and the steps go out.
    let tuning = BackendTuning {
        size: size("512x512"),
        steps: Some(InferenceSteps::new(20).expect("steps")),
    };
    let small = forge_image_job(&request, &[], tuning);
    assert_eq!(sides(&small), (Some(512), Some(512)));
    assert_eq!(small["num_inference_steps"], 20);
    // Forge's own rounding and clamping apply to a size it does not take as given.
    let odd = BackendTuning {
        size: size("500x9000"),
        steps: None,
    };
    let clamped = forge_image_job(&request, &[], odd);
    assert_eq!(sides(&clamped), (Some(512), Some(2752)));
    assert!(clamped.get("num_inference_steps").is_none());
    // The backend reports what it was given; a plain Forge backend and the off switch report none.
    assert_eq!(unreachable_forge().with_tuning(tuning).tuning(), tuning);
    assert_eq!(unreachable_forge().tuning(), BackendTuning::default());
    assert_eq!(SwitchedOff.tuning(), BackendTuning::default());
}
