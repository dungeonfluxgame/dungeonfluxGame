//! `scene music generate`: its grammar, the reuse-or-compose choice, and the flow over a fake
//! composer that counts its calls (no live call).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use df_playground::media::{MediaEntry, MediaKind};
use df_playground::scene::{SceneChange, Target, Track as SceneTrack};

use super::backend::{ComposeFuture, Composer};
use super::request::{MusicError, MusicProviderCause, MusicRequest, MusicSource};
use super::routes::{Track, TrackLocks, choose_track, play};
use crate::command::{Command, CommandError, DEFAULT_VOLUME, parse, split_words};
use crate::media_library::media_entries;
use crate::test_support::{Running, STARTER_FILES, start};
use crate::wire::ControlsReply;

const MP3: &[u8] = b"ID3 not really music";

fn command(line: &str) -> Result<Command, CommandError> {
    parse(&split_words(line).expect("the line splits"))
}

fn request(name: &str) -> MusicRequest {
    MusicRequest {
        name: name.to_owned(),
        prompt: "slow dungeon strings".to_owned(),
        length_seconds: 120,
        target: Target::Both,
        volume_percent: DEFAULT_VOLUME,
    }
}

// ---- grammar

#[test]
fn generate_reads_its_keys_anywhere_and_every_other_word_is_the_prompt() {
    assert_eq!(
        command("scene music generate as=crypt slow, dark strings: no vocals"),
        Ok(Command::MusicGenerate(MusicRequest {
            name: "crypt".to_owned(),
            prompt: "slow, dark strings: no vocals".to_owned(),
            length_seconds: 120,
            target: Target::Both,
            volume_percent: DEFAULT_VOLUME,
        }))
    );
    assert_eq!(
        command("scene music generate low drone seconds=30 as=cell_2 on=tv volume=40 mood=grim"),
        Ok(Command::MusicGenerate(MusicRequest {
            name: "cell_2".to_owned(),
            prompt: "low drone mood=grim".to_owned(),
            length_seconds: 30,
            target: Target::Tv,
            volume_percent: 40,
        }))
    );
    assert_eq!(
        command("scene music crypt.mp3"),
        Ok(Command::Scene(SceneChange::Music {
            media: "crypt.mp3".to_owned(),
            target: Target::Both,
            volume_percent: DEFAULT_VOLUME,
            looping: true,
        }))
    );
}

#[test]
fn generate_without_a_name_or_a_prompt_or_with_a_bad_value_is_refused() {
    for line in [
        "scene music generate",
        "scene music generate slow strings",
        "scene music generate as=crypt",
    ] {
        assert!(
            matches!(command(line), Err(CommandError::Usage(_))),
            "{line}"
        );
    }
    for (line, key) in [
        ("scene music generate as=crypt seconds=2 strings", "seconds"),
        (
            "scene music generate as=crypt seconds=601 strings",
            "seconds",
        ),
        (
            "scene music generate as=crypt seconds=long strings",
            "seconds",
        ),
        ("scene music generate as=crypt.mp3 strings", "as"),
        ("scene music generate as=crypt volume=101 strings", "volume"),
        ("scene music generate as=crypt on=watch strings", "on"),
    ] {
        let refused = command(line);
        assert!(
            matches!(&refused, Err(CommandError::BadChoice { key: found, .. }) if found == key),
            "{line}: {refused:?}"
        );
    }
    assert_eq!(
        command("scene music generate as=a as=b strings"),
        Err(CommandError::DuplicateKey("as".to_owned()))
    );
}

// ---- reuse or compose

fn entry(name: &str, kind: MediaKind) -> MediaEntry {
    MediaEntry {
        name: name.to_owned(),
        kind,
        bytes: 7,
    }
}

#[test]
fn a_sound_of_the_name_is_reused_and_anything_else_is_composed_as_mp3() {
    let compose = Track::Compose {
        file: "crypt.mp3".to_owned(),
    };
    assert_eq!(choose_track("crypt", &[]), compose);
    assert_eq!(
        choose_track(
            "crypt",
            &[
                entry("crypt.png", MediaKind::Image),
                entry("crypt-old.mp3", MediaKind::Audio),
                entry("crypts.mp3", MediaKind::Audio),
            ]
        ),
        compose
    );
    assert_eq!(
        choose_track(
            "crypt",
            &[
                entry("crypt.ogg", MediaKind::Audio),
                entry("crypt.png", MediaKind::Image),
            ]
        ),
        Track::Library {
            file: "crypt.ogg".to_owned(),
            bytes: 7,
        }
    );
}

// ---- the flow over a fake composer

/// What the fake answers each call.
enum Answer {
    Music,
    Fail(MusicProviderCause),
    Hang,
}

struct FakeComposer {
    answer: Answer,
    calls: AtomicUsize,
}

impl FakeComposer {
    fn new(answer: Answer) -> Self {
        Self {
            answer,
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            match self.answer {
                Answer::Music => Ok(MP3.to_vec()),
                Answer::Fail(cause) => Err(MusicError::Provider { cause }),
                Answer::Hang => std::future::pending().await,
            }
        })
    }
}

async fn with_fixture(running: &Running) {
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
}

async fn music(running: &Running) -> Option<SceneTrack> {
    let response = crate::client::get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    let reply: ControlsReply = serde_json::from_str(&response.body).expect("controls");
    reply.scene.music.set_value().cloned()
}

fn lines(running: &Running) -> Vec<String> {
    running
        .state
        .log()
        .newest(200)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("MUSIC"))
        .collect()
}

