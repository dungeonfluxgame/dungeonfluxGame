//! The tempo routes and the CLI over a fake composer that counts its calls (no live call): the
//! load, prepare, impulse, reset and state round trips, the not-prepared skip, a held change
//! carried out by the scheduled wake-up, and the fade plan.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use df_playground::controls::ControlsChange;
use df_playground::scene::Track as SceneTrack;
use serde_json::Value;
use tempfile::TempDir;
use tokio::task::JoinHandle;

use super::cli::{TempoCommand, run};
use super::request::{ImpulseRequest, TempoError};
use super::routes::{Audible, fade_plan, router};
use crate::dev::{DevError, ReplyError};
use crate::music::{ComposeFuture, Composer, MusicError, MusicProviderCause, MusicService};
use crate::test_support::{Running, STARTER_FILES, start};
use crate::wire::ControlsReply;

const SHIPPED: &str = include_str!("../../../../development/market-threads/free-roam.music.json");
const MP3: &[u8] = b"ID3 not really music";

/// Answers every call with a few bytes, or the supplier's refusal on the `fail_on`th call.
struct FakeComposer {
    calls: AtomicUsize,
    fail_on: Option<usize>,
}

impl Composer for FakeComposer {
    fn provider(&self) -> &'static str {
        "fake"
    }

    fn model(&self) -> &'static str {
        "fake_music"
    }

    fn compose<'a>(
        &'a self,
        _prompt: &'a str,
        _length_ms: u32,
        _deadline: Duration,
    ) -> ComposeFuture<'a> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let fails = self.fail_on == Some(call);
        Box::pin(async move {
            if fails {
                Err(MusicError::Provider {
                    cause: MusicProviderCause::Capacity,
                })
            } else {
                Ok(MP3.to_vec())
            }
        })
    }
}

/// A host with a shown fixture, the tempo routes over a fake composer on a port of their own,
/// and a table file made for fast tests: no crossfade and a dwell of 1.5 s.
struct Harness {
    running: Running,
    composer: Arc<FakeComposer>,
    port: u16,
    table_file: std::path::PathBuf,
    _directory: TempDir,
    server: JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn harness(fail_on: Option<usize>) -> Harness {
    let running = start(&STARTER_FILES).await;
    // The music starts off, and a prepare composes only while it is on.
    running.switch_music_on();
    running.write_asset(
        "fixtures/one.json",
        concat!(
            r#"{"name":"One","tv":{"place":"Here","status":{"kind":"ok"},"elements":[]},"#,
            r#""phone":{"hero":"Pip","status":{"kind":"ok"}}}"#
        ),
    );
    crate::client::get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    let composer = Arc::new(FakeComposer {
        calls: AtomicUsize::new(0),
        fail_on,
    });
    let routes = router(
        Arc::clone(&running.state),
        MusicService::with_composer(Arc::clone(&composer) as Arc<dyn Composer>),
    );
    let listener = crate::server::bind(0).await.expect("binds");
    let port = listener.local_addr().expect("has an address").port();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, routes).await;
    });
    let mut table: Value = serde_json::from_str(SHIPPED).expect("is JSON");
    table["director"]["min_dwell_ms"] = 1_500.into();
    table["director"]["tick_ms"] = 250.into();
    for profile in table["profiles"].as_array_mut().expect("profiles") {
        profile["crossfade_ms"] = 0.into();
    }
    let directory = TempDir::new().expect("makes a directory");
    let table_file = directory.path().join("table.json");
    std::fs::write(&table_file, table.to_string()).expect("writes the table");
    Harness {
        running,
        composer,
        port,
        table_file,
        _directory: directory,
        server,
    }
}

impl Harness {
    async fn run(&self, command: TempoCommand) -> Result<String, DevError> {
        run(self.port, command).await
    }

    async fn ok(&self, command: TempoCommand) -> String {
        self.run(command).await.expect("the command succeeds")
    }

    async fn impulse(&self, kind: &str, intensity: Option<u16>) -> Result<String, DevError> {
        self.run(TempoCommand::Impulse(ImpulseRequest {
            kind: kind.to_owned(),
            intensity,
            note: None,
        }))
        .await
    }

    async fn music(&self) -> Option<SceneTrack> {
        let response = crate::client::get(self.running.port, "/playground/controls.json")
            .await
            .expect("answers");
        let reply: ControlsReply = serde_json::from_str(&response.body).expect("controls");
        reply.scene.music.set_value().cloned()
    }

