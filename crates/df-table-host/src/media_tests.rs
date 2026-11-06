//! Pictures and sound: the library, the scene commands and what a page reports back.

use axum::http::StatusCode;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use df_playground::media::{AudioState, ClientReport};
use df_playground::scene::{ImageSlot, SceneChange, Target};
use serde_json::json;
use tempfile::TempDir;

use crate::client::{get, post_json};
use crate::command::{Command, CommandError, DEFAULT_VOLUME, parse, split_words, usage_text};
use crate::dev::{DevError, ReplyError, run_dev};
use crate::media_library::media_problem;
use crate::playground::PlaygroundChangeError;
use crate::test_support::{Running, STARTER_FILES, raw_exchange, start};
use crate::wire::{ClientView, ControlsReply};

const PNG: &[u8] = b"not really a picture";
const WAV: &[u8] = b"RIFFnot really a sound";

fn words(line: &str) -> Vec<String> {
    split_words(line).expect("the line splits")
}

fn scene(line: &str) -> Result<SceneChange, CommandError> {
    match parse(&words(line))? {
        Command::Scene(change) => Ok(change),
        other => panic!("expected a scene change, got {other:?}"),
    }
}

async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &words(line)).await
}

async fn add(running: &Running, name: &str, bytes: &[u8]) -> (StatusCode, String) {
    let body = json!({"name": name, "data_base64": STANDARD.encode(bytes)}).to_string();
    let response = post_json(running.port, "/dev/playground/media", &body)
        .await
        .expect("answers");
    (response.status, response.body)
}

async fn with_library(running: &Running) {
    add(running, "tavern.png", PNG).await;
    add(running, "bell.wav", WAV).await;
    running.write_asset(
        "fixtures/one.json",
        concat!(
            r#"{"name":"One","tv":{"place":"Here","status":{"kind":"ok"},"elements":[]},"#,
            r#""phone":{"hero":"Pip","status":{"kind":"ok"}}}"#
        ),
    );
    get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
}

async fn shown(running: &Running) -> ControlsReply {
    let response = get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    serde_json::from_str(&response.body).expect("controls")
}

// ---- grammar

