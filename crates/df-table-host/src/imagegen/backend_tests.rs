//! The preconditions of the Codex backend: each one fails with its own typed error and says what
//! the owner must do. Nothing here starts Codex or opens a login file.

use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use tempfile::TempDir;

use super::backend::{BackendError, CodexBackend, CodexSettings, ImageBackend, backend_for};
use crate::locate::IoCause;

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