    /// Waits up to five seconds for the scene's music to be a track starting with `prefix`.
    async fn music_becomes(&self, prefix: &str) -> SceneTrack {
        for _ in 0..100 {
            if let Some(track) = self.music().await
                && track.media.starts_with(prefix)
            {
                return track;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the music never became {prefix}: {:?}", self.music().await);
    }

    fn audit(&self) -> Vec<String> {
        self.running
            .state
            .log()
            .newest(400)
            .into_iter()
            .map(|line| line.text)
            .filter(|text| text.starts_with("TEMPO"))
            .collect()
    }
}

fn refused(result: Result<String, DevError>) -> TempoError {
    match result {
        Err(DevError::Rejected {
            error: ReplyError::Tempo(error),
            ..
        }) => error,
        other => panic!("expected a tempo refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn without_a_table_every_verb_but_load_says_so() {
    let harness = harness(None).await;
    assert_eq!(
        refused(harness.run(TempoCommand::State).await),
        TempoError::NoTable
    );
    assert_eq!(
        refused(harness.impulse("look", None).await),
        TempoError::NoTable
    );
    assert_eq!(
        refused(harness.run(TempoCommand::Tick).await),
        TempoError::NoTable
    );
    assert_eq!(
        refused(harness.run(TempoCommand::Reset).await),
        TempoError::NoTable
    );
    assert_eq!(
        refused(harness.run(TempoCommand::Prepare).await),
        TempoError::NoTable
    );
    assert!(
        harness
            .audit()
            .iter()
            .any(|line| line == "TEMPO refused verb=state error=no_table")
    );
}

#[tokio::test]
async fn a_table_that_does_not_read_is_refused_and_changes_nothing() {
    let harness = harness(None).await;
    let broken = harness.table_file.with_file_name("broken.json");
    std::fs::write(&broken, r#"{"version": 1}"#).expect("writes");
    assert!(matches!(
        refused(harness.run(TempoCommand::Load(broken)).await),
        TempoError::InvalidBody(_)
    ));
    assert_eq!(
        refused(harness.run(TempoCommand::State).await),
        TempoError::NoTable
    );
}

#[tokio::test]
async fn bad_impulses_are_refused_with_their_typed_errors() {
    let harness = harness(None).await;
    let loaded = harness
        .ok(TempoCommand::Load(harness.table_file.clone()))
        .await;
    assert!(loaded.starts_with("ok table version=1 base=journey profiles=show,market,study"));
    assert_eq!(
        refused(harness.impulse("jump", None).await),
        TempoError::UnknownImpulse {
            kind: "jump".to_owned()
        }
    );
    assert_eq!(
        refused(harness.impulse("look", Some(1_001)).await),
        TempoError::InvalidIntensity { intensity: 1_001 }
    );
    let noted = TempoCommand::Impulse(ImpulseRequest {
        kind: "look".to_owned(),
        intensity: None,
        note: Some("bad\u{7}note".to_owned()),
    });
    assert!(matches!(
        refused(harness.run(noted).await),
        TempoError::InvalidNote { .. }
    ));
    let state = harness.ok(TempoCommand::State).await;
    assert!(state.contains("recent: 0"), "{state}");
}

#[tokio::test]
async fn an_unprepared_table_plays_nothing_and_says_why() {
    let harness = harness(None).await;
    harness
        .ok(TempoCommand::Load(harness.table_file.clone()))
        .await;
    let reset = harness.ok(TempoCommand::Reset).await;
    assert!(
        reset.contains("decision=skip reason=not_prepared wanted=journey"),
        "{reset}"
    );
    assert_eq!(harness.music().await, None);
    assert!(
        harness
            .audit()
            .iter()
            .any(|line| line == "TEMPO cue_skipped profile=journey reason=not_prepared")
    );
    assert_eq!(
        harness.composer.calls.load(Ordering::SeqCst),
        0,
        "nothing is composed at run time"
    );
}

#[tokio::test]
async fn prepare_composes_each_profile_once_and_a_second_prepare_reuses_them() {
    let harness = harness(None).await;
    harness
        .ok(TempoCommand::Load(harness.table_file.clone()))
        .await;
    let first = harness.ok(TempoCommand::Prepare).await;
    assert!(
        first.starts_with(
            "prepared 6 profiles: 0 reused, 6 composed (465 s of music asked), 0 failed"
        ),
        "{first}"
    );
    assert_eq!(harness.composer.calls.load(Ordering::SeqCst), 6);
    let second = harness.ok(TempoCommand::Prepare).await;
    assert!(
        second.starts_with("prepared 6 profiles: 6 reused, 0 composed (0 s of music asked)"),
        "{second}"
    );
    assert_eq!(harness.composer.calls.load(Ordering::SeqCst), 6);
    assert!(second.contains("show  tempo-show-"), "{second}");
}

#[tokio::test]
async fn prepare_composes_nothing_while_the_music_is_off_and_names_each_profile_refused() {
    let harness = harness(None).await;
    harness
        .running
        .state
        .playground()
        .change_all(vec![ControlsChange::Music(false)], &[], 0, |_| {})
        .expect("the switch is changed");
    harness
        .ok(TempoCommand::Load(harness.table_file.clone()))
        .await;
    let report = harness.ok(TempoCommand::Prepare).await;
    assert!(
        report.starts_with(
            "prepared 6 profiles: 0 reused, 0 composed (0 s of music asked), 6 failed"
        ),
        "{report}"
    );
    assert_eq!(report.matches("failed (disabled)").count(), 6, "{report}");
    assert_eq!(
        harness.composer.calls.load(Ordering::SeqCst),
        0,
        "no composition was sent"
    );
}

#[tokio::test]
async fn a_failed_composition_is_reported_and_the_others_still_go_on() {
    let harness = harness(Some(2)).await;
    harness
        .ok(TempoCommand::Load(harness.table_file.clone()))
        .await;
    let report = harness.ok(TempoCommand::Prepare).await;
    assert!(
        report.starts_with("prepared 6 profiles: 0 reused, 5 composed"),
        "{report}"
    );
    assert!(report.contains("1 failed"), "{report}");
    assert!(report.contains("market  tempo-market-"), "{report}");
    assert!(report.contains("failed (provider)"), "{report}");
    // The base is ready, so the journey can start; the failed profile is skipped when wanted.
    harness.ok(TempoCommand::Reset).await;
    harness.music_becomes("tempo-journey-").await;
    tokio::time::sleep(Duration::from_millis(1_600)).await;
    let skipped = harness
        .impulse("trade", None)
        .await
        .expect("the impulse is taken");
    assert!(
        skipped.contains("decision=skip reason=not_prepared wanted=market"),
        "{skipped}"
    );
}

#[tokio::test]
async fn the_journey_starts_on_the_base_and_a_held_change_is_made_by_the_scheduled_wake() {
    let harness = harness(None).await;
    harness
        .ok(TempoCommand::Load(harness.table_file.clone()))
        .await;
    harness.ok(TempoCommand::Prepare).await;

    let reset = harness.ok(TempoCommand::Reset).await;
    assert!(
        reset.contains("decision=switch reason=start wanted=journey"),
        "{reset}"
    );
    let base = harness.music_becomes("tempo-journey-").await;
    assert_eq!((base.volume_percent, base.looping), (22, true));

    let arrived = harness
        .impulse("arrive_road", Some(1_000))
        .await
        .expect("taken");
    assert!(arrived.starts_with("tempo phase=build "), "{arrived}");
    assert!(
        arrived.contains("decision=hold reason=dwell wanted=road until_ms="),
        "{arrived}"
    );
    assert!(
        arrived.contains("cue=journey track=tempo-journey-"),
        "{arrived}"
    );
    let road = harness.music_becomes("tempo-road-").await;
    assert_eq!(road.volume_percent, 24);

    let state = harness.ok(TempoCommand::State).await;
    assert!(state.contains("cue=road track=tempo-road-"), "{state}");
    assert!(state.contains("arrive_road x1000"), "{state}");
    let audit = harness.audit();
    assert!(audit.iter().any(|line| line.starts_with(
        "TEMPO impulse kind=arrive_road strength=1000 note_chars=0 phase=build intensity="
    )));
    assert!(
        audit
            .iter()
            .any(|line| line.starts_with("TEMPO cue profile=road track=tempo-road-"))
    );
    assert!(
        audit
            .iter()
            .any(|line| line.starts_with("TEMPO decision action=hold reason=dwell"))
    );
}

#[test]
fn a_track_change_dips_the_old_track_out_and_the_new_one_in() {
    let old = Audible {
        file: "journey.mp3".to_owned(),
        volume_percent: 22,
    };
    let plan = |from: Option<&Audible>, to: &str, volume: u8, crossfade_ms: u32| {
        fade_plan(from, to, volume, crossfade_ms)
            .into_iter()
            .map(|step| (step.file, step.volume_percent, step.wait_ms))
            .collect::<Vec<_>>()
    };
    let step = |file: &str, volume: u8, wait: u64| (file.to_owned(), volume, wait);
    assert_eq!(
        plan(Some(&old), "market.mp3", 25, 4_200),
        [
            step("journey.mp3", 16, 600),
            step("journey.mp3", 11, 600),
            step("journey.mp3", 5, 600),
            step("market.mp3", 6, 600),
            step("market.mp3", 12, 600),
            step("market.mp3", 18, 600),
            step("market.mp3", 25, 0),
        ]
    );
    assert_eq!(
        plan(None, "market.mp3", 25, 4_000),
        [
            step("market.mp3", 6, 1_000),
            step("market.mp3", 12, 1_000),
            step("market.mp3", 18, 1_000),
            step("market.mp3", 25, 0),
        ]
    );
    assert_eq!(
        plan(Some(&old), "market.mp3", 25, 0),
        [step("market.mp3", 25, 0)]
    );
    assert_eq!(
        plan(Some(&old), "journey.mp3", 22, 4_000),
        [step("journey.mp3", 22, 0)]
    );
}