fn library(running: &Running) -> Vec<String> {
    media_entries(&running.playground_path("assets/media"))
        .expect("lists the library")
        .into_iter()
        .map(|entry| entry.name)
        .collect()
}

const DEADLINE: Duration = Duration::from_secs(60);

#[tokio::test]
async fn a_new_name_is_composed_once_and_the_same_name_plays_again_with_no_second_call() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    let composer = FakeComposer::new(Answer::Music);

    let first = play(
        &running.state,
        &composer,
        &TrackLocks::default(),
        request("crypt"),
        DEADLINE,
    )
    .await
    .expect("composes and plays");
    assert_eq!(composer.calls(), 1);
    assert_eq!(first.source, MusicSource::Composed);
    assert_eq!(first.file, "crypt.mp3");
    assert_eq!(first.length_ms, Some(120_000));
    assert_eq!(first.bytes, MP3.len() as u64);
    assert_eq!(library(&running), ["crypt.mp3"]);
    let playing = music(&running).await.expect("music is set");
    assert_eq!(
        (playing.media.as_str(), playing.looping),
        ("crypt.mp3", true)
    );

    let again = play(
        &running.state,
        &composer,
        &TrackLocks::default(),
        request("crypt"),
        DEADLINE,
    )
    .await
    .expect("plays again");
    assert_eq!(
        composer.calls(),
        1,
        "the same scene's track is not composed again"
    );
    assert_eq!(again.source, MusicSource::Library);
    assert_eq!((again.length_ms, again.compose_ms), (None, None));

    let audit = lines(&running);
    assert!(audit[0].starts_with("MUSIC compose name=crypt file=crypt.mp3 length_ms=120000"));
    assert!(audit[1].starts_with("MUSIC stored name=crypt file=crypt.mp3 bytes="));
    assert!(audit[1].contains("provider=fake model=fake_music"));
    assert!(audit[2].starts_with("MUSIC reused name=crypt file=crypt.mp3"));
    assert!(
        audit.iter().all(|line| !line.contains("dungeon")),
        "{audit:?}"
    );
}

#[tokio::test]
async fn two_requests_at_once_for_a_new_name_compose_it_once() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    let composer = FakeComposer::new(Answer::Music);
    let tracks = TrackLocks::default();
    let (first, second) = tokio::join!(
        play(
            &running.state,
            &composer,
            &tracks,
            request("crypt"),
            DEADLINE
        ),
        play(
            &running.state,
            &composer,
            &tracks,
            request("crypt"),
            DEADLINE
        ),
    );
    assert_eq!(composer.calls(), 1, "the track is paid for once");
    let sources = [first.expect("plays").source, second.expect("plays").source];
    assert_eq!(sources, [MusicSource::Composed, MusicSource::Library]);
    assert_eq!(library(&running), ["crypt.mp3"]);
}

#[tokio::test]
async fn a_failed_or_late_composition_stores_nothing_and_keeps_the_music_playing() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    play(
        &running.state,
        &FakeComposer::new(Answer::Music),
        &TrackLocks::default(),
        request("hall"),
        DEADLINE,
    )
    .await
    .expect("plays the first scene's music");

    let failing = FakeComposer::new(Answer::Fail(MusicProviderCause::Capacity));
    assert_eq!(
        play(
            &running.state,
            &failing,
            &TrackLocks::default(),
            request("crypt"),
            DEADLINE
        )
        .await,
        Err(MusicError::Provider {
            cause: MusicProviderCause::Capacity
        })
    );
    let hanging = FakeComposer::new(Answer::Hang);
    assert_eq!(
        play(
            &running.state,
            &hanging,
            &TrackLocks::default(),
            request("crypt"),
            Duration::from_millis(20)
        )
        .await,
        Err(MusicError::Provider {
            cause: MusicProviderCause::Deadline
        })
    );
    assert_eq!(library(&running), ["hall.mp3"]);
    assert_eq!(
        music(&running).await.map(|track| track.media),
        Some("hall.mp3".to_owned())
    );
    assert!(lines(&running).iter().any(|line| {
        line.starts_with("MUSIC compose_failed name=crypt error=provider cause=deadline")
    }));
}

#[tokio::test]
async fn an_invalid_request_is_refused_before_any_call() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    let composer = FakeComposer::new(Answer::Music);
    let mut long = request("crypt");
    long.length_seconds = 601;
    assert_eq!(
        play(
            &running.state,
            &composer,
            &TrackLocks::default(),
            long,
            DEADLINE
        )
        .await,
        Err(MusicError::InvalidLength {
            length_seconds: 601
        })
    );
    let mut blank = request("crypt");
    blank.prompt = "  ".to_owned();
    assert_eq!(
        play(
            &running.state,
            &composer,
            &TrackLocks::default(),
            blank,
            DEADLINE
        )
        .await,
        Err(MusicError::InvalidPrompt {
            prompt_characters: 2
        })
    );
    assert_eq!(composer.calls(), 0);
}

#[test]
fn the_source_and_cause_keys_are_their_serde_names() {
    for source in MusicSource::ALL {
        let written = serde_json::to_string(&source).expect("writes");
        assert_eq!(written, format!("\"{}\"", source.key()));
        assert_eq!(MusicSource::from_key(source.key()), Some(source));
    }
    for cause in MusicProviderCause::ALL {
        let written = serde_json::to_string(&cause).expect("writes");
        assert_eq!(written, format!("\"{}\"", cause.key()));
        assert_eq!(MusicProviderCause::from_key(cause.key()), Some(cause));
    }
    let error = MusicError::Provider {
        cause: MusicProviderCause::Denied,
    };
    let written = serde_json::to_value(&error).expect("writes");
    assert_eq!(written["error"], error.key());
}