#[test]
fn an_image_names_a_slot_a_file_and_an_optional_caption() {
    assert_eq!(
        scene(r#"scene image tv-backdrop tavern.png caption="The Flagon""#),
        Ok(SceneChange::Image {
            slot: ImageSlot::TvBackdrop,
            media: "tavern.png".into(),
            caption: Some("The Flagon".into()),
            rotation_degrees: 0,
            slide: None,
            layout: None,
        })
    );
    assert_eq!(
        scene("scene image phone map.png"),
        Ok(SceneChange::Image {
            slot: ImageSlot::PhoneImage,
            media: "map.png".into(),
            caption: None,
            rotation_degrees: 0,
            slide: None,
            layout: None,
        })
    );
    assert_eq!(
        scene("scene image-clear tv-card"),
        Ok(SceneChange::ClearImage {
            slot: ImageSlot::TvCard
        })
    );
    assert!(matches!(
        scene("scene image wall a.png"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        scene("scene image tv-card"),
        Err(CommandError::Usage(_))
    ));
    assert!(matches!(
        scene("scene image-clear"),
        Err(CommandError::Usage(_))
    ));
}

#[test]
fn music_and_sound_take_a_screen_a_volume_and_music_loops_unless_told_not_to() {
    assert_eq!(
        scene("scene music ambience.wav"),
        Ok(SceneChange::Music {
            media: "ambience.wav".into(),
            target: Target::Both,
            volume_percent: DEFAULT_VOLUME,
            looping: true
        })
    );
    assert_eq!(
        scene("scene music a.wav on=tv volume=30 loop=false"),
        Ok(SceneChange::Music {
            media: "a.wav".into(),
            target: Target::Tv,
            volume_percent: 30,
            looping: false
        })
    );
    assert_eq!(scene("scene music-stop"), Ok(SceneChange::StopMusic {}));
    assert_eq!(
        scene("scene sound bell.wav on=phone volume=100"),
        Ok(SceneChange::Sound {
            media: "bell.wav".into(),
            target: Target::Phone,
            volume_percent: 100
        })
    );
}

#[test]
fn a_screen_or_volume_that_is_not_allowed_is_refused_by_name() {
    for line in [
        "scene sound a.wav on=watch",
        "scene sound a.wav volume=101",
        "scene sound a.wav volume=-1",
        "scene sound a.wav volume=loud",
        "scene music a.wav loop=maybe",
    ] {
        assert!(
            matches!(scene(line), Err(CommandError::BadChoice { .. })),
            "{line}"
        );
    }
    assert!(matches!(scene("scene sound"), Err(CommandError::Usage(_))));
    assert_eq!(
        scene("scene sound a.wav volume=1 volume=2"),
        Err(CommandError::DuplicateKey("volume".into()))
    );
}

#[test]
fn media_and_clients_commands_parse_and_the_help_lists_every_new_verb() {
    assert_eq!(parse(&words("media")), Ok(Command::Media));
    assert_eq!(
        parse(&words("media add pics/tavern.png as=inn.png")),
        Ok(Command::MediaAdd {
            path: "pics/tavern.png".into(),
            name: Some("inn.png".into())
        })
    );
    assert_eq!(
        parse(&words("media add bell.wav")),
        Ok(Command::MediaAdd {
            path: "bell.wav".into(),
            name: None
        })
    );
    assert_eq!(
        parse(&words("media remove a.png")),
        Ok(Command::MediaRemove("a.png".into()))
    );
    assert_eq!(parse(&words("clients")), Ok(Command::Clients));
    assert!(matches!(
        parse(&words("media add")),
        Err(CommandError::Usage(_))
    ));
    assert_eq!(
        parse(&words("clients now")),
        Err(CommandError::Usage("clients"))
    );
    let help = usage_text();
    for text in [
        "scene image",
        "scene music",
        "scene sound",
        "media add",
        "clients",
    ] {
        assert!(help.contains(text), "{text}");
    }
}

// ---- the library

#[tokio::test]
async fn a_file_is_added_listed_served_with_its_type_and_removed() {
    let running = start(&STARTER_FILES).await;
    assert_eq!(add(&running, "tavern.png", PNG).await.0, StatusCode::OK);
    let listed = run(&running, "media").await.expect("lists");
    assert!(
        listed.contains("tavern.png") && listed.contains("image"),
        "{listed}"
    );

    let raw = raw_exchange(
        running.port,
        "GET /playground/media/tavern.png HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    )
    .await;
    let head = raw.to_ascii_lowercase();
    assert!(head.starts_with("http/1.1 200"), "{head}");
    assert!(
        head.contains("content-type: image/png")
            && head.contains("x-content-type-options: nosniff")
    );
    assert!(head.contains("cache-control: no-cache") && head.contains("etag:"));

    let removed = run(&running, "media remove tavern.png")
        .await
        .expect("removes");
    assert!(removed.starts_with("ok"));
    assert_eq!(
        run(&running, "media").await.expect("lists"),
        "no media (add one with: media add FILE)"
    );
}

#[tokio::test]
async fn a_file_that_has_not_changed_is_answered_with_a_short_not_modified() {
    let running = start(&STARTER_FILES).await;
    add(&running, "bell.wav", WAV).await;
    let first = raw_exchange(
        running.port,
        "GET /playground/media/bell.wav HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    )
    .await;
    let tag = first
        .lines()
        .find_map(|line| {
            line.strip_prefix("etag: ")
                .or_else(|| line.strip_prefix("ETag: "))
        })
        .expect("has an etag")
        .trim()
        .to_owned();
    let second = raw_exchange(
        running.port,
        &format!(
            "GET /playground/media/bell.wav HTTP/1.1\r\nHost: 127.0.0.1\r\n\
             If-None-Match: {tag}\r\nConnection: close\r\n\r\n"
        ),
    )
    .await;
    assert!(second.starts_with("HTTP/1.1 304"), "{second}");
    add(&running, "bell.wav", b"RIFFa different and longer sound").await;
    let third = raw_exchange(
        running.port,
        &format!(
            "GET /playground/media/bell.wav HTTP/1.1\r\nHost: 127.0.0.1\r\n\
             If-None-Match: {tag}\r\nConnection: close\r\n\r\n"
        ),
    )
    .await;
    assert!(
        third.starts_with("HTTP/1.1 200"),
        "a replaced file is sent again"
    );
}

#[tokio::test]
async fn a_name_that_is_not_a_picture_or_sound_is_refused_and_nothing_is_written() {
    let running = start(&STARTER_FILES).await;
    for name in [
        "logo.svg",
        "run.exe",
        "page.html",
        "../escape.png",
        "a/b.png",
        "noextension",
        "",
    ] {
        let (status, _) = add(&running, name, PNG).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name:?}");
    }
    assert_eq!(
        run(&running, "media").await.expect("lists"),
        "no media (add one with: media add FILE)"
    );
}

#[tokio::test]
async fn an_empty_file_and_data_that_is_not_base64_are_refused() {
    let running = start(&STARTER_FILES).await;
    assert_eq!(add(&running, "a.png", b"").await.0, StatusCode::BAD_REQUEST);
    let body = json!({"name": "a.png", "data_base64": "not base64 !!"}).to_string();
    let response = post_json(running.port, "/dev/playground/media", &body)
        .await
        .expect("answers");
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert!(response.body.contains("base64"));
}

#[tokio::test]
async fn removing_a_file_that_is_not_there_says_so() {
    let running = start(&STARTER_FILES).await;
    let error = run(&running, "media remove ghost.png")
        .await
        .expect_err("refused");
    // The library's refusals carry no cause yet; the sentence names the file.
    assert!(
        matches!(
            &error,
            DevError::Rejected {
                error: ReplyError::Canvas { cause: None, .. },
                message,
            } if message.contains("ghost.png")
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_file_is_added_from_the_command_line_under_its_own_name_or_another() {
    let running = start(&STARTER_FILES).await;
    let directory = TempDir::new().expect("makes a directory");
    let path = directory.path().join("tavern.png");
    std::fs::write(&path, PNG).expect("writes");
    let words = vec![
        "media".to_owned(),
        "add".to_owned(),
        path.to_string_lossy().into_owned(),
    ];
    run_dev(running.port, &words).await.expect("adds");
    let words = vec![
        "media".to_owned(),
        "add".to_owned(),
        path.to_string_lossy().into_owned(),
        "as=inn.png".to_owned(),
    ];
    run_dev(running.port, &words).await.expect("adds");
    let listed = run(&running, "media").await.expect("lists");
    assert!(
        listed.contains("tavern.png") && listed.contains("inn.png"),
        "{listed}"
    );
    let missing = vec![
        "media".to_owned(),
        "add".to_owned(),
        "no/such.png".to_owned(),
    ];
    assert!(matches!(
        run_dev(running.port, &missing).await,
        Err(DevError::UnreadableFile { .. })
    ));
}

// ---- the scene refers to the library

#[tokio::test]
async fn a_picture_and_a_sound_are_shown_and_cued_from_the_library() {
    let running = start(&STARTER_FILES).await;
    with_library(&running).await;
    run(&running, "scene image tv-backdrop tavern.png caption=Inn")
        .await
        .expect("shows");
    run(&running, "scene music bell.wav volume=20")
        .await
        .expect("plays");
    run(&running, "scene sound bell.wav on=tv")
        .await
        .expect("cues");
    let reply = shown(&running).await;
    assert!(!reply.scene.backdrop.is_keep() && !reply.scene.music.is_keep());
    assert_eq!(reply.scene.cue_count, 1);
    assert_eq!(reply.scene.cues[0].media, "bell.wav");
    assert!(
        run(&running, "playground")
            .await
            .expect("shows")
            .contains("scene: changed:")
    );
}

#[tokio::test]
async fn a_media_name_that_is_missing_or_the_wrong_kind_is_refused_and_says_what_exists() {
    let running = start(&STARTER_FILES).await;
    with_library(&running).await;
    let missing = run(&running, "scene image tv-card ghost.png")
        .await
        .expect_err("refused");
    let text = refused_change(&missing);
    assert!(
        text.contains("there is no picture 'ghost.png'") && text.contains("tavern.png"),
        "{missing}"
    );
    let wrong = run(&running, "scene image tv-card bell.wav")
        .await
        .expect_err("refused");
    assert!(
        refused_change(&wrong).contains("is a sound, not a picture"),
        "{wrong}"
    );
    let wrong = run(&running, "scene sound tavern.png")
        .await
        .expect_err("refused");
    assert!(
        refused_change(&wrong).contains("is a picture, not a sound"),
        "{wrong}"
    );
    assert!(shown(&running).await.scene.is_empty());
}

/// The sentence of a scene change the playground's checks refused; the sentence is what these
/// tests are about (it names what exists).
fn refused_change(error: &DevError) -> &str {
    match error {
        DevError::Rejected {
            error: ReplyError::Playground(PlaygroundChangeError::InvalidChange { detail }),
            ..
        } => detail,
        other => panic!("not a refused change: {other:?}"),
    }
}

#[tokio::test]
async fn with_an_empty_library_the_refusal_says_how_to_add_one() {
    let running = start(&STARTER_FILES).await;
    running.write_asset(
        "fixtures/one.json",
        concat!(
            r#"{"name":"One","tv":{"place":"Here","status":{"kind":"ok"},"elements":[]},"#,
            r#""phone":{"hero":"Pip","status":{"kind":"ok"}}}"#
        ),
    );
    get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    let error = run(&running, "scene sound bell.wav")
        .await
        .expect_err("refused");
    assert!(refused_change(&error).contains("media add FILE"), "{error}");
}

#[test]
fn the_library_check_is_only_for_changes_that_name_media() {
    assert_eq!(media_problem(&SceneChange::StopMusic {}, &[]), None);
    assert_eq!(media_problem(&SceneChange::Reset {}, &[]), None);
    assert_eq!(
        media_problem(
            &SceneChange::ClearImage {
                slot: ImageSlot::TvCard
            },
            &[]
        ),
        None
    );
}

#[tokio::test]
async fn choosing_another_fixture_drops_pictures_but_cues_keep_counting() {
    let running = start(&STARTER_FILES).await;
    with_library(&running).await;
    running.write_asset(
        "fixtures/two.json",
        concat!(
            r#"{"name":"Two","tv":{"place":"There","status":{"kind":"ok"},"elements":[]},"#,
            r#""phone":{"hero":"Pip","status":{"kind":"ok"}}}"#
        ),
    );
    run(&running, "scene image tv-card tavern.png")
        .await
        .expect("shows");
    run(&running, "scene sound bell.wav").await.expect("cues");
    run(&running, "playground set fixture=two.json")
        .await
        .expect("switches");
    let reply = shown(&running).await;
    assert!(reply.scene.card.is_keep());
    assert_eq!(
        reply.scene.cue_count, 1,
        "a page that played cue 1 must not replay it"
    );
    run(&running, "scene sound bell.wav").await.expect("cues");
    assert_eq!(shown(&running).await.scene.cues[0].id, 2);
}

// ---- what a page reports

fn report(audio: AudioState, last_cue: u64) -> ClientReport {
    ClientReport {
        audio,
        last_cue,
        music: Some("bell.wav".into()),
        images_shown: vec!["tavern.png".into()],
        errors: vec!["could not load ghost.png".into()],
    }
}

async fn send_report(running: &Running, client: &str, report: &ClientReport) -> StatusCode {
    let body = json!({"client": client, "report": report}).to_string();
    post_json(running.port, "/dev/playground/report", &body)
        .await
        .expect("answers")
        .status
}

#[tokio::test]
async fn a_pages_report_is_kept_and_shown_by_clients_and_playground() {
    let running = start(&STARTER_FILES).await;
    assert!(
        run(&running, "clients")
            .await
            .expect("reads")
            .starts_with("clients: none")
    );
    assert_eq!(
        send_report(&running, "page-1", &report(AudioState::Locked, 2)).await,
        StatusCode::OK
    );
    let text = run(&running, "clients").await.expect("reads");
    assert!(
        text.contains("client page-1") && text.contains("audio LOCKED"),
        "{text}"
    );
    assert!(text.contains("last sound cue 2") && text.contains("music bell.wav"));
    assert!(
        text.contains("pictures shown tavern.png")
            && text.contains("error: could not load ghost.png")
    );
    assert_eq!(
        send_report(&running, "page-1", &report(AudioState::Unlocked, 3)).await,
        StatusCode::OK
    );
    let text = run(&running, "clients").await.expect("reads");
    assert!(text.contains("audio unlocked") && text.contains("last sound cue 3"));
    assert_eq!(
        text.matches("client page-1").count(),
        1,
        "the same page is one entry"
    );
    assert!(
        run(&running, "playground")
            .await
            .expect("shows")
            .contains("client page-1")
    );
}

#[tokio::test]
async fn a_report_does_not_wake_pages_and_is_not_an_audit_line() {
    let running = start(&STARTER_FILES).await;
    let before = running.state.change_count();
    send_report(&running, "page-1", &report(AudioState::Locked, 0)).await;
    assert_eq!(running.state.change_count(), before);
    let events = get(running.port, "/events.json")
        .await
        .expect("answers")
        .body;
    assert!(!events.contains("page-1"));
}

#[tokio::test]
async fn a_report_with_a_bad_client_name_or_body_is_refused() {
    let running = start(&STARTER_FILES).await;
    for name in [
        "",
        "has space",
        "a/b",
        &"x".repeat(crate::limits::MAXIMUM_CLIENT_NAME + 1),
    ] {
        assert_eq!(
            send_report(&running, name, &report(AudioState::Locked, 0)).await,
            StatusCode::BAD_REQUEST,
            "{name:?}"
        );
    }
    let response = post_json(running.port, "/dev/playground/report", "nope")
        .await
        .expect("answers");
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn only_the_newest_pages_are_remembered_past_the_maximum() {
    let running = start(&STARTER_FILES).await;
    for index in 0..crate::limits::MAXIMUM_CLIENTS + 2 {
        send_report(
            &running,
            &format!("page-{index}"),
            &report(AudioState::Locked, 0),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    let body = get(running.port, "/playground/clients.json")
        .await
        .expect("answers")
        .body;
    let clients: Vec<ClientView> = serde_json::from_str(&body).expect("clients");
    assert_eq!(clients.len(), crate::limits::MAXIMUM_CLIENTS);
    assert!(
        clients
            .iter()
            .all(|client| client.client != "page-0" && client.client != "page-1")
    );
}

#[tokio::test]
async fn a_report_keeps_only_its_newest_errors() {
    let running = start(&STARTER_FILES).await;
    let mut noisy = report(AudioState::Locked, 0);
    let kept = df_playground::limits::MAXIMUM_REPORTED_ERRORS;
    noisy.errors = (0..kept + 12)
        .map(|index| format!("error {index}"))
        .collect();
    send_report(&running, "page-1", &noisy).await;
    let body = get(running.port, "/playground/clients.json")
        .await
        .expect("answers")
        .body;
    let clients: Vec<ClientView> = serde_json::from_str(&body).expect("clients");
    assert_eq!(clients[0].report.errors.len(), kept);
    assert_eq!(
        clients[0].report.errors[kept - 1],
        format!("error {}", kept + 11)
    );
}

#[tokio::test]
async fn a_page_that_has_gone_quiet_is_forgotten() {
    let running = start(&STARTER_FILES).await;
    send_report(&running, "page-1", &report(AudioState::Locked, 0)).await;
    let playground = running.state.playground();
    let soon_ms = playground.clock_ms() + 10_000;
    assert_eq!(playground.clients_at(soon_ms).len(), 1);
    let forget_after_ms = u64::try_from(crate::limits::FORGET_CLIENT_AFTER.as_millis()).unwrap();
    let later_ms = playground.clock_ms() + forget_after_ms;
    assert!(playground.clients_at(later_ms).is_empty());
    assert!(
        running.state.playground().clients().is_empty(),
        "and it is gone from memory"
    );
}

#[test]
fn an_image_can_be_turned_with_rotate_and_a_bad_turn_is_refused() {
    assert_eq!(
        scene("scene image tv-card finn-alder.webp rotate=180"),
        Ok(SceneChange::Image {
            slot: ImageSlot::TvCard,
            media: "finn-alder.webp".into(),
            caption: None,
            rotation_degrees: 180,
            slide: None,
            layout: None,
        })
    );
    for bad in ["rotate=360", "rotate=-90", "rotate=upside", "rotate=1.5"] {
        assert!(
            matches!(
                scene(&format!("scene image tv-card a.png {bad}")),
                Err(CommandError::BadChoice { .. })
            ),
            "{bad}"
        );
    }
    assert!(usage_text().contains("rotate=0-359"));
}

#[test]
fn slide_sends_the_card_across_the_screen_with_defaults_from_left_to_right_over_five_seconds() {
    use crate::command::DEFAULT_SLIDE;
    assert_eq!(DEFAULT_SLIDE.from_percent, 15);
    assert_eq!(DEFAULT_SLIDE.to_percent, 85);
    assert_eq!(DEFAULT_SLIDE.duration_ms, 5_000);
    assert_eq!(
        scene("scene slide tv-card finn-alder.webp rotate=180"),
        Ok(SceneChange::Image {
            slot: ImageSlot::TvCard,
            media: "finn-alder.webp".into(),
            caption: None,
            rotation_degrees: 180,
            slide: Some(DEFAULT_SLIDE),
            layout: None,
        })
    );
}

#[test]
fn slide_reads_where_it_starts_ends_and_how_long_it_takes() {
    let Ok(SceneChange::Image {
        slide: Some(moving),
        caption,
        ..
    }) = scene(r#"scene slide tv-card a.png from=5 to=95 over=2500 caption="Finn""#)
    else {
        panic!("expected a slide");
    };
    assert_eq!(
        (moving.from_percent, moving.to_percent, moving.duration_ms),
        (5, 95, 2_500)
    );
    assert_eq!(caption.as_deref(), Some("Finn"));
}

#[test]
fn slide_refuses_a_bad_share_a_bad_length_an_unknown_key_and_a_missing_name() {
    for line in [
        "scene slide tv-card a.png from=101",
        "scene slide tv-card a.png to=-1",
        "scene slide tv-card a.png over=0",
        &format!(
            "scene slide tv-card a.png over={}",
            df_playground::limits::MAXIMUM_SLIDE_MS + 1
        ),
        "scene slide tv-card a.png over=slowly",
        "scene slide tv-card a.png rotate=360",
        "scene slide tv-card a.png speed=3",
        "scene slide wall a.png",
    ] {
        assert!(
            matches!(scene(line), Err(CommandError::BadChoice { .. })),
            "{line}"
        );
    }
    assert!(matches!(
        scene("scene slide tv-card"),
        Err(CommandError::Usage(_))
    ));
    assert!(usage_text().contains("scene slide tv-card"));
}
