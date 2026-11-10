//! The remembered switches: a first run is safe, a change is kept across a restart, and a file
//! that cannot be taken never turns a paid switch on.

use axum::http::StatusCode;
use df_playground::controls::{Controls, ControlsChange};
use df_playground::copy::ReadingPace;
use df_playground::path::CONTROLS_CHANGE;
use tempfile::TempDir;

use super::{Restore, SETTINGS_FILE, SettingsError, StoredSettings, restore};
use crate::io_cause::IoCause;
use crate::playground::{PlaygroundFiles, PlaygroundState};
use crate::test_support::{Running, STARTER_FILES, start};

/// A host started again over `assets`: its switches are whatever that directory remembers.
fn restarted(assets: std::path::PathBuf) -> PlaygroundState {
    PlaygroundState::new(
        PlaygroundFiles::new(assets, "pkg".into()),
        "boot".to_owned(),
    )
}

fn controls_of(state: &PlaygroundState) -> Controls {
    state.snapshot().controls
}

async fn choose(running: &Running, change: ControlsChange) -> StatusCode {
    let body = serde_json::to_string(&change).expect("writes the change");
    crate::client::post_json(running.port, CONTROLS_CHANGE, &body)
        .await
        .expect("answers")
        .status
}

fn settings_lines(running: &Running) -> Vec<String> {
    running
        .state
        .log()
        .newest(1_000)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("PLAYGROUND settings"))
        .collect()
}

#[test]
fn the_stored_switches_are_exactly_the_default_controls_when_nothing_is_stored() {
    assert_eq!(
        StoredSettings::default(),
        StoredSettings::of(&Controls::default())
    );
}

#[test]
fn a_first_run_with_no_saved_file_starts_with_everything_that_spends_money_off() {
    let empty = TempDir::new().expect("makes a directory");
    assert_eq!(restore(empty.path()), Restore::Defaults);
    let state = restarted(empty.path().to_path_buf());
    let controls = controls_of(&state);
    assert!(!controls.tts_on && !controls.stt_on && !controls.audio.music_on);
    assert!(controls.audio.sound_on, "sound is harmless");
    assert!(!controls.show_unavailable && !controls.show_grid && !controls.show_sam);
    let line = state.restore_line();
    assert!(
        line.starts_with("PLAYGROUND settings_restored source=defaults"),
        "{line}"
    );
    assert!(
        line.contains("tts=false stt=false sound=true music=false"),
        "{line}"
    );
}

#[tokio::test]
async fn every_switch_is_kept_across_a_restart_exactly_as_the_owner_left_it() {
    let running = start(&STARTER_FILES).await;
    let assets = running.playground_path("assets");
    for change in [
        ControlsChange::Tts(true),
        ControlsChange::Stt(true),
        ControlsChange::Music(true),
        ControlsChange::Sound(false),
        ControlsChange::Volume(35),
        ControlsChange::ShowUnavailable(true),
        ControlsChange::ShowGrid(true),
        ControlsChange::ShowSam(true),
    ] {
        assert_eq!(
            choose(&running, change.clone()).await,
            StatusCode::OK,
            "{change:?}"
        );
    }
    let after = controls_of(&restarted(assets.clone()));
    assert!(after.tts_on && after.stt_on && after.show_unavailable);
    assert!(after.show_grid && after.show_sam);
    assert!(after.audio.music_on && !after.audio.sound_on);
    assert_eq!(after.audio.volume_percent, 35);

    // The owner turns TTS off: the next start keeps it off, with the rest as it was.
    assert_eq!(
        choose(&running, ControlsChange::Tts(false)).await,
        StatusCode::OK
    );
    let later = restarted(assets);
    let kept = controls_of(&later);
    assert!(!kept.tts_on, "TTS stays off after a restart");
    assert!(kept.audio.music_on && kept.stt_on);
    assert!(later.restore_line().contains("source=file"));
}

#[tokio::test]
async fn a_change_the_host_does_not_remember_writes_no_file() {
    let running = start(&STARTER_FILES).await;
    let paced = ControlsChange::Pace(ReadingPace::Fast);
    assert_eq!(choose(&running, paced).await, StatusCode::OK);
    assert!(
        !running
            .playground_path("assets")
            .join(SETTINGS_FILE)
            .exists()
    );
    assert!(settings_lines(&running).is_empty());
}

#[tokio::test]
async fn a_saved_change_writes_one_audit_line_naming_the_switches() {
    let running = start(&STARTER_FILES).await;
    assert_eq!(
        choose(&running, ControlsChange::Tts(true)).await,
        StatusCode::OK
    );
    let lines = settings_lines(&running);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("PLAYGROUND settings_saved unavailable=false"));
    assert!(lines[0].contains("tts=true stt=false sound=true music=false volume=80"));
}

#[test]
fn a_file_that_does_not_read_leaves_every_switch_off_and_is_named() {
    for text in [
        "not json",
        r#"{"tts_on": true, "surprise": 1}"#,
        r#"{"tts_on": "yes"}"#,
    ] {
        let directory = TempDir::new().expect("makes a directory");
        std::fs::write(directory.path().join(SETTINGS_FILE), text).expect("writes");
        assert_eq!(
            restore(directory.path()),
            Restore::Unusable(SettingsError::Malformed),
            "{text}"
        );
        let state = restarted(directory.path().to_path_buf());
        assert_eq!(controls_of(&state), Controls::default(), "{text}");
        let line = state.restore_line();
        assert!(
            line.starts_with(
                "PLAYGROUND settings_restored source=defaults_after_error error=malformed"
            ),
            "{line}"
        );
    }
}

#[test]
fn a_settings_file_that_is_a_directory_is_unreadable_not_silently_default() {
    let directory = TempDir::new().expect("makes a directory");
    std::fs::create_dir(directory.path().join(SETTINGS_FILE)).expect("makes the directory");
    match restore(directory.path()) {
        Restore::Unusable(SettingsError::Unreadable { cause }) => {
            assert!(IoCause::ALL.contains(&cause));
        }
        other => panic!("expected an unreadable file, got {other:?}"),
    }
}

#[test]
fn a_file_that_lacks_a_key_keeps_that_switch_at_its_safe_default() {
    let directory = TempDir::new().expect("makes a directory");
    std::fs::write(
        directory.path().join(SETTINGS_FILE),
        r#"{"show_grid": true}"#,
    )
    .expect("writes");
    let controls = controls_of(&restarted(directory.path().to_path_buf()));
    assert!(controls.show_grid);
    assert!(!controls.tts_on && !controls.stt_on && !controls.audio.music_on);
    assert!(controls.audio.sound_on);
}

#[tokio::test]
async fn a_switch_that_cannot_be_saved_is_reported_and_logged_not_hidden() {
    let running = start(&STARTER_FILES).await;
    let blocked = running.playground_path("assets").join(SETTINGS_FILE);
    std::fs::create_dir_all(&blocked).expect("a directory where the file goes");
    let status = choose(&running, ControlsChange::Tts(true)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let lines = settings_lines(&running);
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("PLAYGROUND settings_unsaved error=io")),
        "{lines:?}"
    );
}
