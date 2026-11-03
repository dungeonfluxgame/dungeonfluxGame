//! The preconditions of the Codex backend: each one fails with its own typed cause and says what
//! the owner must do. Nothing here starts Codex or opens a login file.

use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::backend::{CodexBackend, CodexSettings, ImageBackend, backend_for};
use super::model::{BackendError, RefusalCause};

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

fn cause(settings: CodexSettings) -> RefusalCause {
    check(settings).expect_err("refused").cause()
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
fn no_codex_home_is_not_signed_in_and_says_how_to_start_the_host() {
    let root = TempDir::new().expect("makes a directory");
    let mut without = settings(root.path());
    without.codex_home = None;
    let error = check(without).expect_err("refused");
    assert_eq!(error.cause(), RefusalCause::NotSignedIn);
    assert!(
        error.message().contains("CODEX_HOME"),
        "{}",
        error.message()
    );
}

#[test]
fn a_home_without_a_login_file_says_to_run_codex_login() {
    let root = TempDir::new().expect("makes a directory");
    let mut signed_out = settings(root.path());
    let empty = root.path().join("empty-home");
    std::fs::create_dir_all(&empty).expect("makes a directory");
    signed_out.codex_home = Some(empty.clone());
    let error = check(signed_out).expect_err("refused");
    assert_eq!(error.cause(), RefusalCause::NotSignedIn);
    assert!(
        error.message().contains("codex login"),
        "{}",
        error.message()
    );
    assert!(error.message().contains(&empty.display().to_string()));

    let mut missing = settings(root.path());
    missing.codex_home = Some(root.path().join("no-such-home"));
    assert_eq!(cause(missing), RefusalCause::NotSignedIn);
}

#[test]
fn a_home_with_a_config_file_is_not_the_dedicated_home() {
    let root = TempDir::new().expect("makes a directory");
    let with_config = settings(root.path());
    let home = with_config.codex_home.clone().expect("a home");
    std::fs::write(home.join("config.toml"), b"model = \"x\"").expect("writes");
    assert_eq!(cause(with_config), RefusalCause::NotConfigured);
}

#[test]
fn a_missing_executable_is_codex_missing() {
    let root = TempDir::new().expect("makes a directory");
    let mut explicit = settings(root.path());
    explicit.executable = Some(root.path().join("nowhere").join("codex.exe"));
    assert_eq!(cause(explicit), RefusalCause::CodexMissing);

    let mut relative = settings(root.path());
    relative.executable = Some(PathBuf::from("codex"));
    assert_eq!(cause(relative), RefusalCause::CodexMissing);

    let mut not_on_path = settings(root.path());
    not_on_path.executable = None;
    not_on_path.path_directories = vec![root.path().join("empty-bin")];
    let error = check(not_on_path).expect_err("refused");
    assert_eq!(error.cause(), RefusalCause::CodexMissing);
    assert!(error.message().contains("PATH"));
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
    let error = check(inside_repository).expect_err("refused");
    assert_eq!(error.cause(), RefusalCause::NotConfigured);
    assert!(error.message().contains("DF_IMAGERY_CODEX_WORKDIR"));

    let instructions = root.path().join("instructions");
    std::fs::create_dir_all(&instructions).expect("makes a directory");
    std::fs::write(instructions.join("AGENTS.md"), b"# rules").expect("writes");
    let mut inside_agents = settings(root.path());
    inside_agents.work_root = instructions.join("deeper").join("jobs");
    assert_eq!(cause(inside_agents), RefusalCause::NotConfigured);

    let mut relative = settings(root.path());
    relative.work_root = PathBuf::from("jobs");
    assert_eq!(cause(relative), RefusalCause::NotConfigured);
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
        let chosen = backend_for(
            mode.map(str::to_owned),
            backend.map(str::to_owned),
            settings(root.path()),
        );
        assert_eq!(chosen.name(), "none", "{mode:?} {backend:?}");
        let error = chosen.check().expect_err("not switched on");
        assert_eq!(error.cause(), RefusalCause::NotConfigured);
        assert!(error.message().contains("DF_IMAGERY_MODE=live"));
    }
    let on = backend_for(
        Some("live".to_owned()),
        Some("codex-cli".to_owned()),
        settings(root.path()),
    );
    assert_eq!(on.name(), "codex-cli");
    on.check().expect("switched on and ready");
}
